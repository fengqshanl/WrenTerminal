//! 界面绘制：状态栏、标签栏、详情面板、终端区域与整体样式。

use crate::app::WrenApp;
use crate::backend::BackendCommand;
use crate::font::{FontSettings, TerminalFont};
use crate::format;
use crate::session::{SessionKind, SessionState};
use crate::stats::Snapshot;
use crate::theme::{ColorPalette, TerminalTheme};
use crate::view::TerminalView;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Pos2, Rect, RichText,
    Sense, Stroke, Vec2, Widget,
};

pub const BG: Color32 = Color32::from_rgb(8, 12, 20);
pub const PANEL: Color32 = Color32::from_rgb(10, 16, 26);
pub const CARD: Color32 = Color32::from_rgb(17, 26, 41);
pub const CARD_ALT: Color32 = Color32::from_rgb(14, 22, 36);
pub const CARD_HOVER: Color32 = Color32::from_rgb(22, 34, 52);
pub const BORDER: Color32 = Color32::from_rgb(30, 45, 68);
pub const TEXT: Color32 = Color32::from_rgb(214, 226, 240);
pub const DIM: Color32 = Color32::from_rgb(126, 143, 166);
pub const ACCENT: Color32 = Color32::from_rgb(61, 220, 151);
pub const BLUE: Color32 = Color32::from_rgb(90, 169, 255);
pub const AMBER: Color32 = Color32::from_rgb(255, 180, 84);
pub const RED: Color32 = Color32::from_rgb(255, 107, 107);
pub const GREEN: Color32 = ACCENT;
pub const TOAST_BG: Color32 = Color32::from_rgb(30, 42, 60);

pub fn apply_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = CARD_ALT;
    visuals.hyperlink_color = BLUE;
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(61, 220, 151, 70);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };

    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.bg_fill = CARD;
    w.inactive.weak_bg_fill = CARD;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.hovered.bg_fill = CARD_HOVER;
    w.hovered.weak_bg_fill = CARD_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(52, 78, 112));
    w.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    w.active.bg_fill = Color32::from_rgb(28, 44, 66);
    w.active.weak_bg_fill = Color32::from_rgb(28, 44, 66);
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.window_margin = Margin::same(12);
    style.spacing.interact_size.y = 22.0;
    // 滚动条：使用常驻（非浮动）的宽滚动条，避免在深色背景里看不清。
    style.spacing.scroll.floating = false;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.floating_width = 10.0;
    style.spacing.scroll.bar_inner_margin = 3.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.spacing.scroll.handle_min_length = 30.0;
    style.spacing.scroll.foreground_color = true;
    style.text_styles.insert(
        egui::TextStyle::Body,
        FontId::new(13.5, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        FontId::new(13.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        FontId::new(11.5, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(18.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        FontId::new(13.0, egui::FontFamily::Monospace),
    );
    ctx.set_style(style);
}

pub fn terminal_theme() -> TerminalTheme {
    let mut p = ColorPalette::default();
    p.foreground = "#d6e2f0".into();
    p.background = "#0a101a".into();
    p.black = "#0a101a".into();
    p.red = "#ff6b72".into();
    p.green = "#41d69a".into();
    p.yellow = "#f2c46d".into();
    p.blue = "#6cb6ff".into();
    p.magenta = "#c792ea".into();
    p.cyan = "#5fd7d7".into();
    p.white = "#cdd9e5".into();
    p.bright_black = "#5b6b80".into();
    p.bright_red = "#ff8a90".into();
    p.bright_green = "#6ff0b8".into();
    p.bright_yellow = "#ffe08a".into();
    p.bright_blue = "#9ad1ff".into();
    p.bright_magenta = "#e0aaff".into();
    p.bright_cyan = "#8af0ea".into();
    p.bright_white = "#f4f8fc".into();
    p.bright_foreground = Some("#f4f8fc".into());
    p.dim_foreground = "#78889c".into();
    TerminalTheme::new(Box::new(p))
}

fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::NONE
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            30,
        ))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).color(color).strong());
        });
}

fn usage_color(pct: f32) -> Color32 {
    if pct >= 90.0 {
        RED
    } else if pct >= 70.0 {
        AMBER
    } else {
        GREEN
    }
}

