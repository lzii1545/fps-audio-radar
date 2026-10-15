//! 设置面板（egui）：无边框窗口 + 深色设计系统 + 自绘组件（滑块/下拉/分段/按钮）
//! + 轻量动画引擎（缓动插值、悬停反馈、胶囊滑动、页面淡入）。

use crate::audio;
use crate::settings::Settings;
use crate::state::{CaptureStatus, EventKind, SharedState, SoundEvent};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

// ---------------- 设计变量 ----------------
const BG0: Color32 = Color32::from_rgb(10, 12, 17);
const BG1: Color32 = Color32::from_rgb(15, 17, 24);
const BG2: Color32 = Color32::from_rgb(21, 24, 33);
const BG3: Color32 = Color32::from_rgb(29, 33, 45);
const BORDER: Color32 = Color32::from_rgb(38, 43, 58);
const ACCENT: Color32 = Color32::from_rgb(0, 166, 255);
const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(0, 166, 255, 48);
const TEXT: Color32 = Color32::from_rgb(228, 231, 238);
const TEXT_DIM: Color32 = Color32::from_rgb(178, 185, 200);
const WHITE: Color32 = Color32::WHITE;
const GREEN: Color32 = Color32::from_rgb(46, 204, 113);
const RED: Color32 = Color32::from_rgb(242, 78, 88);
const YELLOW: Color32 = Color32::from_rgb(243, 186, 66);
const DANGER: Color32 = Color32::from_rgb(226, 70, 80);

// ---------------- 动画引擎 ----------------

/// 极简缓动动画器：每帧向目标值指数逼近（速度快慢可调），未收敛时请求重绘；
/// 拖拽/按住时暂停重绘请求、且最多 30fps 请求，避免窗口拖动卡顿
#[derive(Default)]
struct Animator {
    vals: HashMap<egui::Id, f32>,
    epochs: HashMap<egui::Id, u64>,
    last_repaint: Option<Instant>,
}

impl Animator {
    fn get(&mut self, ctx: &egui::Context, id: egui::Id, target: f32, speed: f32) -> f32 {
        let cur = self.vals.get(&id).copied().unwrap_or(target);
        let dt = ctx.input(|i| i.stable_dt.min(0.1));
        let k = 1.0 - (-speed * dt).exp();
        let next = cur + (target - cur) * k;
        self.vals.insert(id, next);
        let converged = (next - target).abs() <= 0.002;
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        if !converged && !pointer_down {
            let due = match self.last_repaint {
                None => true,
                Some(t) => t.elapsed().as_secs_f32() > 1.0 / 30.0,
            };
            if due {
                ctx.request_repaint();
                self.last_repaint = Some(Instant::now());
            }
        }
        next
    }

    /// 键（epoch）变化时从 0 渐入到 1，用于页面切换淡入
    fn fade_in(&mut self, ctx: &egui::Context, id: egui::Id, epoch: u64) -> f32 {
        if self.epochs.get(&id) != Some(&epoch) {
            self.epochs.insert(id, epoch);
            self.vals.insert(id, 0.0);
        }
        self.get(ctx, id, 1.0, 12.0)
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
    )
}

pub fn run_settings_ui(state: Arc<SharedState>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FPS 可视化脚步器 · 设置面板")
            .with_inner_size([960.0, 720.0])
            .with_min_inner_size([800.0, 560.0])
            .with_decorations(false) // 无边框：自定义标题栏
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "fps-audio-radar-settings",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(SettingsApp {
                state,
                last_saved: None,
                tab: Tab::Display,
                anim: Animator::default(),
                test: TestPanel {
                    kind: 0,
                    dist: 0,
                    angle: 45.0,
                },
            }))
        }),
    )
}

/// 加载系统中文字体作为回退字体
fn setup_fonts(ctx: &egui::Context) {
    let candidates: [(&str, &str); 5] = [
        ("C:\\Windows\\Fonts\\msyh.ttc", "Microsoft YaHei"),
        ("C:\\Windows\\Fonts\\msyh.ttf", "Microsoft YaHei"),
        ("C:\\Windows\\Fonts\\simhei.ttf", "SimHei"),
        ("C:\\Windows\\Fonts\\Deng.ttf", "DengXian"),
        ("C:\\Windows\\Fonts\\simsun.ttc", "SimSun"),
    ];
    for (path, name) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert(name.to_string(), egui::FontData::from_owned(bytes));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push(name.to_string());
            }
            ctx.set_fonts(fonts);
            log::info!("已加载中文字体：{name}（{path}）");
            return;
        }
    }
    log::warn!("未找到系统中文字体，界面中文可能显示为方框");
}

/// 深色主题 + 圆角 + 强调色
fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG1;
    v.window_fill = BG0;
    v.extreme_bg_color = BG0;
    v.faint_bg_color = BG1;
    v.window_rounding = egui::Rounding::same(12.0);
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.slider_trailing_fill = true;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = egui::Rounding::same(8.0);
    }
    v.widgets.noninteractive.bg_fill = BG2;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.bg_fill = BG3;
    v.widgets.inactive.weak_bg_fill = BG2;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_fill = Color32::from_rgb(39, 44, 60);
    v.widgets.hovered.weak_bg_fill = BG2;
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::from_rgb(125, 205, 255));
    v.widgets.active.bg_fill = ACCENT_SOFT;
    v.widgets.active.weak_bg_fill = BG3;
    v.widgets.active.fg_stroke = Stroke::new(1.5, ACCENT);
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    ctx.set_style(style);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Display,
    Detect,
    Position,
    About,
}

/// 声音位置测试面板状态（跨帧保持）
struct TestPanel {
    kind: u8,  // 0=脚步 1=枪声
    dist: u8,  // 0=近 1=中 2=远
    angle: f32,
}

struct SettingsApp {
    state: Arc<SharedState>,
    last_saved: Option<Instant>,
    tab: Tab,
    anim: Animator,
    test: TestPanel,
}

impl SettingsApp {
    fn maybe_save(&mut self, st: &Settings, force: bool) {
        let due = match self.last_saved {
            None => true,
            Some(t) => t.elapsed().as_secs_f32() > 2.0,
        };
        if force || due {
            st.save();
            self.last_saved = Some(Instant::now());
        }
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut st = self.state.settings.read().clone();
        let mut changed = false;
        let status = self.state.status.lock().clone();
        let level = *self.state.level_db.lock();
        let peak = *self.state.peak_db.lock();
        let total_events = *self.state.event_counter.lock();
        let editing = self.state.edit_mode.load(Ordering::Relaxed);

        title_bar(ctx, &status, level, peak, total_events, editing, &mut self.anim);
        sidebar(ctx, &mut self.tab, &mut self.anim);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG0).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| {
                ui.spacing_mut().slider_width = 230.0;
                // 页面切换淡入
                let fade = self
                    .anim
                    .fade_in(ctx, egui::Id::new("page_fade"), self.tab as u64);
                ui.set_opacity(fade);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(2.0);
                        match self.tab {
                            Tab::Detect => detect_page(
                                ui,
                                ctx,
                                &mut self.anim,
                                &mut st,
                                &mut changed,
                                &status,
                                &self.state,
                            ),
                            Tab::Display => display_page(
                                ui,
                                ctx,
                                &mut self.anim,
                                &mut st,
                                &mut changed,
                                &self.state,
                                &mut self.test,
                            ),
                            Tab::Position => position_page(
                                ui,
                                ctx,
                                &mut self.anim,
                                &mut st,
                                &mut changed,
                                &self.state,
                            ),
                            Tab::About => about_page(ui),
                        }
                        ui.add_space(8.0);
                    });
            });

        if changed {
            *self.state.settings.write() = st.clone();
            self.maybe_save(&st, false);
            self.state.wake_overlay();
        }
    }
}

