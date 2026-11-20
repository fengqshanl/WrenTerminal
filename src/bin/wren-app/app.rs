//! 应用状态、标签页生命周期与主循环。

use crate::backend::{BackendCommand, PtyEvent};
use crate::config::{self, AppConfig, SshProfile};
use crate::connect::{ConnectTab, SshForm, WslState};
use crate::session::{Session, SessionKind, SessionState};
use crate::stats::WslDistro;
use crate::theme::TerminalTheme;
use eframe::egui::{self, Key, Modifiers};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct WrenApp {
    pub ctx: egui::Context,
    pub cfg: AppConfig,
    pub sessions: Vec<Session>,
    pub active: usize,
    pub next_backend_id: u64,
    tx: mpsc::Sender<(u64, PtyEvent)>,
    rx: mpsc::Receiver<(u64, PtyEvent)>,

    pub focus_terminal: bool,
    pub details_open: bool,
    pub term_theme: TerminalTheme,

    pub connect_open: bool,
    pub connect_tab: ConnectTab,
    pub wsl_state: WslState,
    pub wsl_distros: Vec<WslDistro>,
    pub wsl_rx: Option<mpsc::Receiver<Result<Vec<WslDistro>, String>>>,
    pub ssh_form: SshForm,
    pub ssh_form_open: bool,
    pub editing_profile: Option<u64>,

    pub toast: Option<(String, Instant)>,
    /// 上一帧 UI 代码发生 panic 时的错误摘要，用于在顶部显示恢复横幅。
    pub crash_notice: Option<String>,
    last_window_size: [f32; 2],
    last_save: Instant,
    top_applied: bool,
}

