//! WASAPI Loopback 采集：从系统输出设备抓取游戏声音。
//!
//! 合规红线：仅使用操作系统公开音频 API，不读取、不注入、不修改任何游戏进程。

use crate::detect::Detector;
use crate::state::{SharedState, SoundEvent};
use anyhow::{bail, Context, Result};
use std::ptr;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows::core::GUID;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    DEVICE_STATE_ACTIVE, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};

const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00AA00389B71);
const CLSID_MMDEVICE_ENUMERATOR: GUID = GUID::from_u128(0xBCDE0395_E52F_467C_8E3D_C4579291692E);
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xA45C254E_DF1C_4EFD_8020_67D146A850E0),
    pid: 14,
};

// ---- 扬声器通道掩码位（KSAUDIO_SPEAKER_*，值与 Windows SDK 一致） ----
const SPK_FRONT_LEFT: u32 = 0x1;
const SPK_FRONT_RIGHT: u32 = 0x2;
const SPK_FRONT_CENTER: u32 = 0x4;
const SPK_LOW_FREQ: u32 = 0x8;
const SPK_BACK_LEFT: u32 = 0x10;
const SPK_BACK_RIGHT: u32 = 0x20;
const SPK_FRONT_LEFT_OF_CENTER: u32 = 0x40;
const SPK_FRONT_RIGHT_OF_CENTER: u32 = 0x80;
const SPK_BACK_CENTER: u32 = 0x100;
const SPK_SIDE_LEFT: u32 = 0x200;
const SPK_SIDE_RIGHT: u32 = 0x400;
const SPK_TOP_BACK_LEFT: u32 = 0x8000;
const SPK_TOP_BACK_RIGHT: u32 = 0x10000;

/// 与 WAVEFORMATEXTENSIBLE 内存布局一致（仅用我们需要的字段）
#[repr(C)]
struct WaveFormatExtensible {
    format: WAVEFORMATEX,
    samples: u16,      // wValidBitsPerSample
    channel_mask: u32, // dwChannelMask
    sub_format: GUID,
}

impl WaveFormatExtensible {
    fn f32(rate: u32, channels: usize, mask: u32) -> Self {
        Self {
            format: WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_EXTENSIBLE,
                nChannels: channels as u16,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: rate * channels as u32 * 4,
                nBlockAlign: (channels * 4) as u16,
                wBitsPerSample: 32,
                cbSize: 22,
            },
            samples: 32,
            channel_mask: mask,
            sub_format: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
        }
    }
}

#[derive(Clone)]
pub struct CaptureFormat {
    pub sample_rate: u32,
    pub channels: usize,
    /// 每通道的方位角（度）：0=正前，+90=右，顺时针
    pub channel_angles: Vec<f32>,
    pub is_lfe: Vec<bool>,
    pub bits_per_sample: u16,
    pub is_float: bool,
}

impl CaptureFormat {
    fn build(rate: u32, channels: usize, mask: u32, bits: u16, is_float: bool) -> Self {
        let (channel_angles, is_lfe) = Self::layout(channels, mask);
        Self {
            sample_rate: rate,
            channels,
            channel_angles,
            is_lfe,
            bits_per_sample: bits,
            is_float,
        }
    }

    fn layout(channels: usize, mask: u32) -> (Vec<f32>, Vec<bool>) {
        let mut angles = vec![0f32; channels];
        let mut lfe = vec![false; channels];
        if mask != 0 {
            for (i, a) in angles.iter_mut().enumerate() {
                if let Some(bit) = nth_set_bit(mask, i as u32) {
                    if bit == SPK_LOW_FREQ {
                        lfe[i] = true;
                        *a = 0.0;
                    } else {
                        *a = angle_for_bit(bit).unwrap_or(0.0);
                    }
                }
            }
        } else {
            // 无通道掩码时按标准顺序回退
            match channels {
                1 => angles[0] = 0.0,
                2 => {
                    // 立体声：只能分辨左右（±90°）
                    angles[0] = -90.0;
                    angles[1] = 90.0;
                }
                4 => {
                    angles[0] = -30.0;
                    angles[1] = 30.0;
                    angles[2] = -110.0;
                    angles[3] = 110.0;
                }
                6 => {
                    angles[0] = -30.0;
                    angles[1] = 30.0;
                    angles[2] = 0.0;
                    lfe[3] = true;
                    angles[4] = -110.0;
                    angles[5] = 110.0;
                }
                8 => {
                    angles[0] = -30.0;
                    angles[1] = 30.0;
                    angles[2] = 0.0;
                    lfe[3] = true;
                    angles[4] = -110.0;
                    angles[5] = 110.0;
                    angles[6] = -90.0;
                    angles[7] = 90.0;
                }
                _ => {}
            }
        }
        (angles, lfe)
    }

