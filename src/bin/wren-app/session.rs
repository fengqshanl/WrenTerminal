//! 一个终端会话：本地 Wren / WSL / SSH，以及它对应的状态采集。

use crate::backend::{BackendSettings, PtyEvent, TerminalBackend};
use crate::config::SshProfile;
use crate::stats::{self, RemoteTarget, StatusHandle};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum SessionKind {
    Local,
    Wsl { distro: Option<String> },
    Ssh { profile: SshProfile },
}

impl SessionKind {
    /// 标签页标题。
    pub fn title(&self) -> String {
        match self {
            SessionKind::Local => "本地".into(),
            SessionKind::Wsl { distro } => distro
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| "WSL".into()),
            SessionKind::Ssh { profile } => profile.display_name(),
        }
    }

    /// 状态栏左侧的连接类型徽标。
    pub fn badge(&self) -> &'static str {
        match self {
            SessionKind::Local => "本地",
            SessionKind::Wsl { .. } => "WSL",
            SessionKind::Ssh { .. } => "SSH",
        }
    }

    /// 连接目标的人类可读描述。
    pub fn target(&self) -> String {
        match self {
            SessionKind::Local => "Windows".into(),
            SessionKind::Wsl { distro } => format!(
                "WSL · {}",
                distro
                    .clone()
                    .filter(|d| !d.trim().is_empty())
                    .unwrap_or_else(|| "默认发行版".into())
            ),
            SessionKind::Ssh { profile } => {
                if profile.port != 0 && profile.port != 22 {
                    format!("{}:{}", profile.target(), profile.port)
                } else {
                    profile.target()
                }
            }
        }
    }

    pub fn is_remote(&self) -> bool {
        !matches!(self, SessionKind::Local)
    }

    fn remote_target(&self) -> Option<RemoteTarget> {
        match self {
            SessionKind::Local => None,
            SessionKind::Wsl { distro } => Some(RemoteTarget::Wsl {
                distro: distro.clone(),
            }),
            SessionKind::Ssh { profile } => {
                if profile.poll_status && profile.valid() {
                    Some(RemoteTarget::Ssh {
                        profile: profile.clone(),
                    })
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Running,
    /// 子进程退出，附带退出码。
    Exited(i32),
    /// PTY 创建失败，错误文本在 `Session::spawn_error`。
    Failed,
}

pub struct Session {
    /// 每次创建 PTY 都分配新 id：egui 用它区分终端部件，事件也按它路由。
    pub backend_id: u64,
    pub kind: SessionKind,
    pub backend: Option<TerminalBackend>,
    pub state: SessionState,
    pub spawn_error: Option<String>,
    pub stats: Option<StatusHandle>,
    /// 终端程序通过 OSC 设置的真实标题（例如 vim / ssh 的窗口标题）。
    pub term_title: Option<String>,
}

impl Session {
    pub fn spawn(
        backend_id: u64,
        kind: SessionKind,
        ctx: &egui::Context,
        events: Sender<(u64, PtyEvent)>,
    ) -> Self {
        let mut session = Self {
            backend_id,
            kind,
            backend: None,
            state: SessionState::Running,
            spawn_error: None,
            stats: None,
            term_title: None,
        };

        let (shell, args, cwd) = match &session.kind {
            SessionKind::Local => {
                let (program, args) = find_local_shell();
                (program, args, home_dir())
            }
            SessionKind::Wsl { distro } => (
                "wsl.exe".into(),
                wsl_args(distro.as_deref()),
                None,
            ),
            SessionKind::Ssh { profile } => {
                ("ssh.exe".into(), ssh_args(profile), None)
            }
        };

        let settings = BackendSettings {
            shell,
            args,
            working_directory: cwd,
        };

        match TerminalBackend::new(backend_id, ctx.clone(), events, settings) {
            Ok(backend) => {
                session.backend = Some(backend);
                session.stats = match &session.kind {
                    SessionKind::Local => Some(stats::spawn_local(ctx.clone())),
                    SessionKind::Wsl { .. } => session
                        .kind
                        .remote_target()
                        .map(|t| stats::spawn_remote(ctx.clone(), t, Duration::from_secs(4))),
                    SessionKind::Ssh { .. } => session
                        .kind
                        .remote_target()
                        .map(|t| stats::spawn_remote(ctx.clone(), t, Duration::from_secs(8))),
                };
            }
            Err(e) => {
                session.state = SessionState::Failed;
                session.spawn_error = Some(e.to_string());
                crate::config::log(&format!(
                    "创建终端失败（{}）: {e}",
                    session.kind.target()
                ));
            }
        }

        session
    }

    pub fn title(&self) -> String {
        // 标签标题优先使用用户能理解的目标名（本地 / 发行版 / 服务器名）。
        // 终端程序通过 OSC 设置的标题常常是主机名或当前路径，只作为悬停提示。
        self.kind.title()
    }

    /// 终端程序自己设置的标题，用于悬停提示。
    pub fn term_title(&self) -> Option<&str> {
        self.term_title.as_deref()
    }
}

/// 交互式 WSL 启动参数：指定发行版（可选），并从 Linux 用户主目录开始。
pub(crate) fn wsl_args(distro: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(d) = distro.filter(|d| !d.trim().is_empty()) {
        args.push("-d".to_string());
        args.push(d.to_string());
    }
    args.push("--cd".into());
    args.push("~".into());
    args
}

/// 交互式 SSH 启动参数：强制分配远端 PTY，并开启保活。
pub(crate) fn ssh_args(profile: &SshProfile) -> Vec<String> {
    let mut args = vec![
        "-tt".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "ServerAliveInterval=30".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
    ];
    if profile.port != 0 && profile.port != 22 {
        args.push("-p".into());
        args.push(profile.port.to_string());
    }
    if !profile.identity_file.trim().is_empty() {
        args.push("-i".into());
        args.push(profile.identity_file.trim().to_string());
    }
    args.push(profile.target());
    args
}

fn home_dir() -> Option<PathBuf> {    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
}

/// 找到与 wren-app.exe 同目录的 wren.exe；找不到时退回系统命令解释器。
fn find_local_shell() -> (String, Vec<String>) {
    let exe_name = format!("wren{}", std::env::consts::EXE_SUFFIX);
    let candidates = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(&exe_name)))
        .filter(|p| p.is_file());
    if let Some(path) = candidates {
        return (path.to_string_lossy().to_string(), Vec::new());
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(&exe_name);
            if p.is_file() {
                return (p.to_string_lossy().to_string(), Vec::new());
            }
        }
    }
    crate::config::log("未找到 wren.exe，临时使用系统命令解释器");
    if cfg!(windows) {
        ("cmd.exe".into(), Vec::new())
    } else {
        ("/bin/sh".into(), Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_titles() {
        assert_eq!(SessionKind::Local.title(), "本地");
        assert_eq!(
            SessionKind::Wsl {
                distro: Some("Ubuntu".into())
            }
            .title(),
            "Ubuntu"
        );
        let p = SshProfile {
            name: "生产机".into(),
            host: "10.0.0.1".into(),
            user: "root".into(),
            port: 2222,
            ..Default::default()
        };
        assert_eq!(SessionKind::Ssh { profile: p.clone() }.title(), "生产机");
        assert_eq!(SessionKind::Ssh { profile: p }.target(), "root@10.0.0.1:2222");
    }

    #[test]
    fn wsl_and_ssh_args() {
        assert_eq!(wsl_args(None), vec!["--cd", "~"]);
        assert_eq!(
            wsl_args(Some("openSUSE-Tumbleweed")),
            vec!["-d", "openSUSE-Tumbleweed", "--cd", "~"]
        );

        let profile = SshProfile {
            host: "example.com".into(),
            user: "root".into(),
            port: 2222,
            identity_file: r"C:\Users\me\.ssh\id_ed25519".into(),
            ..Default::default()
        };
        let args = ssh_args(&profile);
        assert!(args.contains(&"-tt".to_string()));
        assert!(args.windows(2).any(|w| w == ["-p", "2222"]));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-i" && w[1].ends_with("id_ed25519")));
        assert_eq!(args.last().unwrap(), "root@example.com");

        let default_port = SshProfile {
            host: "example.com".into(),
            user: "root".into(),
            ..Default::default()
        };
        assert!(!ssh_args(&default_port).contains(&"-p".to_string()));
    }
}
