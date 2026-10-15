//! 置顶、透明、点击穿透的全屏覆盖层（纯 Win32 实现）。
//! 独立线程运行：tiny-skia 绘制 → UpdateLayeredWindow 呈现（预乘 alpha）。
//! 支持：雷达模式 / 屏幕方位模式（手游式）/ 位置编辑（拖拽定位）。

use crate::settings::Settings;
use crate::state::{EventKind, SharedState, SoundEvent};
use anyhow::{bail, Result};
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_skia::{
    Color, FillRule, LineCap, Paint, Path, PathBuilder, Pixmap, Shader, Stroke, Transform,
};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetSystemMetrics, GetWindowLongPtrW,
    LoadCursorW, PeekMessageW, RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, UpdateLayeredWindow, GWLP_USERDATA, GWL_EXSTYLE, IDC_ARROW, MSG, PM_REMOVE,
    SM_CXSCREEN, SM_CYSCREEN, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SW_SHOWNOACTIVATE, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCHITTEST, WM_QUIT, WM_USER,
};

#[derive(Clone)]
struct Snap {
    visible: bool,
    mode: u8,
    opacity: f32,
    show_rings: bool,
    radar_radius: f32,
    offset: (i32, i32),
    dir_radius: f32,
    dir_icon_size: f32,
    dir_opacity: f32,
    dir_center: (i32, i32),
    dir_show_ring: bool,
    sectors: usize,
    foot: [u8; 3],
    gun: [u8; 3],
    ring: [u8; 3],
}

impl Snap {
    fn of(s: &Settings) -> Self {
        Snap {
            visible: s.overlay_visible,
            mode: s.display_mode,
            opacity: s.overlay_opacity,
            show_rings: s.show_rings,
            radar_radius: s.radar_radius,
            offset: (s.offset_x, s.offset_y),
            dir_radius: s.dir_radius,
            dir_icon_size: s.dir_icon_size,
            dir_opacity: s.dir_opacity,
            dir_center: (s.dir_center_x, s.dir_center_y),
            dir_show_ring: s.dir_show_ring,
            sectors: s.radar_sectors.clamp(4, 48) as usize,
            foot: s.footstep_color,
            gun: s.gunshot_color,
            ring: s.ring_color,
        }
    }
}

impl Default for Snap {
    fn default() -> Self {
        Snap::of(&Settings::default())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DragTarget {
    None,
    RadarCenter,
    DirCenter,
    DirIcon,
}

struct OverlayCtx {
    state: Arc<SharedState>,
    w: u32,
    h: u32,
    pixmap: Option<Pixmap>,
    dib: Option<DibSection>,
    snap: Snap,
    dirty: AtomicBool,
    had_content: bool,
    dragging: DragTarget,
    /// 上次的编辑状态（用于切换点击穿透并强制 Windows 重新命中测试）
    was_editing: bool,
}

/// 32bpp 顶层 DIB 段 + ULW 呈现
struct DibSection {
    hdc_screen: HDC,
    mem_dc: HDC,
    hbmp: HBITMAP,
    bits: *mut c_void,
    w: u32,
    h: u32,
    buf: Vec<u32>,
}

impl DibSection {
    fn new(w: u32, h: u32) -> Result<Self> {
        unsafe {
            let hdc_screen = GetDC(None);
            if hdc_screen.0.is_null() {
                bail!("GetDC 失败");
            }
            let mem_dc = CreateCompatibleDC(hdc_screen);
            if mem_dc.0.is_null() {
                bail!("CreateCompatibleDC 失败");
            }
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(h as i32), // 负值：自上而下
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0, // BI_RGB
                    biSizeImage: 0,
                    ..Default::default()
                },
                bmiColors: [Default::default(); 1],
            };
            let mut bits: *mut c_void = ptr::null_mut();
            let hbmp = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            if bits.is_null() {
                bail!("CreateDIBSection 未返回位图数据");
            }
            Ok(Self {
                hdc_screen,
                mem_dc,
                hbmp,
                bits,
                w,
                h,
                buf: vec![0u32; (w as usize) * (h as usize)],
            })
        }
    }

