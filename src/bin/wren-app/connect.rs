//! 新建连接窗口：本地 / WSL / SSH。

use crate::app::{validate_port, WrenApp};
use crate::config::SshProfile;
use crate::session::SessionKind;
use egui::{Align2, Color32, RichText};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectTab {
    Local,
    Wsl,
    Ssh,
}

#[derive(Debug, Clone)]
pub enum WslState {
    Idle,
    Loading,
    Ready,
    Error(String),
}

impl Default for WslState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone)]
pub struct SshForm {
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: String,
    pub identity_file: String,
    pub poll_status: bool,
}

impl Default for SshForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            user: String::new(),
            port: "22".into(),
            identity_file: String::new(),
            poll_status: true,
        }
    }
}

impl SshForm {
    pub fn from_profile(p: &SshProfile) -> Self {
        Self {
            name: p.name.clone(),
            host: p.host.clone(),
            user: p.user.clone(),
            port: p.port.to_string(),
            identity_file: p.identity_file.clone(),
            poll_status: p.poll_status,
        }
    }

    pub fn to_profile(&self, id: u64) -> Result<SshProfile, String> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err("请填写主机地址".into());
        }
        let port = if self.port.trim().is_empty() {
            22
        } else {
            validate_port(&self.port)?
        };
        Ok(SshProfile {
            id,
            name: self.name.trim().to_string(),
            host: host.to_string(),
            user: self.user.trim().to_string(),
            port,
            identity_file: self.identity_file.trim().to_string(),
            poll_status: self.poll_status,
        })
    }
}

enum SshAction {
    Connect(usize),
    Edit(usize),
    Delete(u64),
}

pub fn connect_window(app: &mut WrenApp, ctx: &egui::Context) {
    let mut open = app.connect_open;
    egui::Window::new("连接")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(660.0)
        .min_width(660.0)
        .max_width(760.0)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (ConnectTab::Wsl, "WSL 发行版"),
                    (ConnectTab::Ssh, "SSH 服务器"),
                    (ConnectTab::Local, "本地终端"),
                ] {
                    if ui
                        .selectable_label(app.connect_tab == tab, RichText::new(label).size(14.0))
                        .clicked()
                    {
                        app.connect_tab = tab;
                        app.ssh_form_open = false;
                    }
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(8.0);

            if app.connect_tab == ConnectTab::Wsl && matches!(app.wsl_state, WslState::Idle) {
                app.start_wsl_detection();
            }

            match app.connect_tab {
                ConnectTab::Local => local_tab(app, ui),
                ConnectTab::Wsl => wsl_tab(app, ui),
                ConnectTab::Ssh => ssh_tab(app, ui),
            }
        });
    if !open {
        app.close_connect();
    }
}

