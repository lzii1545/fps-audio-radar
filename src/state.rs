//! 线程间共享状态：设置、事件队列、电平表、采集状态

use crate::settings::Settings;
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_USER};

/// 自定义唤醒消息
pub const WM_WAKE: u32 = WM_USER + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Footstep,
    Gunshot,
}

/// 声音位置测试请求（设置面板发起，音频线程合成并送入检测器）
#[derive(Clone, Copy, Debug)]
pub struct TestReq {
    pub kind: EventKind,
    pub angle_deg: f32,
    pub distance: f32,
}

#[derive(Clone, Debug)]
pub struct SoundEvent {
    pub kind: EventKind,
    /// 0° = 正前方（屏幕上方），顺时针为正
    pub angle_deg: f32,
    /// 0 = 近，1 = 远（按相对响度估计，仅近/中/远三档精度）
    pub distance: f32,
    /// 0..1 方向置信度
    pub strength: f32,
    pub born: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct CaptureStatus {
    pub devices: Vec<String>,
    pub active_device: String,
    pub format: String,
    pub running: bool,
    pub error: Option<String>,
}

pub struct SharedState {
    pub settings: RwLock<Settings>,
    pub events: Mutex<VecDeque<SoundEvent>>,
    /// 当前帧电平（dBFS）
    pub level_db: Mutex<f32>,
    /// 峰值保持（dBFS）
    pub peak_db: Mutex<f32>,
    pub status: Mutex<CaptureStatus>,
    /// 累计事件数
    pub event_counter: Mutex<u64>,
    /// 位置编辑模式（覆盖层拖拽定位）
    pub edit_mode: AtomicBool,
    /// 覆盖层窗口句柄（PostMessage 唤醒重绘，由覆盖层线程注册；存 isize 以满足 Send）
    pub wake: Mutex<Option<isize>>,
    /// 声音位置测试请求队列
    pub test_queue: Mutex<VecDeque<TestReq>>,
}

impl SharedState {
    pub fn new(settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            settings: RwLock::new(settings),
            events: Mutex::new(VecDeque::new()),
            level_db: Mutex::new(-90.0),
            peak_db: Mutex::new(-90.0),
            status: Mutex::new(CaptureStatus::default()),
            event_counter: Mutex::new(0),
            edit_mode: AtomicBool::new(false),
            wake: Mutex::new(None),
            test_queue: Mutex::new(VecDeque::new()),
        })
    }

    /// 通知覆盖层立即重绘（事件到达 / 设置变化时调用）
    pub fn wake_overlay(&self) {
        if let Some(hwnd_raw) = *self.wake.lock() {
            unsafe {
                let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);
                let _ = PostMessageW(hwnd, WM_WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// 当前有效事件（附带渐隐系数 1→0），自动清理过期事件；
    /// 渐隐时长按类型区分（脚步/枪声各自可调）
    pub fn active_events(&self) -> Vec<(SoundEvent, f32)> {
        let mut evs = self.events.lock();
        let st = self.settings.read();
        evs.retain(|e| {
            let decay = match e.kind {
                EventKind::Footstep => st.footstep_decay.max(0.2),
                EventKind::Gunshot => st.gunshot_decay.max(0.2),
            };
            e.born.elapsed().as_secs_f32() < decay + 0.5
        });
        let now = Instant::now();
        let mut out = Vec::with_capacity(evs.len());
        for e in evs.iter() {
            let decay = match e.kind {
                EventKind::Footstep => st.footstep_decay.max(0.2),
                EventKind::Gunshot => st.gunshot_decay.max(0.2),
            };
            let age = now.duration_since(e.born).as_secs_f32();
            let t = (age / decay).clamp(0.0, 1.0);
            out.push((e.clone(), 1.0 - t));
        }
        // 最新的排前面
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }
}