    pub fn describe(&self) -> String {
        let dir = if self.channels >= 4 {
            "环绕方位可用".to_string()
        } else {
            "仅左右（建议 7.1 虚拟声卡）".to_string()
        };
        format!(
            "{} Hz · {} 声道 · {}bit{} · {}",
            self.sample_rate,
            self.channels,
            self.bits_per_sample,
            if self.is_float { " float" } else { "" },
            dir
        )
    }
}

fn nth_set_bit(mask: u32, n: u32) -> Option<u32> {
    let mut count = 0u32;
    for i in 0..32 {
        if mask & (1 << i) != 0 {
            if count == n {
                return Some(1 << i);
            }
            count += 1;
        }
    }
    None
}

fn angle_for_bit(bit: u32) -> Option<f32> {
    Some(match bit {
        SPK_FRONT_LEFT => -30.0,
        SPK_FRONT_RIGHT => 30.0,
        SPK_FRONT_CENTER => 0.0,
        SPK_FRONT_LEFT_OF_CENTER => -45.0,
        SPK_FRONT_RIGHT_OF_CENTER => 45.0,
        SPK_SIDE_LEFT => -90.0,
        SPK_SIDE_RIGHT => 90.0,
        SPK_BACK_LEFT | SPK_TOP_BACK_LEFT => -135.0,
        SPK_BACK_RIGHT | SPK_TOP_BACK_RIGHT => 135.0,
        SPK_BACK_CENTER => 180.0,
        _ => return None,
    })
}

/// 主线程 STA 初始化（供设置面板枚举设备）
pub fn init_com_sta() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// 枚举当前活动的输出（渲染）设备
pub fn enumerate_devices() -> Result<Vec<String>> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .context("创建 MMDeviceEnumerator 失败")?;
        let coll = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .context("枚举输出设备失败")?;
        let n = coll.GetCount()?;
        let mut names = Vec::with_capacity(n as usize);
        for i in 0..n {
            let dev = coll.Item(i)?;
            names.push(device_friendly_name(&dev).unwrap_or_else(|_| format!("输出设备 {i}")));
        }
        Ok(names)
    }
}

/// 测试回放模式在设备列表中的名称
pub const TEST_DEVICE_LABEL: &str = "（测试）WAV 回放";

/// 音频采集线程：自动重试、设备切换时自动重启
pub fn run_audio_thread(state: Arc<SharedState>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    loop {
        let want = state.settings.read().device_name.clone();
        let result = if want == TEST_DEVICE_LABEL {
            run_wav_replay(&state, &want)
        } else {
            run_capture(&state, &want)
        };
        {
            let mut st = state.status.lock();
            st.running = false;
            match result {
                Ok(reason) => {
                    st.error = None;
                    log::info!("采集结束（{reason}）");
                }
                Err(e) => {
                    st.error = Some(format!("{e:#}"));
                    log::warn!("采集出错: {e:#}");
                }
            }
        }
        thread::sleep(Duration::from_millis(1500));
    }
}