// ---------------- 自定义标题栏（无边框窗口） ----------------

fn title_bar(
    ctx: &egui::Context,
    status: &CaptureStatus,
    level: f32,
    peak: f32,
    total: u64,
    editing: bool,
    anim: &mut Animator,
) {
    egui::TopBottomPanel::top("titlebar")
        .exact_height(44.0)
        .frame(
            egui::Frame::none()
                .fill(BG1)
                .inner_margin(egui::Margin::symmetric(0.0, 0.0)),
        )
        .show(ctx, |ui| {
            let full = ui.max_rect();
            // 整条可拖拽（先注册，后绘制的按钮会在其上层，点击优先）
            let drag = ui.interact(full, ui.id().with("title_drag"), Sense::drag());
            if drag.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            let p = ui.painter();
            // 品牌区
            p.text(
                Pos2::new(full.left() + 14.0, full.center().y),
                Align2::LEFT_CENTER,
                "脚步雷达",
                FontId::proportional(14.0),
                TEXT,
            );
            p.text(
                Pos2::new(full.left() + 82.0, full.center().y),
                Align2::LEFT_CENTER,
                "无障碍声波可视化 · 设置面板",
                FontId::proportional(11.0),
                TEXT_DIM,
            );
            // 状态点
            let (col, label) = if status.running {
                (GREEN, "运行中")
            } else if status.error.is_some() {
                (RED, "异常")
            } else {
                (YELLOW, "未运行")
            };
            let dot_x = full.left() + 268.0;
            p.circle_filled(Pos2::new(dot_x, full.center().y), 4.0, col);
            p.text(
                Pos2::new(dot_x + 10.0, full.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(11.5),
                col,
            );
            p.text(
                Pos2::new(dot_x + 62.0, full.center().y),
                Align2::LEFT_CENTER,
                format!("已捕获事件 {total}"),
                FontId::proportional(11.0),
                TEXT_DIM,
            );
            if editing {
                p.text(
                    Pos2::new(dot_x + 168.0, full.center().y),
                    Align2::LEFT_CENTER,
                    "位置调整中 · 点右上角绿色按钮完成",
                    FontId::proportional(11.5),
                    YELLOW,
                );
            }

            // 右侧：VU + 最小化 + 关闭（VU 与按钮之间留出间距，绝不重叠）
            let close_rect = Rect::from_center_size(
                Pos2::new(full.right() - 22.0, full.center().y),
                egui::vec2(30.0, 28.0),
            );
            let min_rect = Rect::from_center_size(
                Pos2::new(full.right() - 56.0, full.center().y),
                egui::vec2(30.0, 28.0),
            );
            // VU：以「数字右端 = 最小化按钮左边缘 - 12px」为锚点反向排布
            vu_at(
                ui,
                ctx,
                anim,
                min_rect.left() - 12.0,
                full.center().y,
                84.0,
                level,
                peak,
            );
            let min_resp = ui.interact(min_rect, ui.id().with("min_btn"), Sense::click());
            let close_resp = ui.interact(close_rect, ui.id().with("close_btn"), Sense::click());
            // 最小化：横线
            {
                let rect = min_resp.rect;
                let ha = anim.get(ctx, min_resp.id, if min_resp.hovered() { 1.0 } else { 0.0 }, 14.0);
                if ha > 0.01 {
                    p.rect_filled(rect, 7.0, mix(BG3, Color32::from_rgb(39, 44, 60), ha));
                }
                let c = mix(TEXT_DIM, TEXT, ha);
                p.line_segment(
                    [
                        Pos2::new(rect.center().x - 6.0, rect.center().y),
                        Pos2::new(rect.center().x + 6.0, rect.center().y),
                    ],
                    Stroke::new(1.8, c),
                );
            }
            // 关闭：叉
            {
                let rect = close_resp.rect;
                let ha = anim.get(ctx, close_resp.id, if close_resp.hovered() { 1.0 } else { 0.0 }, 14.0);
                if ha > 0.01 {
                    p.rect_filled(rect, 7.0, mix(BG3, DANGER, ha));
                }
                let c = mix(TEXT_DIM, WHITE, ha);
                for (a, b) in [
                    (Pos2::new(rect.center().x - 5.0, rect.center().y - 5.0), Pos2::new(rect.center().x + 5.0, rect.center().y + 5.0)),
                    (Pos2::new(rect.center().x - 5.0, rect.center().y + 5.0), Pos2::new(rect.center().x + 5.0, rect.center().y - 5.0)),
                ] {
                    p.line_segment([a, b], Stroke::new(1.8, c));
                }
            }
            if min_resp.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            if close_resp.clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
}

/// 平滑电平表（绘制在指定位置）
fn vu_at(
    ui: &egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    text_end_x: f32,
    y: f32,
    width: f32,
    level: f32,
    peak: f32,
) {
    let disp = anim.get(ctx, egui::Id::new("vu_level"), level, 25.0);
    // 以 text_end_x 为右边界反向排布：数字 | 间距 | 电平条
    let text_w = 54.0;
    let gap = 6.0;
    let bar_right = text_end_x - text_w - gap;
    let rect = Rect::from_min_max(
        Pos2::new(bar_right - width, y - 6.0),
        Pos2::new(bar_right, y + 6.0),
    );
    let p = ui.painter();
    p.rect_filled(rect, 6.0, BG3);
    let frac = ((disp + 60.0) / 60.0).clamp(0.0, 1.0);
    let col = if disp > -6.0 {
        RED
    } else if disp > -20.0 {
        YELLOW
    } else {
        GREEN
    };
    if frac > 0.005 {
        p.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(rect.width() * frac, rect.height())),
            6.0,
            col,
        );
    }
    let pk = rect.left() + rect.width() * ((peak + 60.0) / 60.0).clamp(0.0, 1.0);
    p.line_segment(
        [Pos2::new(pk, rect.top() - 1.5), Pos2::new(pk, rect.bottom() + 1.5)],
        Stroke::new(1.5, TEXT_DIM),
    );
    p.text(
        Pos2::new(text_end_x, y),
        Align2::RIGHT_CENTER,
        format!("{disp:>5.1} dB"),
        FontId::monospace(10.0),
        TEXT_DIM,
    );
}

// ---------------- 左侧导航 ----------------

fn sidebar(ctx: &egui::Context, tab: &mut Tab, anim: &mut Animator) {
    egui::SidePanel::left("nav")
        .resizable(false)
        .exact_width(166.0)
        .frame(
            egui::Frame::none()
                .fill(BG1)
                .inner_margin(egui::Margin::symmetric(10.0, 14.0)),
        )
        .show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("设置").size(11.0).color(TEXT_DIM));
            ui.add_space(2.0);
            nav_item(ui, ctx, anim, *tab == Tab::Display, "显示", || *tab = Tab::Display);
            nav_item(ui, ctx, anim, *tab == Tab::Detect, "检测", || *tab = Tab::Detect);
            nav_item(ui, ctx, anim, *tab == Tab::Position, "位置", || *tab = Tab::Position);
            nav_item(ui, ctx, anim, *tab == Tab::About, "关于", || *tab = Tab::About);
        });
}