    fn present(&mut self, pix: &Pixmap, hwnd: HWND) -> Result<()> {
        unsafe {
            // tiny-skia 预乘 RGBA → ULW 预乘 BGRA
            for (dst, px) in self.buf.iter_mut().zip(pix.pixels()) {
                *dst = (u32::from(px.alpha()) << 24)
                    | (u32::from(px.blue()) << 16)
                    | (u32::from(px.green()) << 8)
                    | u32::from(px.red());
            }
            ptr::copy_nonoverlapping(
                self.buf.as_ptr() as *const u8,
                self.bits as *mut u8,
                self.buf.len() * 4,
            );
            let old = SelectObject(self.mem_dc, HGDIOBJ(self.hbmp.0));
            let blend = BLENDFUNCTION {
                BlendOp: 0, // AC_SRC_OVER
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: 1, // AC_SRC_ALPHA
            };
            let pt_dst = POINT { x: 0, y: 0 };
            let sz = SIZE {
                cx: self.w as i32,
                cy: self.h as i32,
            };
            let pt_src = POINT { x: 0, y: 0 };
            UpdateLayeredWindow(
                hwnd,
                self.hdc_screen,
                Some(&pt_dst),
                Some(&sz),
                self.mem_dc,
                Some(&pt_src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )?;
            let _ = SelectObject(self.mem_dc, old);
            Ok(())
        }
    }
}

impl Drop for DibSection {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.hbmp.0));
            let _ = DeleteDC(self.mem_dc);
            let _ = ReleaseDC(None, self.hdc_screen);
        }
    }
}