fn local_tab(app: &mut WrenApp, ui: &mut egui::Ui) {
    ui.label(
        RichText::new("打开一个本地 Wren 终端")
            .size(15.0)
            .color(Color32::from_rgb(220, 230, 242)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new("使用 Wren 的 Linux 风格命令，支持管道、重定向、apt、sudo 等。")
            .size(12.5)
            .color(Color32::from_rgb(140, 156, 176)),
    );
    ui.add_space(16.0);
    if ui
        .add_sized(
            [150.0, 30.0],
            egui::Button::new(RichText::new("打开本地终端").size(13.5)),
        )
        .clicked()
    {
        app.open_session(SessionKind::Local);
    }
}

fn wsl_tab(app: &mut WrenApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("WSL 发行版")
                .size(15.0)
                .color(Color32::from_rgb(220, 230, 242)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("重新检测").clicked() {
                app.start_wsl_detection();
            }
        });
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new("连接后，顶部状态栏会自动显示该发行版内部的 CPU、内存、磁盘与网络状态。")
            .size(12.0)
            .color(Color32::from_rgb(140, 156, 176)),
    );
    ui.add_space(10.0);

    let wsl_state = app.wsl_state.clone();
    match &wsl_state {
        WslState::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在检测 WSL 发行版…");
            });
        }
        WslState::Error(err) => {
            egui::Frame::NONE
                .fill(Color32::from_rgb(40, 22, 26))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("无法读取 WSL 发行版")
                            .color(crate::ui::RED)
                            .strong(),
                    );
                    ui.label(RichText::new(err).size(12.0).color(Color32::from_rgb(200, 170, 170)));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("重试").clicked() {
                            app.start_wsl_detection();
                        }
                        if ui.button("打开本地终端").clicked() {
                            app.open_session(SessionKind::Local);
                        }
                    });
                });
        }
        WslState::Ready => {
            let mut connect: Option<Option<String>> = None;
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let has_default = app.wsl_distros.iter().any(|d| d.is_default);
                    egui::Frame::NONE
                        .fill(crate::ui::CARD)
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("默认发行版").size(13.5).strong());
                                if has_default {
                                    ui.label(
                                        RichText::new("wsl.exe 默认").size(11.0).color(crate::ui::DIM),
                                    );
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("连接").clicked() {
                                            connect = Some(None);
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(6.0);

                    if app.wsl_distros.is_empty() {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("没有检测到已安装的 WSL 发行版。可在终端执行 wsl --install 安装。")
                                .size(12.5)
                                .color(crate::ui::DIM),
                        );
                    }

                    for (i, d) in app.wsl_distros.iter().enumerate() {
                        egui::Frame::NONE
                            .fill(if i % 2 == 0 {
                                crate::ui::CARD
                            } else {
                                crate::ui::CARD_ALT
                            })
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(12, 9))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&d.name).size(13.5).strong());
                                    if d.is_default {
                                        ui.label(
                                            RichText::new("默认")
                                                .size(10.5)
                                                .color(crate::ui::ACCENT),
                                        );
                                    }
                                    let running = d.state.contains("Run") || d.state.contains("运行");
                                    ui.label(
                                        RichText::new(if running { "运行中" } else { "已停止" })
                                            .size(10.5)
                                            .color(if running {
                                                crate::ui::GREEN
                                            } else {
                                                crate::ui::DIM
                                            }),
                                    );
                                    ui.label(
                                        RichText::new(format!("WSL {}", d.version))
                                            .size(10.5)
                                            .color(crate::ui::DIM),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.button("连接").clicked() {
                                                connect = Some(Some(d.name.clone()));
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(4.0);
                    }
                });
            if let Some(distro) = connect {
                app.open_session(SessionKind::Wsl { distro });
            }
        }
        WslState::Idle => {}
    }
}

fn ssh_tab(app: &mut WrenApp, ui: &mut egui::Ui) {
    if app.ssh_form_open {
        ssh_form(app, ui);
        return;
    }

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("已保存的服务器")
                .size(15.0)
                .color(Color32::from_rgb(220, 230, 242)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("＋ 新建 SSH 连接").clicked() {
                app.begin_new_ssh();
            }
        });
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new("支持任意标准 SSH 服务器；保存后可一键连接。状态栏需要免密登录（密钥或 ssh-agent）才能读取远端数据。")
            .size(12.0)
            .color(Color32::from_rgb(140, 156, 176)),
    );
    ui.add_space(10.0);

    if app.cfg.ssh_profiles.is_empty() {
        egui::Frame::NONE
            .fill(crate::ui::CARD)
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(18))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("还没有保存的服务器")
                            .size(14.0)
                            .color(Color32::from_rgb(200, 212, 226)),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("添加主机、用户名和端口，即可像本地终端一样使用。")
                            .size(12.0)
                            .color(crate::ui::DIM),
                    );
                    ui.add_space(10.0);
                    if ui.button("新建第一个 SSH 连接").clicked() {
                        app.begin_new_ssh();
                    }
                });
            });
        return;
    }

    let mut action: Option<SshAction> = None;
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (i, p) in app.cfg.ssh_profiles.iter().enumerate() {
                egui::Frame::NONE
                    .fill(if i % 2 == 0 {
                        crate::ui::CARD
                    } else {
                        crate::ui::CARD_ALT
                    })
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(p.display_name()).size(13.5).strong());
                                let mut meta = p.target();
                                if p.port != 0 && p.port != 22 {
                                    meta.push_str(&format!(":{}", p.port));
                                }
                                if !p.poll_status {
                                    meta.push_str(" · 状态轮询关闭");
                                }
                                ui.label(RichText::new(meta).size(11.0).color(crate::ui::DIM));
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("连接").clicked() {
                                        action = Some(SshAction::Connect(i));
                                    }
                                    if ui.button("编辑").clicked() {
                                        action = Some(SshAction::Edit(i));
                                    }
                                    if ui
                                        .button(RichText::new("删除").color(crate::ui::RED))
                                        .clicked()
                                    {
                                        action = Some(SshAction::Delete(p.id));
                                    }
                                },
                            );
                        });
                    });
                ui.add_space(4.0);
            }
        });

    match action {
        Some(SshAction::Connect(i)) => {
            if let Some(p) = app.cfg.ssh_profiles.get(i).cloned() {
                crate::app::log_connecting(&p);
                app.open_session(SessionKind::Ssh { profile: p });
            }
        }
        Some(SshAction::Edit(i)) => {
            if let Some(p) = app.cfg.ssh_profiles.get(i).cloned() {
                app.begin_edit_ssh(&p);
            }
        }
        Some(SshAction::Delete(id)) => app.delete_ssh_profile(id),
        None => {}
    }
}