/// 一个带迷你进度条的指标胶囊。
fn metric(
    ui: &mut egui::Ui,
    label: &str,
    value: impl Into<String>,
    frac: Option<f32>,
    color: Color32,
) -> egui::Response {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(9, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(RichText::new(label).size(10.3).color(DIM));
                ui.label(RichText::new(value.into()).size(12.0).color(color).strong());
            });
            if let Some(f) = frac {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(86.0, 3.0), Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::same(2), Color32::from_rgb(34, 48, 68));
                let width = rect.width() * f.clamp(0.0, 1.0);
                if width > 0.0 {
                    p.rect_filled(
                        Rect::from_min_size(rect.min, Vec2::new(width, rect.height())),
                        CornerRadius::same(2),
                        color,
                    );
                }
            }
        })
        .response
}

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.2, color);
}

pub fn status_bar(app: &mut WrenApp, ctx: &egui::Context) {
    let Some(idx) = app.active_index() else {
        return;
    };
    let session = &app.sessions[idx];
    let snap = session.stats.as_ref().map(|s| s.latest()).unwrap_or_default();
    let has_stats = session.stats.is_some();
    let is_remote = session.kind.is_remote();
    let badge_text = session.kind.badge();
    let target = session.kind.target();
    let is_local = matches!(session.kind, SessionKind::Local);
    let host = if !snap.host.trim().is_empty() {
        snap.host.clone()
    } else if is_local {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".into())
    } else {
        target.clone()
    };
    let os = if snap.os.trim().is_empty() {
        "正在采集状态…".to_string()
    } else {
        snap.os.clone()
    };
    let details_open = app.details_open;
    let screen_w = ctx.screen_rect().width();
    let wide = screen_w >= 1080.0;
    let error = snap.error.clone();
    let connected = snap.connected;
    let updated = snap.updated;

    let dot = if !has_stats {
        Color32::from_rgb(90, 100, 116)
    } else if error.is_some() {
        RED
    } else if connected {
        GREEN
    } else {
        AMBER
    };
    let state_text = if !has_stats {
        "状态轮询关闭"
    } else if error.is_some() {
        "状态离线"
    } else if snap.updated.is_none() {
        "正在连接"
    } else {
        "实时"
    };

    let mut toggle_details = false;
    let mut open_connect = false;
    let mut toggle_top = false;
    let always_on_top = app.cfg.always_on_top;

    egui::TopBottomPanel::top("wren-status")
        .frame(
            egui::Frame::NONE
                .fill(PANEL)
                .inner_margin(Margin::symmetric(10, 6))
                .stroke(Stroke::new(1.0_f32, BORDER)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("◆ WREN").size(13.0).color(ACCENT).strong())
                    .on_hover_text(format!(
                        "Wren v{} · 本地 / WSL / SSH",
                        env!("CARGO_PKG_VERSION")
                    ));
                badge(ui, badge_text, if is_remote { BLUE } else { ACCENT });
                status_dot(ui, dot);
                ui.label(
                    RichText::new(format::shorten(&host, 30))
                        .size(13.5)
                        .color(TEXT)
                        .strong(),
                )
                .on_hover_text(format!("{target}\n{}", snap.os));
                if wide {
                    ui.label(
                        RichText::new(format::shorten(&os, 34))
                            .size(10.8)
                            .color(DIM),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("＋ 连接").size(11.5).color(ACCENT),
                            )
                            .frame(false),
                        )
                        .on_hover_text("新建本地 / WSL / SSH 连接  (Ctrl+Shift+N)")
                        .clicked()
                    {
                        open_connect = true;
                    }
                    if ui
                        .selectable_label(
                            details_open,
                            RichText::new("详情").size(11.5),
                        )
                        .on_hover_text("展开/收起系统详情  (F12)")
                        .clicked()
                    {
                        toggle_details = true;
                    }
                    if ui
                        .selectable_label(
                            always_on_top,
                            RichText::new("置顶").size(11.5),
                        )
                        .on_hover_text("窗口保持在最前")
                        .clicked()
                    {
                        toggle_top = true;
                    }
                    ui.label(RichText::new(format::clock()).size(12.0).color(TEXT));
                    if is_remote || !has_stats {
                        ui.label(
                            RichText::new(format!("{} · {}", state_text, format::age(updated)))
                                .size(10.3)
                                .color(dot),
                        );
                    }
                });
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if !has_stats {
                    metric(ui, "状态", "未启用远端轮询", None, AMBER)
                        .on_hover_text("在 SSH 配置中开启“轮询远端系统状态”");
                } else if let Some(err) = &error {
                    metric(ui, "状态", "离线", None, RED)
                        .on_hover_text(format!("{err}\n\nSSH 状态轮询需要免密登录（密钥或 ssh-agent）。"));
                }

                if snap.updated.is_some() {
                    let cpu = snap.cpu_usage.clamp(0.0, 100.0);
                    metric(ui, "CPU", format::pct(cpu), Some(cpu / 100.0), usage_color(cpu))
                        .on_hover_text(format!(
                            "{} · {} 核",
                            if snap.cpu_model.is_empty() {
                                "CPU"
                            } else {
                                &snap.cpu_model
                            },
                            snap.cores.len()
                        ));
                    let mem = snap.mem_pct();
                    metric(
                        ui,
                        "内存",
                        format!(
                            "{} / {}",
                            format::bytes(snap.mem_used),
                            format::bytes(snap.mem_total)
                        ),
                        Some(mem / 100.0),
                        usage_color(mem),
                    )
                    .on_hover_text(format!("可用 {}", format::bytes(snap.mem_available)));
                    let disk = snap.disk_pct();
                    metric(
                        ui,
                        "磁盘",
                        format!(
                            "{} / {}",
                            format::bytes(snap.disk_used),
                            format::bytes(snap.disk_total)
                        ),
                        Some(disk / 100.0),
                        usage_color(disk),
                    )
                    .on_hover_text("系统盘 / 根分区");
                    metric(
                        ui,
                        "网络",
                        format!(
                            "↓ {}  ↑ {}",
                            format::rate(snap.net_rx_rate),
                            format::rate(snap.net_tx_rate)
                        ),
                        None,
                        BLUE,
                    );
                    if wide {
                        metric(
                            ui,
                            "负载",
                            snap.load
                                .map(|l| format!("{:.2} {:.2} {:.2}", l[0], l[1], l[2]))
                                .unwrap_or_else(|| "—".into()),
                            None,
                            TEXT,
                        );
                        metric(
                            ui,
                            "运行",
                            if snap.uptime > 0 {
                                format::duration(snap.uptime)
                            } else {
                                "—".into()
                            },
                            None,
                            TEXT,
                        );
                        if is_remote && has_stats {
                            metric(ui, "时延", format::latency(snap.latency), None, TEXT);
                        }
                        if snap.processes > 0 {
                            metric(ui, "进程", snap.processes.to_string(), None, TEXT);
                        }
                    }
                } else if is_local {
                    ui.label(
                        RichText::new("正在读取系统状态…")
                            .size(11.5)
                            .color(DIM),
                    );
                } else if has_stats {
                    metric(ui, "状态", "正在连接…", None, AMBER);
                }
            });
        });

    if toggle_details {
        app.details_open = !app.details_open;
        app.cfg.details_open = app.details_open;
        app.cfg.save();
    }
    if toggle_top {
        app.set_always_on_top(!always_on_top, ctx);
    }
    if open_connect {
        app.open_connect();
    }
}

