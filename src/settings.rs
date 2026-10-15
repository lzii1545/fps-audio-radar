//! 用户可调参数与配置持久化（%APPDATA%\FPSAudioRadar\settings.json）

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ---- 采集 ----
    /// 输出设备名；空 = 系统默认输出设备
    pub device_name: String,

    // ---- 检测 ----
    /// 主增益（线性）
    pub master_gain: f32,
    /// 触发阈值：高于噪声地板多少 dB 才算事件
    pub onset_threshold_db: f32,
    /// 自动噪声地板
    pub auto_noise_floor: bool,
    /// 手动噪声地板（dBFS）
    pub manual_noise_floor_db: f32,
    /// 同类事件间方向平滑系数 0..1（越大越稳、响应越慢）
    pub smoothing: f32,
    /// 脚步频段下限 Hz
    pub footstep_low_hz: f32,
    /// 脚步频段上限 Hz
    pub footstep_high_hz: f32,
    /// 枪声高频判定起点 Hz
    pub gunshot_high_hz: f32,
    /// 脚步能量增益
    pub footstep_gain: f32,
    /// 枪声能量增益
    pub gunshot_gain: f32,

    // ---- 显示 ----
    /// 脚步同时显示的最大数量（1~10）
    #[serde(default = "default_max_footsteps")]
    pub max_footsteps: usize,
    /// 枪声同时显示的最大数量（1~10）
    #[serde(default = "default_max_gunshots")]
    pub max_gunshots: usize,
    /// 脚步渐隐时间（秒）
    #[serde(default = "default_footstep_decay")]
    pub footstep_decay: f32,
    /// 枪声渐隐时间（秒）
    #[serde(default = "default_gunshot_decay")]
    pub gunshot_decay: f32,
    /// 雷达半径（像素）
    pub radar_radius: f32,
    /// 覆盖层整体不透明度 0..1
    pub overlay_opacity: f32,
    pub footstep_color: [u8; 3],
    pub gunshot_color: [u8; 3],
    pub ring_color: [u8; 3],
    pub show_rings: bool,
    pub show_center: bool,
    pub overlay_visible: bool,
    /// 雷达扇区数（扇形网格：8/12/16/24）
    #[serde(default = "default_radar_sectors")]
    pub radar_sectors: u8,
    /// 雷达中心相对屏幕中心的偏移（物理像素）
    pub offset_x: i32,
    pub offset_y: i32,

    // ---- 显示模式 ----
    /// 0 = 雷达模式；1 = 屏幕方位模式（手游式）；2 = 双显
    pub display_mode: u8,

    // ---- 屏幕方位模式（手游式） ----
    /// 方位指示器相对方位中心的半径（物理像素）
    pub dir_radius: f32,
    /// 方位指示器图标基准大小（物理像素）
    pub dir_icon_size: f32,
    /// 方位指示器整体不透明度 0..1
    pub dir_opacity: f32,
    /// 方位中心相对屏幕中心的偏移（物理像素）
    pub dir_center_x: i32,
    pub dir_center_y: i32,
    /// 显示方位参考环
    pub dir_show_ring: bool,
}

/// 雷达扇区数默认值（扇形网格）
fn default_radar_sectors() -> u8 {
    12
}

fn default_max_footsteps() -> usize {
    5
}

fn default_max_gunshots() -> usize {
    5
}

fn default_footstep_decay() -> f32 {
    1.5
}

fn default_gunshot_decay() -> f32 {
    2.5
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            master_gain: 1.0,
            onset_threshold_db: 8.0,
            auto_noise_floor: true,
            manual_noise_floor_db: -60.0,
            smoothing: 0.6,
            footstep_low_hz: 60.0,
            footstep_high_hz: 450.0,
            gunshot_high_hz: 1500.0,
            footstep_gain: 1.0,
            gunshot_gain: 1.0,
            max_footsteps: 5,
            max_gunshots: 5,
            footstep_decay: 1.5,
            gunshot_decay: 2.5,
            radar_radius: 240.0,
            overlay_opacity: 0.9,
            footstep_color: [0, 220, 120],
            gunshot_color: [255, 110, 30],
            ring_color: [255, 255, 255],
            show_rings: true,
            show_center: true,
            overlay_visible: true,
            radar_sectors: 12,
            offset_x: 0,
            offset_y: 0,
            display_mode: 2,
            dir_radius: 260.0,
            dir_icon_size: 56.0,
            dir_opacity: 0.95,
            dir_center_x: 0,
            dir_center_y: 0,
            dir_show_ring: true,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("FPSAudioRadar").join("settings.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => {
                // 容错：剥离 UTF-8 BOM（部分编辑器/脚本会写入 BOM）
                let s = s.trim_start_matches('\u{feff}');
                serde_json::from_str(s).unwrap_or_else(|e| {
                    log::warn!("配置文件解析失败，使用默认参数: {e}");
                    Self::default()
                })
            }
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&p, s) {
                    log::warn!("保存配置失败: {e}");
                }
            }
            Err(e) => log::warn!("序列化配置失败: {e}"),
        }
    }
}