/// 测试回放：直接读取 WAV 文件喂给检测器（绕过声卡，用于开发调试）
fn run_wav_replay(state: &Arc<SharedState>, want: &str) -> Result<String> {
    let samples = load_wav("test_sound.wav")?;
    let (channels, rate) = (2u32, 48000u32);
    {
        let mut st = state.status.lock();
        st.devices = vec![TEST_DEVICE_LABEL.to_string()];
        st.active_device = TEST_DEVICE_LABEL.to_string();
        st.format = format!("{} Hz · {} 声道 · 16bit · 测试回放", rate, channels);
        st.running = true;
        st.error = None;
    }
    log::info!("测试回放模式：{} 帧", samples.len() / channels as usize);
    let fmt = CaptureFormat::build(rate, channels as usize, 0, 16, false);
    let mut detector = Detector::new(rate);
    let block_frames = 480usize;
    let block_samples = block_frames * channels as usize;
    let mut pos = 0usize;
    let mut blocks: u64 = 0;
    loop {
        if state.settings.read().device_name != want {
            return Ok("设备选择已更改，重启采集".into());
        }
        drain_test_requests(state);
        if pos + block_samples > samples.len() {
            pos = 0; // 循环回放
            continue;
        }
        let slice = &samples[pos..pos + block_samples];
        let settings = state.settings.read().clone();
        detector.process_block(slice, &fmt, &settings, state);
        blocks += 1;
        if blocks % 100 == 0 {
            log::debug!("回放块 {blocks}，当前电平 {:.1} dB", *state.level_db.lock());
        }
        pos += block_samples;
        thread::sleep(Duration::from_millis(10));
    }
}

/// 声音位置测试：设置面板提交请求（任意角度 360°、任意距离）
pub fn push_test_sound(
    state: &SharedState,
    kind: crate::state::EventKind,
    angle_deg: f32,
    distance: f32,
) {
    state.test_queue.lock().push_back(crate::state::TestReq {
        kind,
        angle_deg,
        distance,
    });
}