pub fn tab_bar(app: &mut WrenApp, ctx: &egui::Context) {
    let mut select: Option<usize> = None;
    let mut close: Option<usize> = None;
    let mut new_kind: Option<SessionKind> = None;
    let mut open_connect = false;
    let mut new_ssh_form = false;

    egui::TopBottomPanel::top("wren-tabs")
        .frame(
            egui::Frame::NONE
                .fill(BG)
                .inner_margin(Margin::symmetric(8, 3))
                .stroke(Stroke::new(1.0_f32, BORDER)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("wren-tab-scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for (i, s) in app.sessions.iter().enumerate() {
                                let active = i == app.active;
                                let (dot, title) = match s.state {
                                    SessionState::Running => (
                                        if active { ACCENT } else { DIM },
                                        s.title(),
                                    ),
                                    SessionState::Exited(_) => (RED, s.title()),
                                    SessionState::Failed => (RED, s.title()),
                                };
                                let title = format::shorten(&title, 28);
                                egui::Frame::NONE
                                    .fill(if active { CARD_HOVER } else { Color32::TRANSPARENT })
                                    .corner_radius(CornerRadius::same(6))
                                    .stroke(if active {
                                        Stroke::new(1.0_f32, BORDER)
                                    } else {
                                        Stroke::NONE
                                    })
                                    .inner_margin(Margin::symmetric(8, 4))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 4.0;
                                            let (rect, _) = ui.allocate_exact_size(
                                                Vec2::splat(7.0),
                                                Sense::hover(),
                                            );
                                            ui.painter().circle_filled(
                                                rect.center(),
                                                3.0,
                                                dot,
                                            );
                                            let label = ui.add(
                                                egui::Button::new(
                                                    RichText::new(title)
                                                        .size(12.0)
                                                        .color(if active { TEXT } else { DIM }),
                                                )
                                                .frame(false),
                                            );
                                            if label.clicked() {
                                                select = Some(i);
                                            }
                                            if label.double_clicked() {
                                                // 双击标签重连当前会话
                                                new_kind = Some(s.kind.clone());
                                                close = Some(i);
                                            }
                                            let close_btn = ui.add(
                                                egui::Button::new(
                                                    RichText::new("×").size(12.0).color(DIM),
                                                )
                                                .frame(false),
                                            );
                                            if close_btn.clicked() {
                                                close = Some(i);
                                            }
                                        });
                                    })
                                    .response
                                    .on_hover_text(match s.term_title() {
                                        Some(t) => format!("{}\n终端标题：{}", s.kind.target(), t),
                                        None => s.kind.target(),
                                    });
                                ui.add_space(2.0);
                            }

                            ui.menu_button(RichText::new("＋").size(15.0).color(ACCENT), |ui| {
                                if ui.button("新建本地终端").clicked() {
                                    new_kind = Some(SessionKind::Local);
                                    ui.close_menu();
                                }
                                ui.separator();
                                ui.menu_button("WSL 发行版", |ui| {
                                    if app.wsl_distros.is_empty() {
                                        ui.label(RichText::new("尚未检测到发行版").color(DIM));
                                        if ui.button("检测 WSL…").clicked() {
                                            open_connect = true;
                                            ui.close_menu();
                                        }
                                    } else {
                                        for d in &app.wsl_distros {
                                            if ui.button(format!("WSL · {}", d.name)).clicked() {
                                                new_kind = Some(SessionKind::Wsl {
                                                    distro: Some(d.name.clone()),
                                                });
                                                ui.close_menu();
                                            }
                                        }
                                    }
                                });
                                ui.menu_button("SSH 服务器", |ui| {
                                    if app.cfg.ssh_profiles.is_empty() {
                                        ui.label(RichText::new("还没有保存的服务器").color(DIM));
                                    } else {
                                        for p in &app.cfg.ssh_profiles {
                                            if ui.button(p.display_name()).clicked() {
                                                new_kind =
                                                    Some(SessionKind::Ssh { profile: p.clone() });
                                                ui.close_menu();
                                            }
                                        }
                                        ui.separator();
                                    }
                                    if ui.button("新建 SSH 连接…").clicked() {
                                        new_ssh_form = true;
                                        ui.close_menu();
                                    }
                                });
                                ui.separator();
                                if ui.button("连接管理…").clicked() {
                                    open_connect = true;
                                    ui.close_menu();
                                }
                            });
                        });
                    });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("连接").size(11.5).color(ACCENT),
                            )
                            .frame(false),
                        )
                        .on_hover_text("打开连接管理器 (Ctrl+Shift+N)")
                        .clicked()
                    {
                        open_connect = true;
                    }
                });
            });
        });

    if let Some(i) = select {
        app.active = i;
        app.focus_terminal = true;
    }
    if let Some(i) = close {
        app.close_session(i);
    }
    if let Some(kind) = new_kind {
        app.open_session(kind);
    }
    if new_ssh_form {
        app.connect_open = true;
        app.connect_tab = crate::connect::ConnectTab::Ssh;
        app.start_wsl_detection();
        app.begin_new_ssh();
        app.focus_terminal = false;
    } else if open_connect {
        app.open_connect();
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(title)
            .size(10.8)
            .color(DIM)
            .strong(),
    );
    ui.add_space(2.0);
}