fn nav_item(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    selected: bool,
    label: &str,
    on_click: impl FnOnce(),
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
    if resp.clicked() {
        on_click();
    }
    let sel = anim.get(ctx, resp.id, if selected { 1.0 } else { 0.0 }, 13.0);
    let hover = anim.get(ctx, resp.id.with("h"), if resp.hovered() { 1.0 } else { 0.0 }, 13.0);
    let p = ui.painter();
    if sel > 0.01 {
        p.rect_filled(rect, 8.0, mix(ACCENT_SOFT, BG2, 1.0 - sel * 0.0));
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.left(), rect.center().y - 10.0),
                Pos2::new(rect.left() + 3.0, rect.center().y + 10.0),
            ),
            1.5,
            mix(ACCENT, WHITE, 0.25 * sel),
        );
    } else if hover > 0.01 {
        p.rect_filled(rect, 8.0, Color32::from_white_alpha((14.0 * hover) as u8));
    }
    p.text(
        Pos2::new(rect.left() + 13.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        if selected {
            mix(TEXT_DIM, ACCENT, sel)
        } else {
            mix(TEXT_DIM, TEXT, hover)
        },
    );
    resp
}

// ---------------- 通用组件 ----------------

/// 圆角卡片（悬停边框微光）；闭包通过参数获得动画器（再借用，动画状态可跨帧保持）
fn card<R>(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    title: &str,
    contents: impl FnOnce(&mut egui::Ui, &mut Animator) -> R,
) -> R {
    let inner = egui::Frame::none()
        .fill(BG2)
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (r, _) = ui.allocate_exact_size(egui::vec2(3.0, 13.0), Sense::hover());
                ui.painter().rect_filled(r, 1.5, ACCENT);
                ui.label(RichText::new(title).size(13.5).strong().color(TEXT));
            });
            ui.add_space(4.0);
            contents(ui, anim)
        });
    // 悬停边框微光
    let rect = inner.response.rect;
    let hovered = rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()));
    let ha = anim.get(ctx, ui.id().with(title), if hovered { 1.0 } else { 0.0 }, 10.0);
    if ha > 0.01 {
        ui.painter()
            .rect_stroke(rect, 12.0, Stroke::new(1.0, mix(BORDER, ACCENT, ha * 0.5)));
    }
    ui.add_space(10.0);
    inner.inner
}

/// 自绘滑块：标签行（左标签 + 右数值芯片）+ 圆角轨道 + 圆形手柄
#[derive(Clone)]
struct SliderSpec {
    min: f32,
    max: f32,
    log: bool,
    integer: bool,
    display: String,
}

fn norm(v: f32, min: f32, max: f32, log: bool) -> f32 {
    let v = v.clamp(min, max);
    if log {
        ((v / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
    } else {
        (v - min) / (max - min)
    }
}

fn denorm(t: f32, min: f32, max: f32, log: bool) -> f32 {
    if log {
        min * (max / min).powf(t)
    } else {
        min + (max - min) * t
    }
}

fn hslider(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    label: &str,
    value: &mut f32,
    spec: &SliderSpec,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(TEXT_DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let galley = ui
                .painter()
                .layout_no_wrap(spec.display.clone(), FontId::monospace(11.0), ACCENT);
            let pad = 8.0;
            let (r, _) = ui.allocate_exact_size(
                egui::vec2(galley.size().x + pad * 2.0, 19.0),
                Sense::hover(),
            );
            ui.painter().rect_filled(r, 5.0, BG3);
            ui.painter().galley(
                Pos2::new(r.center().x - galley.size().x / 2.0, r.center().y - galley.size().y / 2.0),
                galley,
                ACCENT,
            );
        });
    });
    // 轨道与手柄
    let h = 24.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::click_and_drag());
    let rail_y = rect.center().y;
    let rail = Rect::from_min_max(
        Pos2::new(rect.left() + 8.0, rail_y - 2.0),
        Pos2::new(rect.right() - 8.0, rail_y + 2.0),
    );
    let t = norm(*value, spec.min, spec.max, spec.log);
    let hx = rail.left() + rail.width() * t;
    let p = ui.painter();
    p.rect_filled(rail, 2.0, BG3);
    if t > 0.001 {
        p.rect_filled(
            Rect::from_min_max(rail.min, Pos2::new(hx, rail.max.y)),
            2.0,
            ACCENT,
        );
    }
    let hovered = resp.hovered() || resp.dragged();
    let ha = anim.get(ctx, resp.id, if hovered { 1.0 } else { 0.0 }, 14.0);
    if resp.dragged() || resp.clicked() {
        if let Some(ptr) = resp.interact_pointer_pos() {
            let nt = ((ptr.x - rail.left()) / rail.width().max(1.0)).clamp(0.0, 1.0);
            *value = denorm(nt, spec.min, spec.max, spec.log);
            if spec.integer {
                *value = value.round();
            }
            changed = true;
        }
    }
    let r = 6.0 + 2.0 * ha;
    p.circle_filled(Pos2::new(hx, rail_y), r + 4.0, Color32::from_white_alpha((36.0 * ha) as u8));
    p.circle_filled(
        Pos2::new(hx, rail_y),
        r,
        if resp.dragged() { ACCENT } else { mix(WHITE, ACCENT, ha) },
    );
    p.circle_stroke(Pos2::new(hx, rail_y), r, Stroke::new(2.0, ACCENT));
    changed
}