fn ssh_form(app: &mut WrenApp, ui: &mut egui::Ui) {
    let editing = app.editing_profile.is_some();
    ui.label(
        RichText::new(if editing {
            "编辑 SSH 连接"
        } else {
            "新建 SSH 连接"
        })
        .size(15.0)
        .color(Color32::from_rgb(220, 230, 242)),
    );
    ui.add_space(8.0);

    let mut save = false;
    let mut save_and_connect = false;
    let mut cancel = false;

    {
        let form = &mut app.ssh_form;
        egui::Grid::new("wren-ssh-form")
            .num_columns(2)
            .spacing([14.0, 9.0])
            .show(ui, |ui| {
                ui.label("名称");
                ui.add(
                    egui::TextEdit::singleline(&mut form.name)
                        .hint_text("可选，例如：生产服务器")
                        .desired_width(360.0),
                );
                ui.end_row();

                ui.label(RichText::new("主机 *").color(crate::ui::TEXT));
                ui.add(
                    egui::TextEdit::singleline(&mut form.host)
                        .hint_text("IP 或域名，例如 192.168.1.10")
                        .desired_width(360.0),
                );
                ui.end_row();

                ui.label("用户名");
                ui.add(
                    egui::TextEdit::singleline(&mut form.user)
                        .hint_text("留空则使用 SSH 默认用户名")
                        .desired_width(360.0),
                );
                ui.end_row();

                ui.label("端口");
                ui.add(
                    egui::TextEdit::singleline(&mut form.port)
                        .hint_text("22")
                        .desired_width(90.0),
                );
                ui.end_row();

                ui.label("私钥路径");
                ui.add(
                    egui::TextEdit::singleline(&mut form.identity_file)
                        .hint_text(r"可选，例如 C:\Users\me\.ssh\id_ed25519")
                        .desired_width(360.0),
                );
                ui.end_row();

                ui.label("");
                ui.checkbox(
                    &mut form.poll_status,
                    RichText::new("连接后轮询远端系统状态（需免密登录）").size(12.0),
                );
                ui.end_row();
            });
    }

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if ui
            .add_sized(
                [120.0, 30.0],
                egui::Button::new(RichText::new("保存并连接").size(13.0)),
            )
            .clicked()
        {
            save_and_connect = true;
        }
        if ui.button("仅保存").clicked() {
            save = true;
        }
        if ui.button("取消").clicked() {
            cancel = true;
        }
    });

    if save_and_connect {
        app.save_ssh_form(true);
    } else if save {
        app.save_ssh_form(false);
    } else if cancel {
        app.ssh_form_open = false;
        app.editing_profile = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_validation() {
        let form = SshForm::default();
        assert!(form.to_profile(0).is_err(), "空主机必须被拒绝");

        let form = SshForm {
            host: "example.com".into(),
            port: "abc".into(),
            ..Default::default()
        };
        assert!(form.to_profile(0).is_err(), "非法端口必须被拒绝");

        let form = SshForm {
            host: "example.com".into(),
            port: String::new(),
            ..Default::default()
        };
        let p = form.to_profile(7).unwrap();
        assert_eq!(p.port, 22);
        assert_eq!(p.id, 7);
    }

    #[test]
    fn form_roundtrip() {
        let profile = SshProfile {
            id: 3,
            name: "生产机".into(),
            host: "10.0.0.2".into(),
            user: "deploy".into(),
            port: 2202,
            identity_file: "id_ed25519".into(),
            poll_status: false,
        };
        let form = SshForm::from_profile(&profile);
        let back = form.to_profile(3).unwrap();
        assert_eq!(back, profile);
    }
}