fn kv(ui: &mut egui::Ui, label: &str, value: impl Into<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(label).size(11.3).color(DIM));
        ui.label(RichText::new(value.into()).size(11.8).color(TEXT));
    });
}

fn usage_bar(ui: &mut egui::Ui, frac: f32, color: Color32, text: &str, height: f32) {
    let width = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(30, 43, 61));
    let w = rect.width() * frac.clamp(0.0, 1.0);
    if w > 0.0 {
        p.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(w, rect.height())),
            CornerRadius::same(4),
            color,
        );
    }
    p.text(
        rect.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(10.8),
        Color32::from_rgb(240, 246, 252),
    );
}

fn core_bars(ui: &mut egui::Ui, snap: &Snapshot) {
    if snap.cores.len() <= 1 {
        return;
    }
    ui.add_space(4.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        ui.horizontal_wrapped(|ui| {
            for (i, core) in snap.cores.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(13.0, 26.0), Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::same(2), Color32::from_rgb(28, 40, 57));
                let h = rect.height() * (core.usage / 100.0).clamp(0.0, 1.0);
                if h > 0.5 {
                    p.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(rect.left(), rect.bottom() - h),
                            rect.right_bottom(),
                        ),
                        CornerRadius::same(2),
                        usage_color(core.usage),
                    );
                }
                resp.on_hover_text(format!("核心 {}: {}", i, format::pct(core.usage)));
            }
        });
    });
}

