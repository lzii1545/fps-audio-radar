//! 声音事件检测：分块能量 → 事件切分 → 方位计算 → 脚步/枪声分类。
//!
//! v0 使用轻量启发式特征（频带能量比 + 起音速度），结构上预留替换为
//! ONNX 模型（YAMNet/CRNN 微调）的空间。

use crate::audio::CaptureFormat;
use crate::settings::Settings;
use crate::state::{EventKind, SharedState, SoundEvent};
use std::time::{Duration, Instant};

enum FilterKind {
    BandPass,
    HighPass,
}

/// RBJ 双二阶滤波器
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn new(kind: FilterKind, sr: f32, f0: f32, q: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * (f0 / sr);
        let alpha = w.sin() / (2.0 * q);
        let cw = w.cos();
        let (b0, b1, b2) = match kind {
            FilterKind::BandPass => (alpha, 0.0, -alpha),
            FilterKind::HighPass => ((1.0 + cw) * 0.5, -(1.0 + cw), (1.0 + cw) * 0.5),
        };
        let a0 = 1.0 + alpha;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: (-2.0 * cw) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

struct BlockFeat {
    /// 全频段能量（mono 混音）
    e_full: f32,
    /// 脚步频段能量
    e_low: f32,
    /// 高频能量
    e_high: f32,
    /// 各声道能量
    e_ch: Vec<f32>,
    dur: f32,
}

pub struct Detector {
    sr: f32,
    bp: Biquad,
    hp: Biquad,
    /// 缓存的滤波器配置（脚低, 脚高, 枪高）
    cfg: (f32, f32, f32),
    noise_floor: f32,
    floor_ready: bool,
    init_min: f32,
    init_blocks: u32,
    in_event: bool,
    quiet_streak: u32,
    event_blocks: Vec<BlockFeat>,
    event_dur: f32,
    /// 每类事件最近一次的方向与时间（用于平滑）
    last: [(f32, Instant); 2],
    peak: f32,
}

impl Detector {
    pub fn new(sr: u32) -> Self {
        let sr = sr.max(8000) as f32;
        let bp = Biquad::new(FilterKind::BandPass, sr, 160.0, 0.7);
        let hp = Biquad::new(FilterKind::HighPass, sr, 1500.0, 0.7);
        Self {
            sr,
            bp,
            hp,
            cfg: (60.0, 450.0, 1500.0),
            noise_floor: 1e-6,
            floor_ready: false,
            init_min: f32::MAX,
            init_blocks: 0,
            in_event: false,
            quiet_streak: 0,
            event_blocks: Vec::new(),
            event_dur: 0.0,
            last: [(0.0, Instant::now() - Duration::from_secs(10)); 2],
            peak: -90.0,
        }
    }

    pub fn process_block(
        &mut self,
        samples: &[f32],
        fmt: &CaptureFormat,
        st: &Settings,
        state: &SharedState,
    ) {
        let ch = fmt.channels;
        let frames = samples.len() / ch;
        if frames == 0 {
            return;
        }

        // 滤波器参数变化时重建
        let cfg = (st.footstep_low_hz, st.footstep_high_hz, st.gunshot_high_hz);
        if cfg != self.cfg {
            self.cfg = cfg;
            let f0 = (cfg.0 * cfg.1).max(400.0).sqrt();
            let q = (f0 / (cfg.1 - cfg.0).max(10.0)).clamp(0.4, 8.0);
            self.bp = Biquad::new(FilterKind::BandPass, self.sr, f0, q);
            self.hp = Biquad::new(FilterKind::HighPass, self.sr, cfg.2, 0.7);
        }

        let gain = st.master_gain;
        let mut e_full = 0f32;
        let mut e_low = 0f32;
        let mut e_high = 0f32;
        let mut e_ch = vec![0f32; ch];
        for f in 0..frames {
            let base = f * ch;
            let mut mono = 0f32;
            let mut cnt = 0usize;
            for c in 0..ch {
                if fmt.is_lfe[c] {
                    continue;
                }
                let v = samples[base + c];
                mono += v;
                cnt += 1;
                e_ch[c] += v * v;
            }
            let m = if cnt > 0 { (mono / cnt as f32) * gain } else { 0.0 };
            e_full += m * m;
            let l = self.bp.process(m);
            e_low += l * l;
            let h = self.hp.process(m);
            e_high += h * h;
        }
        let inv = 1.0 / frames as f32;
        let feat = BlockFeat {
            e_full: e_full * inv,
            e_low: e_low * inv,
            e_high: e_high * inv,
            e_ch: e_ch.into_iter().map(|x| x * inv).collect(),
            dur: frames as f32 / self.sr,
        };

        // 电平表（供设置面板 VU 显示）
        let db = 20.0 * feat.e_full.max(1e-12).sqrt().log10();
        *state.level_db.lock() = db.clamp(-90.0, 0.0);
        self.peak = (self.peak * 0.9995).max(db);
        *state.peak_db.lock() = self.peak.clamp(-90.0, 0.0);

        // 噪声地板：自动模式取前 1 秒的最小能量作为初始地板，
        // 之后只在"存在真实噪声"时缓慢追踪（数字静音不更新，防止地板衰减到零）
        if st.auto_noise_floor {
            if !self.floor_ready {
                self.init_min = self.init_min.min(feat.e_full);
                self.init_blocks += 1;
                if self.init_blocks >= 100 {
                    self.noise_floor = self.init_min.max(1e-10);
                    self.floor_ready = true;
                }
            } else if feat.e_full < self.noise_floor * 2.0 && feat.e_full > 1e-8 {
                self.noise_floor += (feat.e_full - self.noise_floor) * 0.01;
                self.noise_floor = self.noise_floor.max(1e-10);
            } else {
                // 持续高于地板：环境噪声可能已升高（如音乐/引擎声），地板缓慢上浮（上限 -30dB）
                self.noise_floor = (self.noise_floor * 1.0004).min(1e-3);
            }
        } else {
            self.noise_floor = 10f32.powf(st.manual_noise_floor_db / 10.0);
        }

        let thresh = self.noise_floor * 10f32.powf(st.onset_threshold_db / 10.0);

        // 事件切分状态机
        if !self.in_event {
            if feat.e_full > thresh {
                self.in_event = true;
                self.quiet_streak = 0;
                self.event_blocks.clear();
                self.event_dur = 0.0;
                self.event_blocks.push(feat);
                self.event_dur += self.event_blocks.last().map(|b| b.dur).unwrap_or(0.0);
            }
        } else {
            let dur = feat.dur;
            let is_quiet = feat.e_full < thresh * 0.5;
            self.event_blocks.push(feat);
            self.event_dur += dur;
            if is_quiet {
                self.quiet_streak += 1;
            } else {
                self.quiet_streak = 0;
            }
            if self.quiet_streak >= 2 || self.event_dur > 0.5 {
                self.finalize_event(fmt, st, state);
                self.in_event = false;
                self.quiet_streak = 0;
            }
        }
    }

    fn finalize_event(&mut self, fmt: &CaptureFormat, st: &Settings, state: &SharedState) {
        let blocks = std::mem::take(&mut self.event_blocks);
        self.event_dur = 0.0;
        if blocks.is_empty() {
            return;
        }
        let dur: f32 = blocks.iter().map(|b| b.dur).sum();
        let e_full: f32 = blocks.iter().map(|b| b.e_full).sum();
        let e_low: f32 = blocks.iter().map(|b| b.e_low).sum::<f32>() * st.footstep_gain;
        let e_high: f32 = blocks.iter().map(|b| b.e_high).sum::<f32>() * st.gunshot_gain;
        let after = if blocks.len() > 1 {
            blocks[1..].iter().map(|b| b.e_full).sum::<f32>() / (blocks.len() - 1) as f32
        } else {
            0.0
        };
        let attack = blocks[0].e_full / (after + 1e-12);
        let low_ratio = e_low / (e_full + 1e-12);
        let high_ratio = e_high / (e_full + 1e-12);

        // 启发式分类（v0）：后续可替换为 ONNX 模型
        let kind = if high_ratio >= 0.22 && attack >= 3.0 && dur <= 0.4 {
            Some(EventKind::Gunshot)
        } else if low_ratio >= 0.45 && dur <= 0.3 && attack >= 1.6 {
            Some(EventKind::Footstep)
        } else {
            None
        };
        let kind = match kind {
            Some(k) => k,
            None => return, // 环境声/音乐，忽略
        };

        // 方位：各声道能量加权圆形均值
        let mut sx = 0f32;
        let mut sy = 0f32;
        let mut se = 0f32;
        for b in &blocks {
            for c in 0..b.e_ch.len() {
                if c >= fmt.channels || fmt.is_lfe[c] {
                    continue;
                }
                let e = b.e_ch[c];
                let a = fmt.channel_angles[c].to_radians();
                sx += e * a.cos();
                sy += e * a.sin();
                se += e;
            }
        }
        let strength = (sx * sx + sy * sy).sqrt() / (se + 1e-12);
        if strength < 0.15 {
            // 方向不明（正中声源/环境声），丢弃
            return;
        }
        let mut angle = sy.atan2(sx).to_degrees();

        // 同类事件间方向平滑
        let k = match kind {
            EventKind::Footstep => 0usize,
            EventKind::Gunshot => 1usize,
        };
        let now = Instant::now();
        if now.duration_since(self.last[k].1).as_secs_f32() < 0.6 {
            let prev = self.last[k].0;
            let mut d = angle - prev;
            while d > 180.0 {
                d -= 360.0;
            }
            while d < -180.0 {
                d += 360.0;
            }
            angle = prev + d * (1.0 - st.smoothing);
        }
        self.last[k] = (angle, now);

        // 距离：相对噪声地板的响度 → 0(近)..1(远)，仅近/中/远三档精度；
        // 越响越近：rel 6dB=远，30dB+=近
        let e_db = 10.0 * e_full.max(1e-12).log10();
        let floor_db = 10.0 * self.noise_floor.max(1e-12).log10();
        let rel = (e_db - floor_db).clamp(0.0, 40.0);
        // 距离：0=近（响），1=远（弱）；0~40dB 相对地板映射，保证近/中/远分层拉开
        let distance = 1.0 - (rel / 40.0).clamp(0.0, 1.0);

        let ev = SoundEvent {
            kind,
            angle_deg: angle,
            distance,
            strength,
            born: now,
        };
        log::debug!(
            "事件 {:?} @ {:.0}° 距离 {:.2} 强度 {:.2}",
            kind,
            angle,
            distance,
            strength
        );

        // 合并阈值跟随雷达扇区宽度（360° / 扇区数），12 扇区 = 30°
        let merge_deg = 360.0 / st.radar_sectors.clamp(4, 48) as f32;
        let (decay, max_same) = match kind {
            EventKind::Footstep => (st.footstep_decay.max(0.2), st.max_footsteps.max(1)),
            EventKind::Gunshot => (st.gunshot_decay.max(0.2), st.max_gunshots.max(1)),
        };

        let mut evs = state.events.lock();
        // 合并：同类 + 角度差 ≤ 合并阈值 + 仍在显示或刚消失 → 滑向新方位并刷新生命
        let mut merged = false;
        for e in evs.iter_mut() {
            if e.kind != kind {
                continue;
            }
            let mut d = angle - e.angle_deg;
            while d > 180.0 {
                d -= 360.0;
            }
            while d < -180.0 {
                d += 360.0;
            }
            if d.abs() <= merge_deg && e.born.elapsed().as_secs_f32() <= decay + 0.6 {
                // 上游已有事件间平滑，这里直接滑到新方位，避免快速移动时指示器滞后累积
                e.angle_deg = angle;
                e.distance = distance;
                e.strength = e.strength.max(strength);
                e.born = now;
                merged = true;
                break;
            }
        }
        if !merged {
            evs.push_front(ev);
            // 按类型上限裁剪（超出则移除最旧的同类）
            let mut count = 0usize;
            let mut remove_idx = None;
            for (i, e) in evs.iter().enumerate() {
                if e.kind == kind {
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
        }
        drop(evs);
        *state.event_counter.lock() += 1;
        // 通知覆盖层立即重绘
        state.wake_overlay();
    }
}