/// 自定义唤醒消息
const WM_WAKE: u32 = WM_USER + 1;

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ctx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayCtx;
    match msg {
        WM_WAKE => {
            if !ctx.is_null() {
                (*ctx).dirty.store(true, Ordering::Relaxed);
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            // 非编辑模式下所有点击穿透
            let editing = if ctx.is_null() {
                false
            } else {
                (&(*ctx).state).edit_mode.load(Ordering::Relaxed)
            };
            if editing {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            } else {
                LRESULT(-1) // HTTRANSPARENT
            }
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
            if !ctx.is_null() {
                (*ctx).handle_mouse(msg, lparam);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl OverlayCtx {
    fn handle_mouse(&mut self, msg: u32, lparam: LPARAM) {
        let lp = lparam.0 as u32;
        let x = ((lp & 0xFFFF) as i16) as i32 as f32;
        let y = (((lp >> 16) & 0xFFFF) as i16) as i32 as f32;
        let p = (x, y);
        match msg {
            WM_MOUSEMOVE => {
                if self.dragging != DragTarget::None {
                    self.apply_drag(p);
                }
            }
            WM_LBUTTONDOWN => {
                if !self.state.edit_mode.load(Ordering::Relaxed) {
                    return;
                }
                let (w, h) = (self.w as f32, self.h as f32);
                if hit_check(p, w, h) {
                    self.exit_edit(true);
                    return;
                }
                if hit_cancel(p, w, h) {
                    self.exit_edit(false);
                    return;
                }
                let st = self.state.settings.read();
                let radar_c = (w / 2.0 + st.offset_x as f32, h / 2.0 + st.offset_y as f32);
                let dir_c = (
                    w / 2.0 + st.dir_center_x as f32,
                    h / 2.0 + st.dir_center_y as f32,
                );
                let sample = (dir_c.0, dir_c.1 - st.dir_radius);
                drop(st);
                self.dragging = if dist(p, radar_c) < 28.0 {
                    DragTarget::RadarCenter
                } else if dist(p, dir_c) < 28.0 {
                    DragTarget::DirCenter
                } else if dist(p, sample) < 32.0 {
                    DragTarget::DirIcon
                } else {
                    DragTarget::None
                };
                self.dirty.store(true, Ordering::Relaxed);
            }
            WM_LBUTTONUP => {
                self.dragging = DragTarget::None;
            }
            _ => {}
        }
    }

    fn exit_edit(&mut self, save: bool) {
        self.state.edit_mode.store(false, Ordering::Relaxed);
        self.dragging = DragTarget::None;
        if save {
            self.state.settings.read().save();
        }
        self.dirty.store(true, Ordering::Relaxed);
        self.state.wake_overlay();
    }

    fn apply_drag(&mut self, p: (f32, f32)) {
        let (w, h) = (self.w as f32, self.h as f32);
        let mut st = self.state.settings.write();
        match self.dragging {
            DragTarget::RadarCenter => {
                st.offset_x = (p.0 - w / 2.0).round() as i32;
                st.offset_y = (p.1 - h / 2.0).round() as i32;
            }
            DragTarget::DirCenter => {
                st.dir_center_x = (p.0 - w / 2.0).round() as i32;
                st.dir_center_y = (p.1 - h / 2.0).round() as i32;
            }
            DragTarget::DirIcon => {
                let dc = (
                    w / 2.0 + st.dir_center_x as f32,
                    h / 2.0 + st.dir_center_y as f32,
                );
                let r = dist(p, dc);
                st.dir_radius = r.clamp(40.0, (w.min(h) / 2.0 - 60.0).max(40.0));
            }
            DragTarget::None => {}
        }
        drop(st);
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn draw_frame(&mut self, hwnd: HWND) {
        let st = self.state.settings.read().clone();
        self.snap = Snap::of(&st);
        let editing = self.state.edit_mode.load(Ordering::Relaxed);

        // 编辑模式切换时：切换 WS_EX_TRANSPARENT 并强制 Windows 重新命中测试，
        // 否则进入编辑后鼠标事件仍按旧的"穿透"缓存继续穿透，拖拽完全失效
        if editing != self.was_editing {
            self.was_editing = editing;
            unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let new_style = if editing {
                    style & !(WS_EX_TRANSPARENT.0 as isize)
                } else {
                    style | WS_EX_TRANSPARENT.0 as isize
                };
                let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOACTIVATE,
                );
            }
            log::info!("点击穿透切换：编辑={editing}");
        }

        let (w, h) = (self.w, self.h);
        if self.pixmap.is_none()
            || self.pixmap.as_ref().map(|p| p.width() != w || p.height() != h) == Some(true)
        {
            self.pixmap = Pixmap::new(w.max(1), h.max(1));
        }
        let Some(pix) = self.pixmap.as_mut() else {
            return;
        };
        let wf = w as f32;
        let hf = h as f32;
        let snap = &self.snap;
        let events = self.state.active_events();
        let radar_c = (wf / 2.0 + snap.offset.0 as f32, hf / 2.0 + snap.offset.1 as f32);
        let dir_c = (
            wf / 2.0 + snap.dir_center.0 as f32,
            hf / 2.0 + snap.dir_center.1 as f32,
        );

        if editing {
            // 灰色遮罩 + 元素 + 拖拽手柄 + 完成/取消按钮
            pix.fill(rgba([12, 14, 20], 0.5));
            render_radar(pix, snap, &events, radar_c);
            render_direction(pix, snap, &events, dir_c, wf, hf);
            draw_handle(pix, radar_c, HandleKind::Cross);
            draw_handle(pix, dir_c, HandleKind::Circle);
            draw_handle(pix, (dir_c.0, dir_c.1 - snap.dir_radius), HandleKind::Diamond);
            draw_check_button(pix, wf, hf);
            draw_cancel_button(pix, wf, hf);
        } else {
            pix.fill(Color::TRANSPARENT);
            if snap.visible {
                // 模式：0=雷达 1=屏幕方位 2=双显
                if snap.mode != 1 {
                    render_radar(pix, snap, &events, radar_c);
                }
                if snap.mode != 0 {
                    render_direction(pix, snap, &events, dir_c, wf, hf);
                }
            }
        }

        if let Some(dib) = self.dib.as_mut() {
            if let Err(e) = dib.present(pix, hwnd) {
                log::error!("覆盖层呈现失败: {e:#}");
            }
        }
    }
}

pub fn run_overlay(state: Arc<SharedState>) -> Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class_name = w!("FPSAudioRadarOverlay");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            bail!("RegisterClassW 失败");
        }
        let sw = GetSystemMetrics(SM_CXSCREEN).max(800) as u32;
        let sh = GetSystemMetrics(SM_CYSCREEN).max(600) as u32;
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE
            | WS_EX_TOOLWINDOW;
        let hwnd = CreateWindowExW(
            ex,
            class_name,
            w!(""),
            WS_POPUP,
            0,
            0,
            sw as i32,
            sh as i32,
            None,
            None,
            hinst,
            None,
        )?;
        let mut ctx = OverlayCtx {
            state: state.clone(),
            w: sw,
            h: sh,
            pixmap: None,
            dib: Some(DibSection::new(sw, sh)?),
            snap: Snap::default(),
            dirty: AtomicBool::new(true),
            had_content: false,
            dragging: DragTarget::None,
            was_editing: false,
        };
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut ctx as *mut OverlayCtx as isize);
        *state.wake.lock() = Some(hwnd.0 as isize);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        log::info!("覆盖层窗口已创建（全屏 {}x{}，ULW 呈现）", sw, sh);

        // 消息循环 + 定时重绘
        let mut msg = MSG::default();
        let mut last_draw = Instant::now() - Duration::from_secs(1);
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let editing = state.edit_mode.load(Ordering::Relaxed);
            let visible = state.settings.read().overlay_visible;
            let has_events = !state.active_events().is_empty();
            let dirty = ctx.dirty.swap(false, Ordering::Relaxed);
            let need = editing || (has_events && visible) || dirty;
            let now = Instant::now();
            if need && now.duration_since(last_draw) >= Duration::from_millis(16) {
                ctx.draw_frame(hwnd);
                ctx.had_content = true;
                last_draw = now;
            } else if !need && ctx.had_content {
                // 事件全部结束后清空最后一帧
                ctx.draw_frame(hwnd);
                ctx.had_content = false;
                last_draw = now;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

// ---------------- 绘制工具 ----------------

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0) * (a.0 - b.0) + (a.1 - b.1) * (a.1 - b.1)).sqrt()
}