pub fn details_panel(app: &mut WrenApp, ctx: &egui::Context) {
    let Some(idx) = app.active_index() else {
        return;
    };
    let session = &app.sessions[idx];
    let snap = session.stats.as_ref().map(|s| s.latest()).unwrap_or_default();
    let has_stats = session.stats.is_some();
    let kind = session.kind.clone();
    let mut open = app.details_open;

    egui::SidePanel::right("wren-details")
        .frame(
            egui::Frame::NONE
                .fill(PANEL)
                .inner_margin(Margin::symmetric(12, 8))
                .stroke(Stroke::new(1.0_f32, BORDER)),
        )
        .resizable(true)
        .default_width(342.0)
        .min_width(300.0)
        .max_width(480.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                status_dot(
                    ui,
                    if snap.error.is_some() {
                        RED
                    } else if snap.connected {
                        GREEN
                    } else {
                        AMBER
                    },
                );
                ui.label(RichText::new("系统详情").size(14.0).color(TEXT).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("×").size(15.0).color(DIM))
                                .frame(false),
                        )
                        .clicked()
                    {
                        open = false;
                    }
                });
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                badge(ui, kind.badge(), if kind.is_remote() { BLUE } else { ACCENT });
                ui.label(
                    RichText::new(format::shorten(&kind.target(), 40))
                        .size(11.0)
                        .color(DIM),
                );
            });

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if !has_stats {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new("该会话未启用远端状态轮询")
                                .size(13.0)
                                .color(AMBER),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "在 SSH 连接设置中开启轮询后，即可在这里查看远端系统的详细信息。",
                            )
                            .size(11.5)
                            .color(DIM),
                        );
                        return;
                    }

                    if let Some(err) = &snap.error {
                        ui.add_space(8.0);
                        egui::Frame::NONE
                            .fill(Color32::from_rgb(42, 24, 28))
                            .corner_radius(CornerRadius::same(7))
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("状态读取失败")
                                        .size(11.5)
                                        .color(RED)
                                        .strong(),
                                );
                                ui.label(
                                    RichText::new(err)
                                        .size(10.8)
                                        .color(Color32::from_rgb(214, 176, 176)),
                                );
                                if kind.is_remote() {
                                    ui.add_space(3.0);
                                    ui.label(
                                        RichText::new(
                                            "提示：SSH 状态轮询使用独立的免密连接，请配置密钥或 ssh-agent。",
                                        )
                                        .size(10.3)
                                        .color(DIM),
                                    );
                                }
                            });
                    }

                    section(ui, "连接");
                    kv(ui, "类型", kind.badge());
                    kv(ui, "目标", kind.target());
                    kv(
                        ui,
                        "状态",
                        if snap.error.is_some() {
                            "状态离线".to_string()
                        } else if snap.connected {
                            "已连接".to_string()
                        } else {
                            "正在连接".to_string()
                        },
                    );
                    if kind.is_remote() {
                        kv(ui, "时延", format::latency(snap.latency));
                        kv(ui, "数据更新", format::age(snap.updated));
                        if let Some(t) = snap.remote_time() {
                            kv(ui, "远端时间", t);
                        }
                        if let Some(offset) = snap.clock_offset {
                            kv(ui, "时钟偏差", format!("{offset:+} 秒"));
                        }
                    }
                    kv(ui, "本地时间", format::datetime());
                    if let Some(t) = session.term_title() {
                        kv(ui, "终端标题", t);
                    }

                    section(ui, "系统");
                    kv(ui, "主机名", if snap.host.is_empty() { "—" } else { &snap.host });
                    kv(ui, "操作系统", if snap.os.is_empty() { "—" } else { &snap.os });
                    kv(ui, "内核", if snap.kernel.is_empty() { "—" } else { &snap.kernel });
                    kv(ui, "架构", if snap.arch.is_empty() { "—" } else { &snap.arch });
                    if snap.uptime > 0 {
                        kv(ui, "运行时长", format::duration(snap.uptime));
                        kv(ui, "启动时间", &snap.boot_time);
                    }
                    if snap.processes > 0 {
                        kv(ui, "进程数", snap.processes.to_string());
                    }
                    if let Some(load) = snap.load {
                        kv(
                            ui,
                            "负载",
                            format!("{:.2} / {:.2} / {:.2}", load[0], load[1], load[2]),
                        );
                    }

                    section(ui, "CPU");
                    if !snap.cpu_model.is_empty() {
                        kv(ui, "型号", &snap.cpu_model);
                    }
                    let cpu = snap.cpu_usage.clamp(0.0, 100.0);
                    usage_bar(
                        ui,
                        cpu / 100.0,
                        usage_color(cpu),
                        &format!("{} · {} 核", format::pct(cpu), snap.cores.len()),
                        18.0,
                    );
                    core_bars(ui, &snap);

                    section(ui, "内存");
                    let mem = snap.mem_pct();
                    usage_bar(
                        ui,
                        mem / 100.0,
                        usage_color(mem),
                        &format!(
                            "{} / {} · {}",
                            format::bytes(snap.mem_used),
                            format::bytes(snap.mem_total),
                            format::pct(mem)
                        ),
                        18.0,
                    );
                    kv(ui, "可用", format::bytes(snap.mem_available));
                    if snap.swap_total > 0 {
                        let swap = if snap.swap_total == 0 {
                            0.0
                        } else {
                            snap.swap_used as f32 / snap.swap_total as f32 * 100.0
                        };
                        kv(
                            ui,
                            "交换分区",
                            format!(
                                "{} / {} · {}",
                                format::bytes(snap.swap_used),
                                format::bytes(snap.swap_total),
                                format::pct(swap)
                            ),
                        );
                    }

                    section(ui, "磁盘");
                    if snap.disks.is_empty() {
                        ui.label(RichText::new("—").size(11.5).color(DIM));
                    }
                    for d in snap.disks.iter().take(8) {
                        let pct = if d.total == 0 {
                            0.0
                        } else {
                            d.used as f32 / d.total as f32 * 100.0
                        };
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            let mount = ui.label(
                                RichText::new(format::shorten(&d.mount, 26))
                                    .size(11.2)
                                    .color(TEXT),
                            );
                            mount.on_hover_text(format!(
                                "{}\n设备：{}\n可用：{}",
                                d.mount,
                                if d.name.is_empty() { "—" } else { &d.name },
                                format::bytes(d.available)
                            ));
                            if !d.file_system.is_empty() {
                                ui.label(
                                    RichText::new(&d.file_system).size(9.8).color(DIM),
                                );
                            }
                            if d.removable {
                                ui.label(RichText::new("可移动").size(9.3).color(AMBER));
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format::pct(pct))
                                        .size(10.5)
                                        .color(usage_color(pct)),
                                );
                            });
                        });
                        usage_bar(
                            ui,
                            pct / 100.0,
                            usage_color(pct),
                            &format!(
                                "{} / {}",
                                format::bytes(d.used),
                                format::bytes(d.total)
                            ),
                            12.0,
                        );
                    }

                    section(ui, "网络");
                    kv(
                        ui,
                        "当前速率",
                        format!(
                            "↓ {}   ↑ {}",
                            format::rate(snap.net_rx_rate),
                            format::rate(snap.net_tx_rate)
                        ),
                    );
                    kv(
                        ui,
                        "累计流量",
                        format!(
                            "↓ {}   ↑ {}",
                            format::bytes(snap.net_rx_total),
                            format::bytes(snap.net_tx_total)
                        ),
                    );
                    for iface in snap.ifaces.iter().take(8) {
                        let mut addrs = if iface.addrs.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", iface.addrs.join(", "))
                        };
                        if iface.rx_rate > 0.0 || iface.tx_rate > 0.0 {
                            addrs.push_str(&format!(
                                " · ↓ {} ↑ {}",
                                format::rate(iface.rx_rate),
                                format::rate(iface.tx_rate)
                            ));
                        }
                        kv(ui, &iface.name, addrs);
                    }
                    if !kind.is_remote() && snap.net_rx_rate == 0.0 && snap.net_tx_rate == 0.0 {
                        ui.label(
                            RichText::new("网络速率在第一次采样后显示")
                                .size(10.3)
                                .color(DIM),
                        );
                    }

                    section(ui, "进程");
                    if snap.top_procs.is_empty() {
                        ui.label(RichText::new("—").size(11.5).color(DIM));
                    }
                    for p in snap.top_procs.iter().take(8) {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format::shorten(&p.name, 24))
                                    .size(11.0)
                                    .color(TEXT),
                            );
                            ui.label(
                                RichText::new(format!("PID {}", p.pid))
                                    .size(9.8)
                                    .color(DIM),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "CPU {:.0}% · 内存 {:.1}%",
                                        p.cpu, p.mem_pct
                                    ))
                                    .size(9.8)
                                    .color(DIM),
                                );
                            });
                        });
                    }

                    ui.add_space(14.0);
                    ui.separator();
                    ui.label(
                        RichText::new("本地 1.5s · WSL 4s · SSH 8s 自动刷新")
                            .size(9.8)
                            .color(DIM),
                    );
                    ui.add_space(6.0);
                });
        });

    if !open {
        app.details_open = false;
        app.cfg.details_open = false;
        app.cfg.save();
    }
}