/// 自绘下拉框：圆角按钮 + 弹出列表（展开动画、悬停高亮、选中 ✓）
fn combo_ui(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    label: &str,
    options: &[String],
    selected: &mut String,
) -> bool {
    let mut changed = false;
    ui.label(RichText::new(label).size(12.0).color(TEXT_DIM));
    ui.add_space(2.0);
    let id = ui.make_persistent_id("combo");
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    let open = ui.memory(|m| m.is_popup_open(id));
    let ha = anim.get(ctx, resp.id, if resp.hovered() || open { 1.0 } else { 0.0 }, 12.0);
    let p = ui.painter();
    p.rect_filled(rect, 8.0, mix(BG3, Color32::from_rgb(40, 45, 62), ha));
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, mix(BORDER, ACCENT, ha * 0.7)));
    p.text(
        Pos2::new(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        selected.as_str(),
        FontId::proportional(12.5),
        TEXT,
    );
    // 下三角箭头
    {
        let cx = rect.right() - 14.0;
        let cy = rect.center().y;
        p.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(cx - 4.0, cy - 2.0),
                Pos2::new(cx + 4.0, cy - 2.0),
                Pos2::new(cx, cy + 4.0),
            ],
            mix(TEXT_DIM, TEXT, ha),
            Stroke::NONE,
        ));
    }
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup_below_widget(ui, id, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(rect.width());
        // 显式深色背景，避免个别主题下弹出层为浅色导致文字不可见
        egui::Frame::none()
            .fill(BG2)
            .stroke(Stroke::new(1.0, BORDER))
            .inner_margin(egui::Margin::symmetric(2.0, 2.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for opt in options {
                        let is_sel = *selected == *opt;
                        let (row, rresp) = ui.allocate_exact_size(
                            egui::vec2(rect.width(), 28.0),
                            Sense::click(),
                        );
                        let ha2 = anim.get(ctx, rresp.id, if rresp.hovered() { 1.0 } else { 0.0 }, 14.0);
                        let p2 = ui.painter();
                        if is_sel {
                            p2.rect_filled(row, 6.0, ACCENT_SOFT);
                        } else if ha2 > 0.01 {
                            p2.rect_filled(row, 6.0, Color32::from_white_alpha((16.0 * ha2) as u8));
                        }
                        p2.text(
                            Pos2::new(row.left() + 10.0, row.center().y),
                            Align2::LEFT_CENTER,
                            opt.as_str(),
                            FontId::proportional(12.0),
                            if is_sel { ACCENT } else { TEXT },
                        );
                        if is_sel {
                            // 绘制对勾
                            let cx = row.right() - 13.0;
                            let cy = row.center().y;
                            p2.line_segment(
                                [
                                    Pos2::new(cx - 4.0, cy),
                                    Pos2::new(cx - 1.0, cy + 3.0),
                                ],
                                Stroke::new(1.8, ACCENT),
                            );
                            p2.line_segment(
                                [
                                    Pos2::new(cx - 1.0, cy + 3.0),
                                    Pos2::new(cx + 4.0, cy - 3.0),
                                ],
                                Stroke::new(1.8, ACCENT),
                            );
                        }
                        if rresp.clicked() {
                            *selected = opt.clone();
                            changed = true;
                        }
                    }
                });
            });
    });
    changed
}

/// 分段选择器：选中胶囊平滑滑动；salt 为控件唯一标识（多个分段控件必须不同，否则状态互相干扰）
fn segmented(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    salt: &str,
    selected: &mut u8,
    options: &[(u8, &str)],
) {
    let h = 36.0;
    let w = ui.available_width();
    let n = options.len() as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    ui.painter().rect_filled(rect, 9.0, BG3);
    let seg_w = (w - 6.0) / n;
    let target = options
        .iter()
        .position(|(v, _)| *v == *selected)
        .unwrap_or(0) as f32;
    let pos = anim.get(ctx, ui.id().with(("seg_pill", salt)), target, 16.0);
    let pill = Rect::from_min_size(
        Pos2::new(rect.left() + 3.0 + seg_w * pos, rect.top() + 3.0),
        egui::vec2(seg_w - 2.0, h - 6.0),
    );
    ui.painter().rect_filled(pill, 7.0, ACCENT);
    for (i, (val, label)) in options.iter().enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(rect.left() + 3.0 + seg_w * i as f32, rect.top() + 3.0),
            egui::vec2(seg_w - 2.0, h - 6.0),
        );
        let resp = ui.interact(r, ui.id().with((salt, val)), Sense::click());
        let is_active = *val == *selected;
        let ha = anim.get(ctx, resp.id, if resp.hovered() { 1.0 } else { 0.0 }, 12.0);
        if !is_active && ha > 0.01 {
            ui.painter()
                .rect_filled(r, 7.0, Color32::from_white_alpha((18.0 * ha) as u8));
        }
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(12.5),
            if is_active { WHITE } else { mix(TEXT_DIM, TEXT, ha) },
        );
        if resp.clicked() {
            *selected = *val;
        }
    }
}

#[derive(Clone, Copy)]
enum BtnKind {
    Primary,
    Ghost,
}

/// 自绘按钮：悬停提亮 + 按压缩放
fn btn(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    text: &str,
    kind: BtnKind,
) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(12.5), TEXT);
    let w = galley.size().x + 40.0;
    let h = 30.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), Sense::click());
    let hover = anim.get(ctx, resp.id, if resp.hovered() { 1.0 } else { 0.0 }, 14.0);
    let down = anim.get(
        ctx,
        resp.id.with("press"),
        if resp.is_pointer_button_down_on() { 1.0 } else { 0.0 },
        24.0,
    );
    let p = ui.painter();
    let (base, textcol) = match kind {
        BtnKind::Primary => (ACCENT, WHITE),
        BtnKind::Ghost => (BG3, TEXT),
    };
    let rect2 = rect.shrink2(egui::vec2(0.0, 1.5 * down));
    p.rect_filled(rect2, 8.0, mix(base, WHITE, 0.12 * hover));
    if matches!(kind, BtnKind::Ghost) {
        p.rect_stroke(rect2, 8.0, Stroke::new(1.0, mix(BORDER, ACCENT, hover * 0.5)));
    }
    p.galley(
        Pos2::new(rect2.center().x - galley.size().x / 2.0, rect2.center().y - galley.size().y / 2.0),
        galley,
        textcol,
    );
    resp
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(TEXT_DIM));
}

/// 自绘颜色方块：圆角色块 + 弹出深色颜色选择器
fn color_swatch(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    label: &str,
    color: &mut [u8; 3],
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(label).size(12.0).color(TEXT_DIM));
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::click());
        let ha = anim.get(ctx, resp.id, if resp.hovered() { 1.0 } else { 0.0 }, 14.0);
        let p = ui.painter();
        let r = rect.shrink(1.0);
        p.rect_filled(r, 7.0, Color32::from_rgb(color[0], color[1], color[2]));
        p.rect_stroke(
            r,
            7.0,
            Stroke::new(1.5, Color32::from_white_alpha((64.0 + 130.0 * ha) as u8)),
        );
        let id = ui.make_persistent_id(("swatch", label));
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        egui::popup_below_widget(
            ui,
            id,
            &resp,
            egui::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                egui::Frame::none()
                    .fill(BG2)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        let mut c = Color32::from_rgb(color[0], color[1], color[2]);
                        if egui::color_picker::color_picker_color32(
                            ui,
                            &mut c,
                            egui::color_picker::Alpha::Opaque,
                        ) {
                            color[0] = c.r();
                            color[1] = c.g();
                            color[2] = c.b();
                            changed = true;
                        }
                    });
            },
        );
    });
    changed
}

// ---------------- 检测页 ----------------