/// 声音位置测试：设置面板提交请求 → 直接向覆盖层注入指定方位/距离的事件。
/// 任意角度（360°）与近/中/远精确显示，不再受立体声"只能左右"限制；
/// 检测链路的正确性由「（测试）WAV 回放」验证。
fn drain_test_requests(state: &Arc<SharedState>) {
    let mut reqs = Vec::new();
    {
        let mut q = state.test_queue.lock();
        while let Some(r) = q.pop_front() {
            reqs.push(r);
        }
    }
    for r in reqs {
        log::info!(
            "声音测试注入：{:?} @ {:.0}° 距离 {:.2}",
            r.kind,
            r.angle_deg,
            r.distance
        );
        let max_same = {
            let st = state.settings.read();
            match r.kind {
                crate::state::EventKind::Footstep => st.max_footsteps.max(1),
                crate::state::EventKind::Gunshot => st.max_gunshots.max(1),
            }
        };
        let mut evs = state.events.lock();
        evs.push_front(SoundEvent {
            kind: r.kind,
            angle_deg: r.angle_deg,
            distance: r.distance,
            strength: 0.9,
            born: Instant::now(),
        });
        // 按类型上限裁剪（超出则移除最旧的同类）
        let mut count = 0usize;
        let mut remove_idx = None;
        for (i, e) in evs.iter().enumerate() {
            if e.kind == r.kind {
                count += 1;
                if count > max_same {
                    remove_idx = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = remove_idx {
            evs.remove(i);
        }
        drop(evs);
        *state.event_counter.lock() += 1;
        state.wake_overlay();
    }
}

/// 极简 WAV 解析（PCM16 / F32）
fn load_wav(path: &str) -> Result<Vec<f32>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).with_context(|| format!("找不到测试音频 {path}"))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    if buf.len() < 44 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
        bail!("无效的 WAV 文件（需要 PCM16 或 F32）");
    }
    let mut pos = 12usize;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data: &[u8] = &[];
    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let size = u32::from_le_bytes(buf[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let start = pos + 8;
        match id {
            b"fmt " => {
                channels = u16::from_le_bytes(buf[start..start + 2].try_into().unwrap());
                bits = u16::from_le_bytes(buf[start + 14..start + 16].try_into().unwrap());
            }
            b"data" => {
                data = &buf[start..start + size.min(buf.len() - start)];
                break;
            }
            _ => {}
        }
        pos = start + size + (size & 1);
    }
    if data.is_empty() || channels == 0 {
        bail!("WAV 缺少数据块");
    }
    let mut out = Vec::with_capacity(data.len() / (bits as usize / 8));
    match bits {
        16 => {
            for i in (0..data.len()).step_by(2) {
                if i + 1 >= data.len() {
                    break;
                }
                let v = i16::from_le_bytes([data[i], data[i + 1]]);
                out.push(v as f32 / 32768.0);
            }
        }
        32 => {
            for i in (0..data.len()).step_by(4) {
                if i + 3 >= data.len() {
                    break;
                }
                let v = f32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
                out.push(v);
            }
        }
        _ => bail!("不支持的位深 {bits}（需要 16 或 32）"),
    }
    Ok(out)
}

fn run_capture(state: &Arc<SharedState>, want: &str) -> Result<String> {
    let devices = enumerate_devices()?;
    {
        let mut st = state.status.lock();
        st.devices = devices.clone();
        st.error = None;
        st.running = false;
    }
    let device = pick_device(&devices, want)?;
    let active_name = device_friendly_name(&device)?;
    let (capture, client, fmt) = open_capture(&device)?;
    {
        let mut st = state.status.lock();
        st.active_device = active_name.clone();
        st.format = fmt.describe();
        st.running = true;
        st.error = None;
    }
    log::info!("开始采集: {} ({})", active_name, fmt.describe());

    let mut detector = Detector::new(fmt.sample_rate);
    let mut buf: Vec<f32> = Vec::new();
    let mut blocks: u64 = 0;

    unsafe {
        client.Start()?;
        loop {
            // 用户在设置面板切换了设备 → 重启采集
            if state.settings.read().device_name != want {
                let _ = client.Stop();
                return Ok("设备选择已更改，重启采集".into());
            }
            // 声音位置测试请求（设置面板发起）
            drain_test_requests(state);
            // 每轮循环都重新查询待取包数（0 表示暂无数据）
            let mut packet = capture.GetNextPacketSize()?;
            while packet > 0 {
                let mut data: *mut u8 = ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                if frames > 0 && !data.is_null() {
                    let n = frames as usize * fmt.channels;
                    if buf.len() < n {
                        buf.resize(n, 0.0);
                    }
                    let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                    if silent {
                        for x in buf[..n].iter_mut() {
                            *x = 0.0;
                        }
                    } else {
                        convert_to_f32(data, frames as usize, &fmt, &mut buf[..n]);
                    }
                    let settings = state.settings.read().clone();
                    detector.process_block(&buf[..n], &fmt, &settings, state);
                    blocks += 1;
                    if blocks % 100 == 0 {
                        log::debug!(
                            "采集块数 {blocks}，当前电平 {:.1} dB",
                            *state.level_db.lock()
                        );
                    }
                }
                capture.ReleaseBuffer(frames)?;
                packet = capture.GetNextPacketSize()?;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn pick_device(names: &[String], want: &str) -> Result<IMMDevice> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .context("创建 MMDeviceEnumerator 失败")?;
        if want.is_empty() {
            return Ok(enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .context("获取默认输出设备失败")?);
        }
        let coll = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .context("枚举输出设备失败")?;
        let n = coll.GetCount()?;
        for i in 0..n {
            let dev = coll.Item(i)?;
            if device_friendly_name(&dev)
                .map(|s| s == want)
                .unwrap_or(false)
            {
                return Ok(dev);
            }
        }
        bail!(
            "找不到输出设备「{want}」，可能已断开。可用设备：{}",
            names.join("、")
        );
    }
}

fn device_friendly_name(dev: &IMMDevice) -> Result<String> {
    unsafe {
        let store: IPropertyStore = dev
            .OpenPropertyStore(STGM_READ)
            .context("打开设备属性存储失败")?;
        let mut pv = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME)?;
        let out = PropVariantToStringAlloc(&pv)?;
        let s = if out.0.is_null() {
            String::new()
        } else {
            out.to_string().unwrap_or_default()
        };
        if !out.0.is_null() {
            CoTaskMemFree(Some(out.0 as *const _));
        }
        let _ = PropVariantClear(&mut pv);
        Ok(s)
    }
}

fn open_capture(device: &IMMDevice) -> Result<(IAudioCaptureClient, IAudioClient, CaptureFormat)> {
    unsafe {
        let client: IAudioClient = device
            .Activate::<IAudioClient>(CLSCTX_ALL, None)
            .context("激活 IAudioClient 失败")?;

        let mix_ptr = client.GetMixFormat()?;
        if mix_ptr.is_null() {
            bail!("GetMixFormat 返回空指针");
        }
        let mix = &*mix_ptr;
        let rate = mix.nSamplesPerSec;
        let channels = mix.nChannels as usize;
        if channels == 0 || rate == 0 {
            CoTaskMemFree(Some(mix_ptr as *const _));
            bail!("设备返回无效的混音格式");
        }
        let mut mask = 0u32;
        let mix_is_float = mix.wFormatTag == WAVE_FORMAT_IEEE_FLOAT;
        if mix.wFormatTag == WAVE_FORMAT_EXTENSIBLE && mix.cbSize >= 22 {
            let ext = &*(mix_ptr as *const WaveFormatExtensible);
            mask = ext.channel_mask;
        }

        // 优先请求 float32（与混音格式同采样率/声道）；
        // 统一使用 IAudioClient::Initialize（IAudioClient3 的 loopback 在部分设备上
        // 会返回 AUDCLNT_E_BUFFER_SIZE_ERROR）
        let want_f32 = WaveFormatExtensible::f32(rate, channels, mask);
        let f32_ptr = &want_f32 as *const WaveFormatExtensible as *const WAVEFORMATEX;
        let f32_supported = client
            .IsFormatSupported(AUDCLNT_SHAREMODE_SHARED, f32_ptr, None)
            .is_ok();

        let mut use_f32 = false;
        if f32_supported {
            if client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    0,
                    0,
                    f32_ptr,
                    None,
                )
                .is_ok()
            {
                use_f32 = true;
            }
        }
        if !use_f32 {
            // 回退：直接使用设备混音格式（采集后在软件中转换）
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                0,
                0,
                mix_ptr,
                None,
            )?;
        }

        let capture: IAudioCaptureClient = client
            .GetService::<IAudioCaptureClient>()
            .context("获取 IAudioCaptureClient 失败")?;
        CoTaskMemFree(Some(mix_ptr as *const _));

        let fmt = if use_f32 {
            CaptureFormat::build(rate, channels, mask, 32, true)
        } else {
            CaptureFormat::build(rate, channels, mask, mix.wBitsPerSample, mix_is_float)
        };
        Ok((capture, client, fmt))
    }
}