pub fn terminal_area(app: &mut WrenApp, ui: &mut egui::Ui) {
    let Some(idx) = app.active_index() else {
        welcome(app, ui);
        return;
    };

    if app.sessions[idx].backend.is_none() {
        failed_session(app, ui, idx);
        return;
    }

    let state = app.sessions[idx].state;
    let backend_id = app.sessions[idx].backend_id;
    let focus = app.focus_terminal;
    let size = app.cfg.font_size;
    let theme = app.term_theme.clone();

    let mut clicked_focus = false;
    let mut restart = false;
    let mut close_tab = false;

    {
        let session = &mut app.sessions[idx];
        // 上面虽然已经检查过；这里再用 let-else 兜底，避免状态被并发修改时 panic。
        let Some(backend) = session.backend.as_mut() else {
            return;
        };
        egui::Frame::NONE
            .fill(Color32::from_rgb(10, 16, 26))
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                let font = TerminalFont::new(FontSettings {
                    font_type: FontId::monospace(size),
                });
                ui.set_min_width(ui.available_width());
                // 右侧固定留出 12px 给滚动条，避免拖动滚动条时误触发终端选区。
                let available = ui.available_size();
                let term_width = (available.x - 12.0).max(120.0);
                let resp = TerminalView::new(ui, backend)
                    .set_size(Vec2::new(term_width, available.y))
                    .set_font(font)
                    .set_theme(theme)
                    .set_focus(focus)
                    .ui(ui);
                if resp.clicked() {
                    clicked_focus = true;
                }

                let (offset, history) = backend.scroll_state();
                if history > 0 {
                    let screen_lines = backend.size().lines() as usize;
                    let lane = Rect::from_min_max(
                        Pos2::new(resp.rect.right() + 2.0, resp.rect.top()),
                        Pos2::new(resp.rect.right() + 12.0, resp.rect.bottom()),
                    );
                    if lane.width() >= 4.0 && lane.height() > 10.0 {
                        let bar_resp = ui.interact(
                            lane,
                            egui::Id::new(("wren-term-scroll", backend_id)),
                            Sense::click_and_drag(),
                        );
                        let p = ui.painter();
                        p.rect_filled(lane, CornerRadius::same(4), Color32::from_rgb(18, 27, 42));
                        let total = (history + screen_lines).max(1) as f32;
                        let thumb_h = (lane.height() * screen_lines as f32 / total)
                            .clamp(28.0, lane.height());
                        let max_offset = history as f32;
                        let frac = if max_offset > 0.0 {
                            (offset as f32 / max_offset).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let top =
                            lane.bottom() - thumb_h - (lane.height() - thumb_h) * frac;
                        let thumb = Rect::from_min_size(
                            Pos2::new(lane.left() + 1.0, top),
                            Vec2::new(lane.width() - 2.0, thumb_h),
                        );
                        let hovering = bar_resp.hovered() || bar_resp.dragged();
                        p.rect_filled(
                            thumb,
                            CornerRadius::same(3),
                            if hovering {
                                Color32::from_rgb(184, 202, 224)
                            } else {
                                Color32::from_rgb(130, 152, 178)
                            },
                        );
                        if bar_resp.dragged() || bar_resp.clicked() {
                            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                                let track = (lane.height() - thumb_h).max(1.0);
                                let t = ((pos.y - lane.top() - thumb_h / 2.0) / track)
                                    .clamp(0.0, 1.0);
                                let desired = ((1.0 - t) * max_offset).round() as usize;
                                let delta = desired as i32 - offset as i32;
                                if delta != 0 {
                                    backend.process_command(BackendCommand::Scroll(delta));
                                }
                            }
                        }
                        bar_resp.on_hover_text(format!(
                            "拖动查看历史输出 · 已回滚 {} 行",
                            offset
                        ));
                    }
                }
            });
    }

    if state != SessionState::Running {
        egui::Area::new(egui::Id::new(("wren-exit", backend_id)))
            .anchor(Align2::CENTER_BOTTOM, [0.0, -20.0])
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgb(20, 30, 45))
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 9))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (text, color) = match state {
                                SessionState::Exited(0) => {
                                    ("会话已结束".to_string(), DIM)
                                }
                                SessionState::Exited(code) => {
                                    (format!("会话已结束（退出码 {code}）"), AMBER)
                                }
                                _ => ("会话未运行".to_string(), RED),
                            };
                            ui.label(RichText::new(text).size(11.5).color(color));
                            if ui.button("重新连接").clicked() {
                                restart = true;
                            }
                            if ui.button("关闭标签").clicked() {
                                close_tab = true;
                            }
                        });
                    });
            });
    }

    if clicked_focus {
        app.focus_terminal = true;
    }
    if restart {
        app.restart_session(idx);
    } else if close_tab {
        app.close_session(idx);
    }
}