impl WrenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: AppConfig) -> Self {
        crate::ui::apply_style(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let initial_size = cfg.window_size.unwrap_or([1280.0, 820.0]);
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            cfg,
            sessions: Vec::new(),
            active: 0,
            next_backend_id: 1,
            tx,
            rx,
            focus_terminal: true,
            details_open: false,
            term_theme: crate::ui::terminal_theme(),
            connect_open: false,
            connect_tab: ConnectTab::Wsl,
            wsl_state: WslState::Idle,
            wsl_distros: Vec::new(),
            wsl_rx: None,
            ssh_form: SshForm::default(),
            ssh_form_open: false,
            editing_profile: None,
            toast: None,
            crash_notice: None,
            last_window_size: initial_size,
            last_save: Instant::now(),
            top_applied: false,
        };
        app.details_open = app.cfg.details_open;
        app.open_session(SessionKind::Local);
        app
    }

    pub fn active_index(&self) -> Option<usize> {
        if self.sessions.is_empty() {
            None
        } else {
            Some(self.active.min(self.sessions.len() - 1))
        }
    }

    pub fn active_session(&self) -> Option<&Session> {
        self.active_index().map(|i| &self.sessions[i])
    }

    pub fn open_session(&mut self, kind: SessionKind) {
        if let SessionKind::Wsl { distro } = &kind {
            self.cfg.last_wsl_distro = distro.clone();
        }
        let id = self.next_backend_id;
        self.next_backend_id = self.next_backend_id.saturating_add(1);
        let session = Session::spawn(id, kind, &self.ctx, self.tx.clone());
        self.sessions.push(session);
        self.active = self.sessions.len() - 1;
        self.connect_open = false;
        self.ssh_form_open = false;
        self.focus_terminal = true;
        self.persist_now();
    }

    pub fn close_session(&mut self, idx: usize) {
        if idx >= self.sessions.len() {
            return;
        }
        self.sessions.remove(idx);
        if self.sessions.is_empty() {
            self.active = 0;
            self.focus_terminal = false;
        } else if idx < self.active {
            self.active -= 1;
        } else if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        }
    }

    pub fn restart_session(&mut self, idx: usize) {
        let Some(old) = self.sessions.get(idx) else {
            return;
        };
        let kind = old.kind.clone();
        let id = self.next_backend_id;
        self.next_backend_id = self.next_backend_id.saturating_add(1);
        let session = Session::spawn(id, kind, &self.ctx, self.tx.clone());
        let failed = session.state == SessionState::Failed;
        self.sessions[idx] = session;
        self.active = idx;
        self.focus_terminal = true;
        self.toast(if failed {
            "重新启动失败".into()
        } else {
            "会话已重新启动".into()
        });
    }

    pub fn cycle_tab(&mut self, delta: isize) {
        if self.sessions.len() < 2 {
            return;
        }
        let n = self.sessions.len() as isize;
        let cur = self.active as isize;
        self.active = (((cur + delta) % n + n) % n) as usize;
        self.focus_terminal = true;
    }

    pub fn toast(&mut self, msg: String) {
        self.toast = Some((msg, Instant::now()));
    }

    pub fn persist_now(&mut self) {
        self.cfg.details_open = self.details_open;
        if self.last_window_size[0] > 100.0 && self.last_window_size[1] > 100.0 {
            self.cfg.window_size = Some(self.last_window_size);
        }
        self.cfg.save();
    }

    fn save_if_needed(&mut self) {
        if self.last_save.elapsed() < Duration::from_secs(10) {
            return;
        }
        self.last_save = Instant::now();
        let size_changed = self
            .cfg
            .window_size
            .map(|w| {
                (w[0] - self.last_window_size[0]).abs() > 1.0
                    || (w[1] - self.last_window_size[1]).abs() > 1.0
            })
            .unwrap_or(true);
        if size_changed || self.cfg.details_open != self.details_open {
            self.persist_now();
        }
    }

    pub fn adjust_font(&mut self, delta: f32) {
        self.cfg.font_size = (self.cfg.font_size + delta).clamp(10.0, 28.0);
        self.cfg.save();
    }

    /// 切换窗口置顶。只有用户主动点击“置顶”按钮时才会调用。
    pub fn set_always_on_top(&mut self, on: bool, ctx: &egui::Context) {
        self.cfg.always_on_top = on;
        self.cfg.save();
        self.top_applied = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if on {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        }));
        self.toast(if on {
            "窗口已置顶".into()
        } else {
            "已取消置顶".into()
        });
    }

    // ===================== 连接对话框 =====================

    pub fn open_connect(&mut self) {
        self.connect_open = true;
        self.ssh_form_open = false;
        self.focus_terminal = false;
        if matches!(self.wsl_state, WslState::Idle) {
            self.start_wsl_detection();
        }
    }

    pub fn close_connect(&mut self) {
        self.connect_open = false;
        self.ssh_form_open = false;
        self.editing_profile = None;
        self.focus_terminal = true;
    }

    pub fn start_wsl_detection(&mut self) {
        if matches!(self.wsl_state, WslState::Loading) {
            return;
        }
        self.wsl_state = WslState::Loading;
        self.wsl_rx = Some(crate::stats::detect_wsl_async());
    }

    fn poll_wsl(&mut self) {
        let Some(rx) = &self.wsl_rx else { return };
        match rx.try_recv() {
            Ok(Ok(distros)) => {
                self.wsl_distros = distros;
                self.wsl_state = WslState::Ready;
                self.wsl_rx = None;
            }
            Ok(Err(err)) => {
                self.wsl_state = WslState::Error(err);
                self.wsl_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.wsl_state = WslState::Error("WSL 检测线程异常结束".into());
                self.wsl_rx = None;
            }
        }
    }

    pub fn begin_new_ssh(&mut self) {
        self.ssh_form = SshForm::default();
        self.editing_profile = None;
        self.ssh_form_open = true;
    }

    pub fn begin_edit_ssh(&mut self, profile: &SshProfile) {
        self.ssh_form = SshForm::from_profile(profile);
        self.editing_profile = Some(profile.id);
        self.ssh_form_open = true;
    }

    /// 返回 true 表示表单已保存（并可能已发起连接）。
    pub fn save_ssh_form(&mut self, connect_now: bool) -> bool {
        let profile = match self.ssh_form.to_profile(self.editing_profile.unwrap_or(0)) {
            Ok(p) => p,
            Err(e) => {
                self.toast(e);
                return false;
            }
        };
        self.cfg.upsert_profile(profile.clone());
        self.cfg.save();
        self.ssh_form_open = false;
        self.editing_profile = None;
        if connect_now {
            self.open_session(SessionKind::Ssh { profile });
        }
        true
    }

    pub fn delete_ssh_profile(&mut self, id: u64) {
        self.cfg.remove_profile(id);
        self.cfg.save();
        self.toast("已删除服务器配置".into());
    }

    // ===================== 事件 =====================

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok((backend_id, event)) = self.rx.try_recv() {
            let Some(idx) = self
                .sessions
                .iter()
                .position(|s| s.backend_id == backend_id)
            else {
                continue;
            };
            match event {
                PtyEvent::PtyWrite(text) => {
                    if let Some(backend) = self.sessions[idx].backend.as_mut() {
                        backend.process_command(BackendCommand::Write(text.into_bytes()));
                    }
                }
                PtyEvent::ClipboardStore(_, text) => {
                    ctx.copy_text(text);
                }
                PtyEvent::TextAreaSizeRequest(f) => {
                    if let Some(backend) = self.sessions[idx].backend.as_ref() {
                        let size = backend.size();
                        let reply = f(size.into());
                        if let Some(backend) = self.sessions[idx].backend.as_mut() {
                            backend.process_command(BackendCommand::Write(reply.into_bytes()));
                        }
                    }
                }
                PtyEvent::ChildExit(code) => {
                    self.sessions[idx].state = SessionState::Exited(code);
                    self.sessions[idx].term_title = None;
                }
                PtyEvent::Exit => {
                    if self.sessions[idx].state == SessionState::Running {
                        self.sessions[idx].state = SessionState::Exited(0);
                    }
                }
                PtyEvent::Title(title) => {
                    // ConPTY 的默认标题是子进程的完整路径（如 …\wren.exe），
                    // 这种标题没有信息量，忽略；只保留 shell/程序设置的真实标题。
                    let t = title.trim();
                    let noisy = t.is_empty()
                        || t.contains('\\')
                        || t.contains('/')
                        || t.to_ascii_lowercase().ends_with(".exe");
                    self.sessions[idx].term_title = if noisy {
                        None
                    } else {
                        Some(title)
                    };
                }
                PtyEvent::ResetTitle => {
                    self.sessions[idx].term_title = None;
                }
                _ => {}
            }
        }
    }

    // ===================== 快捷键 =====================

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.connect_open {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.close_connect();
            }
            return;
        }

        let mut new_local = false;
        let mut new_connect = false;
        let mut close = false;
        let mut next: Option<isize> = None;
        let mut toggle_details = false;
        let mut activate: Option<usize> = None;
        let mut font_delta = 0.0f32;

        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::T) {
                new_local = true;
            }
            if i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::N) {
                new_connect = true;
            }
            if i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::W) {
                close = true;
            }
            if i.consume_key(Modifiers::CTRL, Key::Tab) {
                next = Some(1);
            }
            if i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
                next = Some(-1);
            }
            if i.consume_key(Modifiers::NONE, Key::F12) {
                toggle_details = true;
            }
            if i.consume_key(Modifiers::CTRL, Key::Equals)
                || i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Plus)
            {
                font_delta = 1.0;
            }
            if i.consume_key(Modifiers::CTRL, Key::Minus) {
                font_delta = -1.0;
            }
            if i.consume_key(Modifiers::CTRL, Key::Num0) {
                font_delta = 14.0 - self.cfg.font_size;
            }
            for n in 1..=9usize {
                let key = match n {
                    1 => Key::Num1,
                    2 => Key::Num2,
                    3 => Key::Num3,
                    4 => Key::Num4,
                    5 => Key::Num5,
                    6 => Key::Num6,
                    7 => Key::Num7,
                    8 => Key::Num8,
                    _ => Key::Num9,
                };
                if i.consume_key(Modifiers::ALT, key) {
                    activate = Some(n - 1);
                }
            }
        });

        if new_local {
            self.open_session(SessionKind::Local);
        }
        if new_connect {
            self.open_connect();
        }
        if close {
            if let Some(idx) = self.active_index() {
                self.close_session(idx);
            }
        }
        if let Some(delta) = next {
            self.cycle_tab(delta);
        }
        if toggle_details {
            self.details_open = !self.details_open;
            self.cfg.details_open = self.details_open;
            self.cfg.save();
        }
        if let Some(idx) = activate {
            if idx < self.sessions.len() {
                self.active = idx;
                self.focus_terminal = true;
            }
        }
        if font_delta.abs() > f32::EPSILON {
            self.adjust_font(font_delta);
        }
    }
}