fn detect_page(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    st: &mut Settings,
    changed: &mut bool,
    status: &CaptureStatus,
    state: &SharedState,
) {
    card(ui, ctx, anim, "声音采集", |ui, anim| {
        let mut selected = if st.device_name.is_empty() {
            "（默认输出设备）".to_string()
        } else {
            st.device_name.clone()
        };
        let mut options: Vec<String> = vec![audio::TEST_DEVICE_LABEL.to_string(), "（默认输出设备）".to_string()];
        options.extend(status.devices.iter().cloned());
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width() - 100.0);
            if combo_ui(ui, ctx, anim, "", &options, &mut selected) {
                let new_device = if selected == "（默认输出设备）" {
                    String::new()
                } else {
                    selected.clone()
                };
                if new_device != st.device_name {
                    st.device_name = new_device;
                    *changed = true;
                }
            }
            if btn(ui, ctx, anim, "刷新列表", BtnKind::Ghost).clicked() {
                match audio::enumerate_devices() {
                    Ok(names) => {
                        state.status.lock().devices = names;
                    }
                    Err(e) => log::error!("枚举设备失败: {e:#}"),
                }
            }
        });
        ui.add_space(6.0);
        hint(
            ui,
            "立体声只能分辨左右。想要 360° 方位（含前后）：安装 7.1 虚拟声卡（VB-Audio / \
             Voicemeeter），游戏以 7.1 声道输出到该设备并开启监听，再在这里选择它。",
        );
        hint(ui, "「（测试）WAV 回放」可用 test_sound.wav 验证全链路，无需声卡。");
    });

    card(ui, ctx, anim, "事件触发", |ui, anim| {
        let spec = SliderSpec {
            min: 0.1,
            max: 4.0,
            log: true,
            integer: false,
            display: format!("{:.2}×", st.master_gain),
        };
        *changed |= hslider(ui, ctx, anim, "主增益", &mut st.master_gain, &spec);
        ui.add_space(2.0);
        let spec = SliderSpec {
            min: 3.0,
            max: 24.0,
            log: false,
            integer: false,
            display: format!("{:.1} dB", st.onset_threshold_db),
        };
        *changed |= hslider(ui, ctx, anim, "触发阈值", &mut st.onset_threshold_db, &spec);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            *changed |= ui.checkbox(&mut st.auto_noise_floor, "自动噪声地板（推荐）").changed();
            if !st.auto_noise_floor {
                *changed |= ui
                    .add(
                        egui::Slider::new(&mut st.manual_noise_floor_db, -90.0..=-20.0)
                            .suffix(" dB"),
                    )
                    .changed();
            }
        });
        ui.add_space(2.0);
        let spec = SliderSpec {
            min: 0.0,
            max: 0.9,
            log: false,
            integer: false,
            display: format!("{:.0}%", st.smoothing * 100.0),
        };
        *changed |= hslider(ui, ctx, anim, "方向平滑", &mut st.smoothing, &spec);
    });

    ui.columns(2, |cols| {
        card(&mut cols[0], ctx, anim, "脚步声识别", |ui, anim| {
            let spec = SliderSpec {
                min: 30.0,
                max: 200.0,
                log: true,
                integer: false,
                display: format!("{:.0} Hz", st.footstep_low_hz),
            };
            *changed |= hslider(ui, ctx, anim, "频段下限", &mut st.footstep_low_hz, &spec);
            ui.add_space(2.0);
            let spec = SliderSpec {
                min: 200.0,
                max: 1200.0,
                log: true,
                integer: false,
                display: format!("{:.0} Hz", st.footstep_high_hz),
            };
            *changed |= hslider(ui, ctx, anim, "频段上限", &mut st.footstep_high_hz, &spec);
            ui.add_space(2.0);
            let spec = SliderSpec {
                min: 0.2,
                max: 4.0,
                log: true,
                integer: false,
                display: format!("{:.2}×", st.footstep_gain),
            };
            *changed |= hslider(ui, ctx, anim, "识别增益", &mut st.footstep_gain, &spec);
        });
        card(&mut cols[1], ctx, anim, "枪声识别", |ui, anim| {
            let spec = SliderSpec {
                min: 500.0,
                max: 4000.0,
                log: true,
                integer: false,
                display: format!("{:.0} Hz", st.gunshot_high_hz),
            };
            *changed |= hslider(ui, ctx, anim, "高频判定起点", &mut st.gunshot_high_hz, &spec);
            ui.add_space(2.0);
            let spec = SliderSpec {
                min: 0.2,
                max: 4.0,
                log: true,
                integer: false,
                display: format!("{:.2}×", st.gunshot_gain),
            };
            *changed |= hslider(ui, ctx, anim, "识别增益", &mut st.gunshot_gain, &spec);
        });
    });
}

// ---------------- 显示页 ----------------