fn failed_session(app: &mut WrenApp, ui: &mut egui::Ui, idx: usize) {
    let kind = app.sessions[idx].kind.clone();
    let err = app
        .sessions[idx]
        .spawn_error
        .clone()
        .unwrap_or_else(|| "未知错误".into());
    let mut restart = false;
    let mut close_tab = false;
    let mut open_connect = false;

    ui.vertical_centered(|ui| {
        ui.add_space(70.0);
        ui.label(RichText::new("无法启动会话").size(20.0).color(RED).strong());
        ui.add_space(6.0);
        ui.label(
            RichText::new(kind.target())
                .size(12.5)
                .color(DIM),
        );
        ui.add_space(14.0);
        egui::Frame::NONE
            .fill(Color32::from_rgb(38, 22, 26))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(&err)
                        .size(11.5)
                        .color(Color32::from_rgb(230, 190, 190)),
                );
                if matches!(kind, SessionKind::Wsl { .. }) {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("提示：确认 WSL 已安装，并在终端执行 wsl --list --verbose 检查发行版。")
                            .size(10.8)
                            .color(DIM),
                    );
                }
                if matches!(kind, SessionKind::Ssh { .. }) {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("提示：确认 ssh.exe 可用、主机地址与端口正确，并已接受主机指纹。")
                            .size(10.8)
                            .color(DIM),
                    );
                }
            });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("重试").clicked() {
                restart = true;
            }
            if ui.button("连接管理").clicked() {
                open_connect = true;
            }
            if ui.button("关闭标签").clicked() {
                close_tab = true;
            }
        });
    });

    if restart {
        app.restart_session(idx);
    } else if close_tab {
        app.close_session(idx);
    } else if open_connect {
        app.open_connect();
    }
}