fn paint(color: Color) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(color),
        anti_alias: true,
        ..Default::default()
    }
}

fn rgba(c: [u8; 3], a: f32) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// 屏幕坐标：0° = 正上方，顺时针
fn pt(cx: f32, cy: f32, r: f32, deg: f32) -> (f32, f32) {
    let t = deg.to_radians();
    (cx + r * t.sin(), cy - r * t.cos())
}

fn arc_path(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let (x0, y0) = pt(cx, cy, r, a0);
    pb.move_to(x0, y0);
    let steps = (((a1 - a0).abs() / 4.0).ceil() as usize).max(2);
    for i in 1..=steps {
        let (x, y) = pt(cx, cy, r, a0 + (a1 - a0) * i as f32 / steps as f32);
        pb.line_to(x, y);
    }
    pb.finish()
}

fn diamond_path(cx: f32, cy: f32, r: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - r);
    pb.line_to(cx + r, cy);
    pb.line_to(cx, cy + r);
    pb.line_to(cx - r, cy);
    pb.close();
    pb.finish()
}

fn fill_circle(pix: &mut Pixmap, c: (f32, f32), r: f32, color: Color) {
    if let Some(path) = PathBuilder::from_circle(c.0, c.1, r) {
        pix.fill_path(
            &path,
            &paint(color),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

fn stroke_circle(pix: &mut Pixmap, c: (f32, f32), r: f32, width: f32, color: Color) {
    if let Some(path) = PathBuilder::from_circle(c.0, c.1, r) {
        pix.stroke_path(
            &path,
            &paint(color),
            &Stroke {
                width,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

fn stroke_line(pix: &mut Pixmap, a: (f32, f32), b: (f32, f32), width: f32, color: Color) {
    let mut pb = PathBuilder::new();
    pb.move_to(a.0, a.1);
    pb.line_to(b.0, b.1);
    if let Some(path) = pb.finish() {
        pix.stroke_path(
            &path,
            &paint(color),
            &Stroke {
                width,
                line_cap: LineCap::Round,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

// ---------------- 雷达模式（近/中/远三层 + 细分刻度） ----------------

/// 三层边界（占半径比例）
const LAYER_NEAR: f32 = 0.28;
const LAYER_MID: f32 = 0.62;
const LAYER_FAR: f32 = 1.0;

/// 扇形网格单元：角度 [a0,a1] × 半径 [r0,r1]（圆弧用折线近似）
fn cell_path(cx: f32, cy: f32, r0: f32, r1: f32, a0: f32, a1: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let n = 10;
    let (sx, sy) = pt(cx, cy, r0, a0);
    pb.move_to(sx, sy);
    for i in 0..=n {
        let a = a0 + (a1 - a0) * i as f32 / n as f32;
        let (x, y) = pt(cx, cy, r1, a);
        pb.line_to(x, y);
    }
    for i in 0..=n {
        let a = a1 - (a1 - a0) * i as f32 / n as f32;
        let (x, y) = pt(cx, cy, r0, a);
        pb.line_to(x, y);
    }
    pb.close();
    pb.finish()
}

fn render_radar(pix: &mut Pixmap, snap: &Snap, events: &[(SoundEvent, f32)], c: (f32, f32)) {
    let (cx, cy) = c;
    let r = snap.radar_radius.max(20.0);
    let opacity = snap.opacity.clamp(0.0, 1.0);
    let n_sectors = snap.sectors.max(4) as f32;
    let step = 360.0 / n_sectors;

    if snap.show_rings {
        // 三层底色（同心填充，由外向内叠涂，层次分明）
        fill_circle(pix, c, r * LAYER_FAR, rgba(snap.ring, 0.028 * opacity));
        fill_circle(pix, c, r * LAYER_MID, rgba(snap.ring, 0.030 * opacity));
        fill_circle(pix, c, r * LAYER_NEAR, rgba(snap.ring, 0.034 * opacity));
        // 扇区分隔线（细、弱，与层环构成网格）
        let mut i = 0;
        while i < n_sectors as usize {
            let a = i as f32 * step;
            let (x0, y0) = pt(cx, cy, 0.0, a);
            let (x1, y1) = pt(cx, cy, r, a);
            stroke_line(pix, (x0, y0), (x1, y1), 1.0, rgba(snap.ring, 0.12 * opacity));
            i += 1;
        }
        // 层分隔环（比底色更清晰）
        for f in [LAYER_NEAR, LAYER_MID, LAYER_FAR] {
            stroke_circle(pix, c, r * f, 1.5, rgba(snap.ring, 0.34 * opacity));
        }
        // 细分刻度：15° 一小格，45° 一大格，四个正方向最长
        let mut deg: f32 = 0.0;
        while deg < 360.0 {
            let major = (deg % 45.0).abs() < 0.01;
            let cardinal = (deg % 90.0).abs() < 0.01;
            let (x0, y0, x1, y1) = if cardinal {
                let (a, b) = pt(cx, cy, r * 0.82, deg);
                let (c2, d2) = pt(cx, cy, r, deg);
                (a, b, c2, d2)
            } else if major {
                let (a, b) = pt(cx, cy, r * 0.90, deg);
                let (c2, d2) = pt(cx, cy, r, deg);
                (a, b, c2, d2)
            } else {
                let (a, b) = pt(cx, cy, r * 0.95, deg);
                let (c2, d2) = pt(cx, cy, r, deg);
                (a, b, c2, d2)
            };
            let alpha = if cardinal {
                0.45
            } else if major {
                0.34
            } else {
                0.16
            };
            stroke_line(pix, (x0, y0), (x1, y1), 1.0, rgba(snap.ring, alpha * opacity));
            deg += 15.0;
        }
    }

    for (ev, fade) in events.iter() {
        let a = fade * opacity;
        if a <= 0.01 {
            continue;
        }
        let (col, _is_gun) = match ev.kind {
            EventKind::Footstep => (snap.foot, false),
            EventKind::Gunshot => (snap.gun, true),
        };
        let th = ev.angle_deg;
        // 按距离分层：近层 / 中层 / 远层（单元格带）
        let dist = ev.distance.clamp(0.0, 1.0);
        let (inner_f, outer_f, _band_t) = if dist < 0.34 {
            (0.06, LAYER_NEAR * 0.90, dist / 0.34)
        } else if dist < 0.67 {
            (LAYER_NEAR * 1.06, LAYER_MID * 0.95, (dist - 0.34) / 0.33)
        } else {
            (LAYER_MID * 1.05, LAYER_FAR * 0.96, (dist - 0.67) / 0.33)
        };

        // 整格高亮：所属扇区 × 距离层
        let sec_a = (th / step).floor() * step;
        let sec_b = sec_a + step;
        if let Some(cell) = cell_path(cx, cy, r * inner_f, r * outer_f, sec_a, sec_b) {
            pix.fill_path(
                &cell,
                &paint(rgba(col, 0.20 * a)),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        // 格内图标（脚步 / 子弹，正立显示）
        let band_h = (outer_f - inner_f) * r;
        let icon_size = (band_h * 0.85).clamp(22.0, 64.0);
        let mid_a = (sec_a + sec_b) / 2.0;
        let mid_r = r * (inner_f + outer_f) / 2.0;
        let (icx, icy) = pt(cx, cy, mid_r, mid_a);
        let icon_path = match ev.kind {
            EventKind::Footstep => crate::svg::footstep_path(),
            EventKind::Gunshot => crate::svg::bullet_path(),
        };
        let t = crate::svg::icon_transform(icx, icy, icon_size, 0.0);
        pix.fill_path(icon_path, &paint(rgba(col, 0.92 * a)), FillRule::Winding, t, None);
        // 外环边缘刻度（角度指示）
        if let Some(tick) = arc_path(cx, cy, r, th - 8.0, th + 8.0) {
            pix.stroke_path(
                &tick,
                &paint(rgba(col, a)),
                &Stroke {
                    width: 4.0,
                    line_cap: LineCap::Round,
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }
    }
}

// ---------------- 屏幕方位模式（手游式） ----------------

fn render_direction(
    pix: &mut Pixmap,
    snap: &Snap,
    events: &[(SoundEvent, f32)],
    c: (f32, f32),
    w: f32,
    h: f32,
) {
    let opacity = snap.dir_opacity.clamp(0.0, 1.0);
    if snap.dir_show_ring {
        stroke_circle(pix, c, snap.dir_radius, 1.5, rgba(snap.ring, 0.14 * opacity));
        for i in 0..8 {
            let a = i as f32 * 45.0;
            let (x0, y0) = pt(c.0, c.1, snap.dir_radius * 0.94, a);
            let (x1, y1) = pt(c.0, c.1, snap.dir_radius, a);
            stroke_line(pix, (x0, y0), (x1, y1), 1.0, rgba(snap.ring, 0.1 * opacity));
        }
    }

    for (ev, fade) in events.iter() {
        let a = fade * opacity;
        if a <= 0.01 {
            continue;
        }
        let col = match ev.kind {
            EventKind::Footstep => snap.foot,
            EventKind::Gunshot => snap.gun,
        };
        // 方向置信度越高图标越大
        let s = snap.dir_icon_size
            * (1.25 - 0.35 * ev.distance.clamp(0.0, 1.0))
            * (0.9 + 0.25 * ev.strength.clamp(0.0, 1.0));
        let th = ev.angle_deg;
        let dir = (th.to_radians().sin(), -th.to_radians().cos());
        let mut p = (c.0 + snap.dir_radius * dir.0, c.1 + snap.dir_radius * dir.1);
        // 夹紧在屏幕内
        let m = s * 1.9;
        p.0 = p.0.clamp(m, (w - m).max(m));
        p.1 = p.1.clamp(m, (h - m).max(m));
        // 图标按方位旋转、尖端朝外（脚步自然朝上 / 子弹弹头自然朝右上 45°）
        let (icon, natural) = match ev.kind {
            EventKind::Footstep => (crate::svg::footstep_path(), crate::svg::FOOTSTEP_NATURAL),
            EventKind::Gunshot => (crate::svg::bullet_path(), crate::svg::BULLET_NATURAL),
        };
        fill_circle(pix, p, s * 0.72, rgba(col, 0.10 * a));
        let t = crate::svg::icon_transform(p.0, p.1, s * 1.4, th - natural);
        pix.fill_path(icon, &paint(rgba(col, 0.92 * a)), FillRule::Winding, t, None);
    }
}

// ---------------- 位置编辑模式 ----------------

#[derive(Clone, Copy)]
enum HandleKind {
    Cross,
    Circle,
    Diamond,
}

fn draw_handle(pix: &mut Pixmap, p: (f32, f32), kind: HandleKind) {
    fill_circle(pix, p, 15.0, rgba([255, 255, 255], 0.16));
    stroke_circle(pix, p, 15.0, 2.0, rgba([255, 255, 255], 0.85));
    let white = rgba([255, 255, 255], 0.95);
    match kind {
        HandleKind::Cross => {
            stroke_line(pix, (p.0 - 8.0, p.1), (p.0 + 8.0, p.1), 2.0, white);
            stroke_line(pix, (p.0, p.1 - 8.0), (p.0, p.1 + 8.0), 2.0, white);
        }
        HandleKind::Circle => fill_circle(pix, p, 5.0, white),
        HandleKind::Diamond => {
            if let Some(path) = diamond_path(p.0, p.1, 6.0) {
                pix.fill_path(
                    &path,
                    &paint(white),
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }
    }
}

fn draw_check_button(pix: &mut Pixmap, w: f32, _h: f32) {
    let p = (w - 56.0, 56.0);
    fill_circle(pix, p, 24.0, rgba([0, 200, 110], 0.95));
    let white = rgba([255, 255, 255], 1.0);
    stroke_line(pix, (p.0 - 9.0, p.1 - 1.0), (p.0 - 3.0, p.1 + 5.0), 4.0, white);
    stroke_line(pix, (p.0 - 3.0, p.1 + 5.0), (p.0 + 10.0, p.1 - 8.0), 4.0, white);
}

fn draw_cancel_button(pix: &mut Pixmap, w: f32, _h: f32) {
    let p = (w - 112.0, 56.0);
    fill_circle(pix, p, 24.0, rgba([120, 132, 148], 0.9));
    let white = rgba([255, 255, 255], 1.0);
    stroke_line(pix, (p.0 - 8.0, p.1 - 8.0), (p.0 + 8.0, p.1 + 8.0), 4.0, white);
    stroke_line(pix, (p.0 - 8.0, p.1 + 8.0), (p.0 + 8.0, p.1 - 8.0), 4.0, white);
}

fn hit_check(p: (f32, f32), w: f32, _h: f32) -> bool {
    dist(p, (w - 56.0, 56.0)) <= 32.0
}

fn hit_cancel(p: (f32, f32), w: f32, _h: f32) -> bool {
    dist(p, (w - 112.0, 56.0)) <= 32.0
}