fn display_page(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    st: &mut Settings,
    changed: &mut bool,
    state: &SharedState,
    test: &mut TestPanel,
) {
    card(ui, ctx, anim, "显示模式", |ui, anim| {
        let mut mode = st.display_mode;
        segmented(ui, ctx, anim, "mode", &mut mode, &[(0, "雷达"), (1, "屏幕方位"), (2, "双显")]);
        if mode != st.display_mode {
            st.display_mode = mode;
            *changed = true;
        }
        ui.add_space(4.0);
        hint(
            ui,
            "雷达：以准星为中心的方位环。屏幕方位：像手游一样在对应方向直接显示图标。\
             位置/大小/透明度各自独立，可在「位置」页拖拽定位。",
        );
    });

    ui.columns(2, |cols| {
        card(&mut cols[0], ctx, anim, "雷达模式", |ui, anim| {
            let spec = SliderSpec {
                min: 80.0,
                max: 500.0,
                log: false,
                integer: false,
                display: format!("{:.0} px", st.radar_radius),
            };
            *changed |= hslider(ui, ctx, anim, "雷达半径", &mut st.radar_radius, &spec);
            ui.add_space(2.0);
            let spec = SliderSpec {
                min: 0.1,
                max: 1.0,
                log: false,
                integer: false,
                display: format!("{:.0}%", st.overlay_opacity * 100.0),
            };
            *changed |= hslider(ui, ctx, anim, "透明度", &mut st.overlay_opacity, &spec);
            ui.add_space(4.0);
            ui.label(RichText::new("扇区数（扇形网格）").size(12.0).color(TEXT_DIM));
            ui.add_space(2.0);
            let mut sectors = st.radar_sectors;
            segmented(
                ui,
                ctx,
                anim,
                "sectors",
                &mut sectors,
                &[(8, "8"), (12, "12"), (16, "16"), (24, "24")],
            );
            if sectors != st.radar_sectors {
                st.radar_sectors = sectors;
                *changed = true;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                *changed |= ui.checkbox(&mut st.show_rings, "距离环与刻度").changed();
            });
        });
        card(&mut cols[1], ctx, anim, "屏幕方位模式", |ui, anim| {
            let spec = SliderSpec {
                min: 24.0,
                max: 140.0,
                log: false,
                integer: false,
                display: format!("{:.0} px", st.dir_icon_size),
            };
            *changed |= hslider(ui, ctx, anim, "图标大小", &mut st.dir_icon_size, &spec);
            ui.add_space(2.0);
            let spec = SliderSpec {
                min: 60.0,
                max: 520.0,
                log: false,
                integer: false,
                display: format!("{:.0} px", st.dir_radius),
            };
            *changed |= hslider(ui, ctx, anim, "方位环半径", &mut st.dir_radius, &spec);
            ui.add_space(2.0);
            let spec = SliderSpec {
                min: 0.1,
                max: 1.0,
                log: false,
                integer: false,
                display: format!("{:.0}%", st.dir_opacity * 100.0),
            };
            *changed |= hslider(ui, ctx, anim, "透明度", &mut st.dir_opacity, &spec);
            ui.add_space(4.0);
            *changed |= ui.checkbox(&mut st.dir_show_ring, "显示参考环").changed();
        });
    });

    card(ui, ctx, anim, "事件与颜色", |ui, anim| {
        ui.columns(2, |cols| {
            {
                let mut v = st.max_footsteps as f32;
                let spec = SliderSpec {
                    min: 1.0,
                    max: 10.0,
                    log: false,
                    integer: true,
                    display: format!("{} 个", v.round()),
                };
                if hslider(&mut cols[0], ctx, anim, "脚步数量上限", &mut v, &spec) {
                    st.max_footsteps = v.round() as usize;
                    *changed = true;
                }
                let mut v = st.max_gunshots as f32;
                let spec = SliderSpec {
                    min: 1.0,
                    max: 10.0,
                    log: false,
                    integer: true,
                    display: format!("{} 个", v.round()),
                };
                if hslider(&mut cols[0], ctx, anim, "枪声数量上限", &mut v, &spec) {
                    st.max_gunshots = v.round() as usize;
                    *changed = true;
                }
            }
            {
                let mut v = st.footstep_decay;
                let spec = SliderSpec {
                    min: 0.5,
                    max: 8.0,
                    log: false,
                    integer: false,
                    display: format!("{:.1} 秒", v),
                };
                if hslider(&mut cols[1], ctx, anim, "脚步保留时长", &mut v, &spec) {
                    st.footstep_decay = v;
                    *changed = true;
                }
                let mut v = st.gunshot_decay;
                let spec = SliderSpec {
                    min: 0.5,
                    max: 8.0,
                    log: false,
                    integer: false,
                    display: format!("{:.1} 秒", v),
                };
                if hslider(&mut cols[1], ctx, anim, "枪声保留时长", &mut v, &spec) {
                    st.gunshot_decay = v;
                    *changed = true;
                }
            }
        });
        ui.add_space(4.0);
        *changed |= ui.checkbox(&mut st.overlay_visible, "覆盖层总开关").changed();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            *changed |= color_swatch(ui, ctx, anim, "脚步", &mut st.footstep_color);
            *changed |= color_swatch(ui, ctx, anim, "枪声", &mut st.gunshot_color);
            *changed |= color_swatch(ui, ctx, anim, "参考环", &mut st.ring_color);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if btn(ui, ctx, anim, "恢复默认参数", BtnKind::Ghost).clicked() {
                let dev = st.device_name.clone();
                *st = Settings::default();
                st.device_name = dev;
                *changed = true;
            }
            if btn(ui, ctx, anim, "立即保存配置", BtnKind::Ghost).clicked() {
                st.save();
            }
        });
    });

    card(ui, ctx, anim, "实时预览", |ui, _anim| {
        let events = state.active_events();
        let canvas = egui::vec2(ui.available_width(), 250.0);
        let (rect, _) = ui.allocate_exact_size(canvas, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10.0, BG0);
        painter.rect_stroke(rect, 10.0, Stroke::new(1.0, BORDER));
        let left = Rect::from_min_size(rect.min + egui::vec2(24.0, 16.0), egui::vec2(210.0, 210.0));
        let right =
            Rect::from_min_size(rect.min + egui::vec2(rect.width() - 234.0, 16.0), egui::vec2(210.0, 210.0));
        for (r, label) in [(&left, "雷达模式"), (&right, "屏幕方位模式")] {
            let chip_w = 66.0;
            let chip = Rect::from_min_size(
                Pos2::new(r.center().x - chip_w / 2.0, r.top() - 10.0),
                egui::vec2(chip_w, 18.0),
            );
            painter.rect_filled(chip, 5.0, BG3);
            painter.text(
                chip.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(10.5),
                TEXT_DIM,
            );
        }
        // 直接拖动预览中心 = 调整位置（2 倍缩放映射到真实像素）
        let radar_drag = ui.interact(left, ui.id().with("preview_radar_drag"), Sense::drag());
        if radar_drag.dragged() {
            if let Some(p) = radar_drag.interact_pointer_pos() {
                st.offset_x = ((p.x - left.center().x) * 2.0).round().clamp(-400.0, 400.0) as i32;
                st.offset_y = ((p.y - left.center().y) * 2.0).round().clamp(-400.0, 400.0) as i32;
                *changed = true;
            }
        }
        let dir_drag = ui.interact(right, ui.id().with("preview_dir_drag"), Sense::drag());
        if dir_drag.dragged() {
            if let Some(p) = dir_drag.interact_pointer_pos() {
                st.dir_center_x =
                    ((p.x - right.center().x) * 2.0).round().clamp(-400.0, 400.0) as i32;
                st.dir_center_y =
                    ((p.y - right.center().y) * 2.0).round().clamp(-400.0, 400.0) as i32;
                *changed = true;
            }
        }
        let radar_off = egui::vec2(st.offset_x as f32 / 2.0, st.offset_y as f32 / 2.0);
        let dir_off = egui::vec2(st.dir_center_x as f32 / 2.0, st.dir_center_y as f32 / 2.0);
        preview_radar(&painter, left, st, &events, radar_off);
        preview_direction(&painter, right, st, &events, dir_off);
        hint(
            ui,
            "可直接拖动两个预览框内的图形来调整位置（实时生效）。\
             方位：0° = 正前方（屏幕上方），顺时针旋转；图标越大 / 越靠近中心 = 越近。",
        );
    });

    card(ui, ctx, anim, "声音位置测试", |ui, anim| {
        segmented(ui, ctx, anim, "test_kind", &mut test.kind, &[(0, "脚步"), (1, "枪声")]);
        ui.add_space(4.0);
        segmented(ui, ctx, anim, "test_dist", &mut test.dist, &[(0, "近"), (1, "中"), (2, "远")]);
        ui.add_space(4.0);
        let spec = SliderSpec {
            min: -180.0,
            max: 180.0,
            log: false,
            integer: true,
            display: format!("{:.0}°", test.angle),
        };
        let _ = hslider(
            ui,
            ctx,
            anim,
            "测试角度（0° = 正前，-90° = 左，90° = 右）",
            &mut test.angle,
            &spec,
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if btn(ui, ctx, anim, "播放测试音", BtnKind::Primary).clicked() {
                let kind = if test.kind == 0 {
                    EventKind::Footstep
                } else {
                    EventKind::Gunshot
                };
                let distance = match test.dist {
                    0 => 0.0,
                    1 => 0.5,
                    _ => 1.0,
                };
                crate::audio::push_test_sound(state, kind, test.angle, distance);
            }
        });
        ui.add_space(4.0);
        hint(
            ui,
            "任意角度（360°）都能精确显示：0° = 正前，±90° = 左右，180° = 正后；\
             近/中/远对应雷达的三层环带。测试直接注入显示层，检测链路可用「（测试）WAV 回放」验证。",
        );
    });
}

// ---------------- 位置页 ----------------