/// 将采集缓冲转换为 f32 交错数据
unsafe fn convert_to_f32(data: *const u8, frames: usize, fmt: &CaptureFormat, out: &mut [f32]) {
    let ch = fmt.channels;
    let n = frames * ch;
    match (fmt.is_float, fmt.bits_per_sample) {
        (true, 32) => {
            ptr::copy_nonoverlapping(data as *const f32, out.as_mut_ptr(), n);
        }
        (false, 16) => {
            let p = data as *const i16;
            for i in 0..n {
                *out.get_unchecked_mut(i) = (*p.add(i) as f32) / 32768.0;
            }
        }
        (false, 24) => {
            for i in 0..n {
                let b = data.add(i * 3);
                let v = (*b as i32) | ((*b.add(1) as i32) << 8) | ((*b.add(2) as i32) << 16);
                let v = if v & 0x800000 != 0 { v | !0xFF_FFFF } else { v };
                *out.get_unchecked_mut(i) = v as f32 / 8388608.0;
            }
        }
        (false, 32) => {
            let p = data as *const i32;
            for i in 0..n {
                *out.get_unchecked_mut(i) = (*p.add(i) as f32) / 2147483648.0;
            }
        }
        (false, 8) => {
            for i in 0..n {
                *out.get_unchecked_mut(i) = ((*data.add(i) as i16) - 128) as f32 / 128.0;
            }
        }
        _ => {
            for x in out.iter_mut() {
                *x = 0.0;
            }
        }
    }
}
