#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod detect;
mod overlay;
mod settings;
mod state;
mod svg;

use crate::settings::Settings;
use crate::state::SharedState;
use std::thread;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("FPS 可视化脚步器启动（公益无障碍版 v0.1）");

    // 进程级 DPI 感知（覆盖层用物理像素坐标）
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    // 主线程 COM 初始化（设置面板里枚举音频设备需要）
    audio::init_com_sta();

    let state = SharedState::new(Settings::load());

    // 音频采集线程
    let st = state.clone();
    thread::Builder::new()
        .name("audio".into())
        .spawn(move || audio::run_audio_thread(st))
        .expect("无法创建音频线程");

    // 覆盖层线程
    let st = state.clone();
    thread::Builder::new()
        .name("overlay".into())
        .spawn(move || {
            if let Err(e) = overlay::run_overlay(st) {
                log::error!("覆盖层线程退出: {e:#}");
            }
        })
        .expect("无法创建覆盖层线程");

    // 设置界面（主线程）
    if let Err(e) = app::run_settings_ui(state) {
        log::error!("设置界面退出: {e:#}");
    }
}