impl eframe::App for WrenApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // 界面代码里的任何 panic 都被限制在这一帧内：记录日志、显示提示，
        // 应用继续运行，用户不会因为一次绘制异常而丢失所有终端会话。
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.update_frame(ctx, frame);
        }));
        if let Err(payload) = outcome {
            let msg = panic_text(payload.as_ref());
            config::log(&format!("UI 帧发生 panic（已恢复运行）: {msg}"));
            self.crash_notice = Some(msg);
            ctx.request_repaint();
        }
    }
}

/// 从 panic payload 中提取可读文本。
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "未知 panic".to_string())
}

impl WrenApp {
    fn update_frame(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 上一帧发生过 panic 时，顶部显示一条可关闭的错误横幅。
        if self.crash_notice.is_some() {
            let mut dismiss = false;
            egui::TopBottomPanel::top("wren-crash-banner")
                .frame(
                    egui::Frame::NONE
                        .fill(egui::Color32::from_rgb(122, 32, 32))
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("界面发生错误，已自动恢复运行")
                                .color(egui::Color32::WHITE)
                                .strong(),
                        );
                        if let Some(msg) = &self.crash_notice {
                            ui.label(
                                egui::RichText::new(msg)
                                    .color(egui::Color32::from_rgb(255, 220, 220))
                                    .size(12.0),
                            );
                        }
                        if ui.button("关闭提示").clicked() {
                            dismiss = true;
                        }
                    });
                });
            if dismiss {
                self.crash_notice = None;
            }
        }

        self.handle_events(ctx);
        self.poll_wsl();
        self.handle_shortcuts(ctx);

        self.focus_terminal = !self.connect_open;
        let screen = ctx.screen_rect().size();
        self.last_window_size = [screen.x, screen.y];
        self.save_if_needed();

        // 首次启动时应用保存的置顶设置（默认关闭）。
        if !self.top_applied {
            self.top_applied = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                if self.cfg.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                },
            ));
        }

        let title = self
            .active_session()
            .map(|s| s.title())
            .unwrap_or_else(|| "Wren".into());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("Wren — {title}")));

        crate::ui::status_bar(self, ctx);
        crate::ui::tab_bar(self, ctx);
        if self.details_open {
            crate::ui::details_panel(self, ctx);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(crate::ui::BG))
            .show(ctx, |ui| {
                crate::ui::terminal_area(self, ui);
            });

        if self.connect_open {
            crate::connect::connect_window(self, ctx);
        }

        if let Some((msg, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(3) {
                egui::Area::new(egui::Id::new("wren-toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -24.0])
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        egui::Frame::NONE
                            .fill(crate::ui::TOAST_BG)
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(14, 8))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(msg)
                                        .size(13.0)
                                        .color(egui::Color32::from_rgb(230, 238, 248)),
                                );
                            });
                    });
                ctx.request_repaint_after(Duration::from_millis(200));
            } else {
                self.toast = None;
            }
        }

        ctx.request_repaint_after(Duration::from_millis(500));
    }
}


/// 供连接窗口在保存前展示表单校验错误。
pub fn validate_port(text: &str) -> Result<u16, String> {
    let port: u16 = text
        .trim()
        .parse()
        .map_err(|_| "端口必须是 1-65535 的数字".to_string())?;
    if port == 0 {
        return Err("端口必须在 1-65535 之间".into());
    }
    Ok(port)
}

/// 让日志中出现的 SSH 配置不包含敏感信息（当前没有密码字段，仅保留占位函数）。
pub fn log_connecting(profile: &SshProfile) {
    config::log(&format!("连接 SSH: {}", profile.target()));
}