fn position_page(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anim: &mut Animator,
    st: &mut Settings,
    changed: &mut bool,
    state: &SharedState,
) {
    card(ui, ctx, anim, "拖拽定位（推荐）", |ui, anim| {
        hint(ui, "进入调整模式后，覆盖层会全屏显示灰色遮罩，直接拖动元素到想要的位置：");
        ui.add_space(4.0);
        ui.label(RichText::new("　＋　十字手柄 —— 雷达中心（对准准星）").size(12.5).color(TEXT));
        ui.label(RichText::new("　○　圆形手柄 —— 屏幕方位指示器的中心").size(12.5).color(TEXT));
        ui.label(RichText::new("　◆　菱形手柄 —— 方位环半径（拖远拖近）").size(12.5).color(TEXT));
        ui.label(RichText::new("　调整完成后点右上角绿色按钮保存，灰色按钮取消").size(12.5).color(TEXT_DIM));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if btn(ui, ctx, anim, "进入位置调整模式", BtnKind::Primary).clicked() {
                state.edit_mode.store(true, Ordering::SeqCst);
                state.wake_overlay();
            }
            if btn(ui, ctx, anim, "退出并保存", BtnKind::Ghost).clicked() {
                state.edit_mode.store(false, Ordering::SeqCst);
                st.save();
                state.wake_overlay();
            }
            if btn(ui, ctx, anim, "退出不保存", BtnKind::Ghost).clicked() {
                state.edit_mode.store(false, Ordering::SeqCst);
                state.wake_overlay();
            }
        });
        ui.add_space(4.0);
        hint(ui, "调整期间设置面板会被遮罩盖住，直接在遮罩上点绿色按钮完成。");
    });

    card(ui, ctx, anim, "数值微调（备选）", |ui, anim| {
        ui.columns(2, |cols| {
            {
                let mut v = st.offset_x as f32;
                let spec = SliderSpec {
                    min: -800.0,
                    max: 800.0,
                    log: false,
                    integer: true,
                    display: format!("{} px", v.round()),
                };
                if hslider(&mut cols[0], ctx, anim, "雷达中心偏移 X", &mut v, &spec) {
                    st.offset_x = v.round() as i32;
                    *changed = true;
                }
                let mut v = st.offset_y as f32;
                let spec = SliderSpec {
                    min: -800.0,
                    max: 800.0,
                    log: false,
                    integer: true,
                    display: format!("{} px", v.round()),
                };
                if hslider(&mut cols[0], ctx, anim, "雷达中心偏移 Y", &mut v, &spec) {
                    st.offset_y = v.round() as i32;
                    *changed = true;
                }
            }
            {
                let mut v = st.dir_center_x as f32;
                let spec = SliderSpec {
                    min: -800.0,
                    max: 800.0,
                    log: false,
                    integer: true,
                    display: format!("{} px", v.round()),
                };
                if hslider(&mut cols[1], ctx, anim, "方位中心偏移 X", &mut v, &spec) {
                    st.dir_center_x = v.round() as i32;
                    *changed = true;
                }
                let mut v = st.dir_center_y as f32;
                let spec = SliderSpec {
                    min: -800.0,
                    max: 800.0,
                    log: false,
                    integer: true,
                    display: format!("{} px", v.round()),
                };
                if hslider(&mut cols[1], ctx, anim, "方位中心偏移 Y", &mut v, &spec) {
                    st.dir_center_y = v.round() as i32;
                    *changed = true;
                }
            }
        });
        ui.add_space(2.0);
        let spec = SliderSpec {
            min: 60.0,
            max: 520.0,
            log: false,
            integer: false,
            display: format!("{:.0} px", st.dir_radius),
        };
        *changed |= hslider(ui, ctx, anim, "方位环半径", &mut st.dir_radius, &spec);
    });
}

// ---------------- 关于页 ----------------

fn about_page(ui: &mut egui::Ui) {
    egui::Frame::none()
        .fill(BG2)
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("项目定位").size(13.5).strong().color(TEXT));
            ui.add_space(2.0);
            ui.label(
                RichText::new(
                    "帮助听障玩家「看见」声音方向的开源公益工具：实时把游戏中的脚步声、枪声等，\
                     以雷达或屏幕方位图标的方式显示在覆盖层上。",
                )
                .size(12.5)
                .color(TEXT),
            );
            ui.add_space(10.0);
            ui.label(RichText::new("使用提示").size(13.5).strong().color(TEXT));
            ui.add_space(2.0);
            for line in [
                "1. 游戏内切换为【无边框窗口】模式（独占全屏会挡住覆盖层）；",
                "2. 推荐 7.1 虚拟声卡（VB-Audio / Voicemeeter）获得前后 360° 方位；",
                "3. 在竞技对战中使用前，建议先了解该游戏的规则与反作弊政策。",
            ] {
                ui.label(RichText::new(line).size(12.5).color(TEXT_DIM));
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(
                    "愿景：推动游戏厂商把声音可视化做成官方无障碍功能，\
                     参考《堡垒之夜》内置的「可视化音效」。",
                )
                .size(12.5)
                .color(TEXT_DIM),
            );
        });
    ui.add_space(10.0);
}

// ---------------- 预览绘制 ----------------