fn welcome(app: &mut WrenApp, ui: &mut egui::Ui) {
    let mut local = false;
    let mut connect = false;
    ui.vertical_centered(|ui| {
        ui.add_space(90.0);
        ui.label(RichText::new("◆ WREN").size(34.0).color(ACCENT).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new("Windows 上的 Linux 风格终端 · 状态栏常驻 · 本地 / WSL / SSH")
                .size(12.5)
                .color(DIM),
        );
        ui.add_space(28.0);
        ui.horizontal(|ui| {
            if ui
                .add_sized(
                    [120.0, 32.0],
                    egui::Button::new(RichText::new("新建本地终端").size(13.0)),
                )
                .clicked()
            {
                local = true;
            }
            if ui
                .add_sized(
                    [120.0, 32.0],
                    egui::Button::new(RichText::new("连接 WSL / SSH").size(13.0)),
                )
                .clicked()
            {
                connect = true;
            }
        });
        ui.add_space(26.0);
        ui.label(
            RichText::new("Ctrl+Shift+T 新建本地  ·  Ctrl+Shift+N 连接  ·  Ctrl+Tab 切换标签  ·  F12 系统详情")
                .size(10.5)
                .color(DIM),
        );
    });
    if local {
        app.open_session(SessionKind::Local);
    } else if connect {
        app.open_connect();
    }
}
