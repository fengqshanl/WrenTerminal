//! 应用配置与 SSH 服务器配置的持久化。
//!
//! 配置文件位于 `%APPDATA%\Wren\app.json`，与 CLI 版 Wren 的历史记录分开存放。

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

/// 一条保存下来的 SSH 服务器配置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SshProfile {
    pub id: u64,
    /// 显示名称，留空时显示 user@host。
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: u16,
    /// 私钥路径，留空则使用 ssh-agent / 默认密钥。
    pub identity_file: String,
    /// 是否在连接后轮询远端系统状态（需要免密登录）。
    pub poll_status: bool,
}

impl Default for SshProfile {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            host: String::new(),
            user: String::new(),
            port: 22,
            identity_file: String::new(),
            poll_status: true,
        }
    }
}

impl SshProfile {
    /// `user@host`，未填用户名时只用主机名。
    pub fn target(&self) -> String {
        if self.user.trim().is_empty() {
            self.host.trim().to_string()
        } else {
            format!("{}@{}", self.user.trim(), self.host.trim())
        }
    }

    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            self.target()
        } else {
            name.to_string()
        }
    }

    pub fn valid(&self) -> bool {
        !self.host.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub ssh_profiles: Vec<SshProfile>,
    /// 终端字号。
    pub font_size: f32,
    /// 顶部状态栏详情面板是否展开。
    pub details_open: bool,
    /// 上次关闭时的窗口逻辑尺寸。
    pub window_size: Option<[f32; 2]>,
    /// 上次使用的 WSL 发行版。
    pub last_wsl_distro: Option<String>,
    /// 窗口是否保持置顶（由状态栏的“置顶”按钮控制）。
    pub always_on_top: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ssh_profiles: Vec::new(),
            font_size: 14.0,
            details_open: false,
            window_size: None,
            last_wsl_distro: None,
            always_on_top: false,
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let Some(path) = config_file() else {
            return Self::default();
        };
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(mut cfg) => {
                    cfg.font_size = cfg.font_size.clamp(10.0, 28.0);
                    cfg
                }
                Err(e) => {
                    log(&format!("配置文件解析失败，使用默认配置: {e}"));
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                log(&format!("读取配置文件失败: {e}"));
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = config_file() else { return };
        let Some(dir) = path.parent() else { return };
        if let Err(e) = std::fs::create_dir_all(dir) {
            log(&format!("创建配置目录失败: {e}"));
            return;
        }
        let data = match serde_json::to_vec_pretty(self) {
            Ok(d) => d,
            Err(e) => {
                log(&format!("序列化配置失败: {e}"));
                return;
            }
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, data) {
            log(&format!("写入配置失败: {e}"));
            return;
        }
        // Windows 上 rename 不能覆盖已存在的文件，先删除旧的。
        let _ = std::fs::remove_file(&path);
        if let Err(e) = std::fs::rename(&tmp, &path) {
            log(&format!("替换配置文件失败: {e}"));
        }
    }

    pub fn next_profile_id(&self) -> u64 {
        self.ssh_profiles
            .iter()
            .map(|p| p.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn upsert_profile(&mut self, mut profile: SshProfile) -> u64 {
        if profile.id == 0 {
            profile.id = self.next_profile_id();
        }
        if let Some(existing) = self.ssh_profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile.clone();
        } else {
            self.ssh_profiles.push(profile.clone());
        }
        profile.id
    }

    pub fn remove_profile(&mut self, id: u64) {
        self.ssh_profiles.retain(|p| p.id != id);
    }
}

pub fn data_dir() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            return Some(PathBuf::from(appdata).join("Wren"));
        }
    }
    std::env::var("USERPROFILE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|home| PathBuf::from(home).join(".wren"))
}

pub fn config_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("app.json"))
}

pub fn log_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("wren-app.log"))
}

/// 追加一行运行日志（超过 2 MiB 时轮转一次，避免无限增长）。
pub fn log(msg: &str) {
    let Some(path) = log_file() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 2 * 1024 * 1024 {
            let _ = std::fs::rename(&path, path.with_extension("log.old"));
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%F %T"), msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_target_and_name() {
        let mut p = SshProfile::default();
        p.host = "example.com".into();
        assert_eq!(p.target(), "example.com");
        p.user = "root".into();
        assert_eq!(p.target(), "root@example.com");
        assert_eq!(p.display_name(), "root@example.com");
        p.name = "生产机".into();
        assert_eq!(p.display_name(), "生产机");
    }

    #[test]
    fn upsert_and_remove_profiles() {
        let mut cfg = AppConfig::default();
        let id = cfg.upsert_profile(SshProfile {
            host: "a.example".into(),
            ..Default::default()
        });
        assert_eq!(cfg.ssh_profiles.len(), 1);
        cfg.upsert_profile(SshProfile {
            id,
            host: "b.example".into(),
            ..Default::default()
        });
        assert_eq!(cfg.ssh_profiles.len(), 1);
        assert_eq!(cfg.ssh_profiles[0].host, "b.example");
        cfg.remove_profile(id);
        assert!(cfg.ssh_profiles.is_empty());
    }
}