fn preview_radar(
    p: &egui::Painter,
    rect: Rect,
    st: &Settings,
    events: &[(SoundEvent, f32)],
    center_offset: egui::Vec2,
) {
    let c = rect.center() + center_offset;
    let r = (rect.width().min(rect.height()) / 2.0) - 10.0;
    // 中心偏移标记（十字）
    p.line_segment(
        [
            Pos2::new(rect.center().x - 4.0, rect.center().y),
            Pos2::new(rect.center().x + 4.0, rect.center().y),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(60)),
    );
    p.line_segment(
        [
            Pos2::new(rect.center().x, rect.center().y - 4.0),
            Pos2::new(rect.center().x, rect.center().y + 4.0),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(60)),
    );
    let ring_col = Color32::from_rgb(st.ring_color[0], st.ring_color[1], st.ring_color[2]);
    let n_sectors = (st.radar_sectors.clamp(4, 48)) as f32;
    let step = 360.0 / n_sectors;
    if st.show_rings {
        // 三层底色 + 分隔环（与覆盖层一致）
        p.circle_filled(c, r * 1.0, ring_col.gamma_multiply(0.028));
        p.circle_filled(c, r * 0.62, ring_col.gamma_multiply(0.030));
        p.circle_filled(c, r * 0.28, ring_col.gamma_multiply(0.034));
        // 扇区分隔线
        let mut i = 0;
        while i < n_sectors as usize {
            let a = (i as f32 * step).to_radians();
            p.line_segment(
                [c, Pos2::new(c.x + r * a.sin(), c.y - r * a.cos())],
                Stroke::new(1.0, ring_col.gamma_multiply(0.12)),
            );
            i += 1;
        }
        for f in [0.28, 0.62, 1.0] {
            p.circle_stroke(c, r * f, Stroke::new(1.2, ring_col.gamma_multiply(0.34)));
        }
        // 15° 细分刻度：正方向最长、45° 次之
        let mut deg: f32 = 0.0;
        while deg < 360.0 {
            let cardinal = (deg % 90.0).abs() < 0.01;
            let major = (deg % 45.0).abs() < 0.01;
            let inner = if cardinal {
                0.82
            } else if major {
                0.90
            } else {
                0.95
            };
            let alpha = if cardinal {
                0.45
            } else if major {
                0.34
            } else {
                0.16
            };
            let a0 = deg.to_radians();
            p.line_segment(
                [
                    Pos2::new(c.x + r * inner * a0.sin(), c.y - r * inner * a0.cos()),
                    Pos2::new(c.x + r * a0.sin(), c.y - r * a0.cos()),
                ],
                Stroke::new(1.0, ring_col.gamma_multiply(alpha)),
            );
            deg += 15.0;
        }
    }
    let fcol = Color32::from_rgb(st.footstep_color[0], st.footstep_color[1], st.footstep_color[2]);
    let gcol = Color32::from_rgb(st.gunshot_color[0], st.gunshot_color[1], st.gunshot_color[2]);
    for (ev, fade) in events.iter() {
        let col = match ev.kind {
            EventKind::Footstep => fcol,
            EventKind::Gunshot => gcol,
        };
        // 按距离分层
        let dist = ev.distance.clamp(0.0, 1.0);
        let (inner_f, outer_f, _band_t) = if dist < 0.34 {
            (0.06, 0.28 * 0.90, dist / 0.34)
        } else if dist < 0.67 {
            (0.28 * 1.06, 0.62 * 0.95, (dist - 0.34) / 0.33)
        } else {
            (0.62 * 1.05, 1.0 * 0.96, (dist - 0.67) / 0.33)
        };
        // 整格高亮（扇区 × 层带）+ 格内图标
        let sec_a = (ev.angle_deg / step).floor() * step;
        let sec_b = sec_a + step;
        let mut pts = Vec::new();
        let sub = 10;
        for i in 0..=sub {
            let a = (sec_a + (sec_b - sec_a) * i as f32 / sub as f32).to_radians();
            pts.push(c + egui::vec2(r * outer_f * a.sin(), -r * outer_f * a.cos()));
        }
        for i in 0..=sub {
            let a = (sec_b - (sec_b - sec_a) * i as f32 / sub as f32).to_radians();
            pts.push(c + egui::vec2(r * inner_f * a.sin(), -r * inner_f * a.cos()));
        }
        p.add(egui::Shape::convex_polygon(
            pts,
            col.gamma_multiply(0.20 * fade),
            Stroke::NONE,
        ));
        // 格内 SVG 图标（正立）
        let band_h = (outer_f - inner_f) * r;
        let icon_size = (band_h * 0.85).clamp(22.0, 64.0);
        let mid_a = (sec_a + sec_b) / 2.0;
        let mid_r = r * (inner_f + outer_f) / 2.0;
        let icx = c.x + mid_r * mid_a.to_radians().sin();
        let icy = c.y - mid_r * mid_a.to_radians().cos();
        let icon = match ev.kind {
            EventKind::Footstep => crate::svg::footstep_path(),
            EventKind::Gunshot => crate::svg::bullet_path(),
        };
        draw_svg_icon_strokes(p, icx, icy, icon_size, 0.0, icon, col.gamma_multiply(*fade));
        let a0 = (ev.angle_deg - 8.0).to_radians();
        let a1 = (ev.angle_deg + 8.0).to_radians();
        p.line_segment(
            [
                Pos2::new(c.x + r * a0.sin(), c.y - r * a0.cos()),
                Pos2::new(c.x + r * a1.sin(), c.y - r * a1.cos()),
            ],
            Stroke::new(4.0, col.gamma_multiply(*fade)),
        );
    }
}

/// 预览：SVG 图标折线描边（缩放 + 旋转）
fn draw_svg_icon_strokes(
    p: &egui::Painter,
    cx: f32,
    cy: f32,
    size: f32,
    deg: f32,
    path: &tiny_skia::Path,
    col: Color32,
) {
    let k = size / 1024.0;
    for poly in crate::svg::flattened_polylines(path, 8) {
        if poly.len() < 2 {
            continue;
        }
        let pts: Vec<Pos2> = poly
            .iter()
            .map(|(x, y)| {
                let (rx, ry) = preview_rot(((x - 512.0) * k, (y - 512.0) * k), deg);
                Pos2::new(cx + rx, cy + ry)
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.6, col)));
    }
}

fn preview_direction(
    p: &egui::Painter,
    rect: Rect,
    st: &Settings,
    events: &[(SoundEvent, f32)],
    center_offset: egui::Vec2,
) {
    let c = rect.center() + center_offset;
    let r = (rect.width().min(rect.height()) / 2.0) - 26.0;
    p.line_segment(
        [
            Pos2::new(rect.center().x - 4.0, rect.center().y),
            Pos2::new(rect.center().x + 4.0, rect.center().y),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(60)),
    );
    p.line_segment(
        [
            Pos2::new(rect.center().x, rect.center().y - 4.0),
            Pos2::new(rect.center().x, rect.center().y + 4.0),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(60)),
    );
    let ring_col = Color32::from_rgb(st.ring_color[0], st.ring_color[1], st.ring_color[2]);
    if st.dir_show_ring {
        p.circle_stroke(c, r, Stroke::new(1.2, ring_col.gamma_multiply(0.14)));
    }
    let fcol = Color32::from_rgb(st.footstep_color[0], st.footstep_color[1], st.footstep_color[2]);
    let gcol = Color32::from_rgb(st.gunshot_color[0], st.gunshot_color[1], st.gunshot_color[2]);
    for (ev, fade) in events.iter() {
        let col = match ev.kind {
            EventKind::Footstep => fcol,
            EventKind::Gunshot => gcol,
        };
        let s = 15.0 * (1.25 - 0.35 * ev.distance);
        let t = ev.angle_deg.to_radians();
        let dir = egui::vec2(t.sin(), -t.cos());
        let mut pos = c + dir * r;
        let m = s * 1.9;
        pos.x = pos.x.clamp(rect.left() + m, rect.right() - m);
        pos.y = pos.y.clamp(rect.top() + m, rect.bottom() - m);
        let th = ev.angle_deg;
        // 光晕 + SVG 图标（按方位旋转，尖端朝外）
        p.circle_filled(pos, s * 0.72, col.gamma_multiply(0.10 * fade));
        let (icon, natural) = match ev.kind {
            EventKind::Footstep => (crate::svg::footstep_path(), crate::svg::FOOTSTEP_NATURAL),
            EventKind::Gunshot => (crate::svg::bullet_path(), crate::svg::BULLET_NATURAL),
        };
        draw_svg_icon_strokes(
            p,
            pos.x,
            pos.y,
            s * 1.4,
            th - natural,
            icon,
            col.gamma_multiply(*fade),
        );
    }
}

/// 预览：局部坐标点按方位旋转（0° = 朝上）
fn preview_rot(local: (f32, f32), deg: f32) -> (f32, f32) {
    let a = deg.to_radians();
    (
        local.0 * a.cos() - local.1 * a.sin(),
        local.0 * a.sin() + local.1 * a.cos(),
    )
}
