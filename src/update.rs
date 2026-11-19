//! Wren 自更新：检查 GitHub Releases、缓存检查结果、下载并安装新版 MSI。
//!
//! 启动时只读取本地缓存（不阻塞），缓存过期才在后台访问 GitHub API；
//! `update` 命令则强制检查并可通过 UAC 启动 MSI 安装程序。

use crate::builtins::{Context, Flow, Output};
use crate::winutil::find_program;
use colored::Colorize;
use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{SystemTime, UNIX_EPOCH};

const REPO: &str = "fengqshanl/WrenTerminal";
const API_LATEST: &str = "https://api.github.com/repos/fengqshanl/WrenTerminal/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/fengqshanl/WrenTerminal/releases";
const CACHE_FILE: &str = ".wren_update_check";
const CACHE_TTL_OK: u64 = 24 * 3600;
const CACHE_TTL_FAIL: u64 = 6 * 3600;

/// 一条可用的更新信息。
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    /// 版本号，不含前缀 `v`。
    pub version: String,
    /// 安装包下载地址。
    pub url: String,
}

/// 启动时的检查结论（完全来自本地缓存）。
pub struct StartupStatus {
    pub notice: Option<UpdateInfo>,
    pub needs_refresh: bool,
}

#[derive(Debug, Clone)]
struct Cache {
    checked: u64,
    latest: String,
    url: String,
}

// ---------------------------------------------------------------------------
// 启动检查
// ---------------------------------------------------------------------------

/// 读取缓存，决定是否需要显示更新提示、是否需要后台刷新。
pub fn startup_status(home: &str, current: &str) -> StartupStatus {
    if disabled() {
        return StartupStatus {
            notice: None,
            needs_refresh: false,
        };
    }
    match read_cache(home) {
        Some(cache) => {
            let age = now().saturating_sub(cache.checked);
            let ttl = if cache.latest.is_empty() {
                CACHE_TTL_FAIL
            } else {
                CACHE_TTL_OK
            };
            let notice = if !cache.latest.is_empty() && is_newer(&cache.latest, current) {
                Some(UpdateInfo {
                    version: cache.latest,
                    url: cache.url,
                })
            } else {
                None
            };
            StartupStatus {
                notice,
                needs_refresh: age > ttl,
            }
        }
        None => StartupStatus {
            notice: None,
            needs_refresh: true,
        },
    }
}

/// 后台刷新缓存；发现比当前版本新的版本时通过 channel 通知。
pub fn spawn_refresh(home: String, current: String, path_var: String) -> Receiver<UpdateInfo> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || match fetch_latest(&current, &path_var) {
        Ok(info) => {
            write_cache(&home, Some(&info));
            if is_newer(&info.version, &current) {
                let _ = tx.send(info);
            }
        }
        Err(_) => {
            // 网络失败也写缓存，避免每次启动都卡在超时上。
            write_cache(&home, None);
        }
    });
    rx
}

/// 在启动界面打印更新提示。
pub fn print_notice(info: &UpdateInfo, current: &str) {
    println!(
        "{}",
        format!(
            "发现 Wren 新版本 v{}（当前 v{}）",
            info.version, current
        )
        .yellow()
        .bold()
    );
    println!(
        "{}",
        format!("运行 `update` 下载并安装，或访问 {}", RELEASES_PAGE).yellow()
    );
}

// ---------------------------------------------------------------------------
// update 内置命令
// ---------------------------------------------------------------------------

pub fn run(ctx: &mut Context, args: &[String]) -> Output {
    let mut check_only = false;
    let mut assume_yes = false;
    for a in args {
        match a.as_str() {
            "--check" | "-c" => check_only = true,
            "-y" | "--yes" => assume_yes = true,
            "-h" | "--help" => return Output::ok(help_text()),
            _ => return Output::err(format!("update: 不支持的选项 '{}'", a)),
        }
    }

    let current = env!("CARGO_PKG_VERSION");
    let home = ctx.env.home_string();
    let path_var = ctx.env.get_or("PATH", "");
    let interactive =
        ctx.interactive && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut sink = Sink::new(interactive);

    sink.line(format!("正在检查 Wren 更新（当前 v{}）...", current));
    let info = match fetch_latest(current, &path_var) {
        Ok(info) => info,
        Err(e) => {
            sink.error(format!("update: {}", e));
            sink.error(format!("也可以手动访问 {}", RELEASES_PAGE));
            return sink.finish(1);
        }
    };
    write_cache(&home, Some(&info));

    if !is_newer(&info.version, current) {
        sink.line(format!(
            "当前 v{}，已是最新版本（GitHub 最新发布 v{}）。",
            current, info.version
        ));
        return sink.finish(0);
    }

    sink.line(format!("发现新版本 v{}（当前 v{}）。", info.version, current));
    if check_only {
        sink.line(format!("下载地址：{}", info.url));
        sink.line("运行 `update` 下载并安装。");
        return sink.finish(0);
    }

    if interactive {
        if !assume_yes
            && !confirm(&format!(
                "是否下载并安装 v{}？安装时 Wren 会退出。[Y/n] ",
                info.version
            ))
        {
            sink.line("已取消。");
            return sink.finish(1);
        }
    } else if !assume_yes {
        sink.error("update: 非交互模式需要 -y（--yes）。");
        return sink.finish(1);
    }

    sink.line(format!("正在下载 Wren v{} 安装包...", info.version));
    let msi = match download_msi(&info, &path_var, interactive) {
        Ok(path) => path,
        Err(e) => {
            sink.error(format!("update: {}", e));
            return sink.finish(1);
        }
    };
    sink.line(format!("已下载到 {}", msi.display()));

    sink.line("正在请求管理员权限并启动安装程序（可能出现 UAC 提示）...");
    if let Err(e) = launch_installer(&msi, &path_var) {
        sink.error(format!("update: {}", e));
        return sink.finish(1);
    }

    sink.line("安装程序已启动。Wren 将退出，安装完成后请重新打开。");
    let mut out = sink.finish(0);
    out.flow = Flow::Exit;
    out
}

fn help_text() -> String {
    format!(
        "用法: update [选项]\n\
         \n\
         检查 GitHub Releases 上是否有 Wren 新版本，并下载 MSI 更新。\n\
         \n\
         选项:\n\
         \x20 --check, -c   只检查并显示最新版本，不下载\n\
         \x20 -y, --yes     自动确认（非交互模式必需）\n\
         \x20 -h, --help    显示本帮助\n\
         \n\
         说明:\n\
         \x20 交互模式启动 Wren 时会自动检查更新（每 24 小时一次，结果缓存）。\n\
         \x20 设置环境变量 WREN_NO_UPDATE_CHECK=1 可关闭启动检查。\n\
         \x20 更新源: {}",
        RELEASES_PAGE
    )
}

// ---------------------------------------------------------------------------
// 下载与安装
// ---------------------------------------------------------------------------

fn download_msi(info: &UpdateInfo, path_var: &str, interactive: bool) -> Result<PathBuf, String> {
    let curl = find_program(path_var, "curl")
        .ok_or_else(|| "未找到 curl.exe（Windows 10 1803+ 自带）".to_string())?;
    let url = if info.url.is_empty() {
        fallback_asset_url(&format!("v{}", info.version))
    } else {
        info.url.clone()
    };
    let file = std::env::temp_dir().join(format!("Wren-{}-x64.msi", info.version));
    let _ = fs::remove_file(&file);

    let mut cmd = Command::new(curl);
    cmd.args([
        "-L",
        "--fail",
        "--retry",
        "2",
        "--connect-timeout",
        "10",
        "--max-time",
        "600",
        "-o",
    ])
    .arg(&file)
    .arg(&url)
    .stdin(Stdio::null());
    if interactive {
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    } else {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    }

    let out = cmd.output().map_err(|e| format!("无法启动 curl: {}", e))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let _ = fs::remove_file(&file);
        return Err(format!(
            "下载安装包失败{}",
            if err.is_empty() {
                String::new()
            } else {
                format!("：{}", err)
            }
        ));
    }
    if let Err(e) = validate_msi(&file) {
        let _ = fs::remove_file(&file);
        return Err(e);
    }
    Ok(file)
}

/// 检查文件确实是 MSI（OLE 复合文档）。
fn validate_msi(path: &Path) -> Result<(), String> {
    const OLE_SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    let meta = fs::metadata(path).map_err(|e| format!("无法读取下载的文件: {}", e))?;
    if meta.len() < 50_000 {
        return Err("下载的文件过小，可能不是有效安装包".into());
    }
    let mut file = fs::File::open(path).map_err(|e| format!("无法读取下载的文件: {}", e))?;
    let mut sig = [0u8; 8];
    file.read_exact(&mut sig)
        .map_err(|_| "下载的文件不完整".to_string())?;
    if sig != OLE_SIGNATURE {
        return Err("下载的文件不是有效的 MSI 安装包（可能是网页或错误响应）".into());
    }
    Ok(())
}

/// 启动一个隐藏的提权 PowerShell：等待当前 Wren 退出后运行 msiexec。
fn launch_installer(msi: &Path, path_var: &str) -> Result<(), String> {
    let powershell =
        find_program(path_var, "powershell").ok_or_else(|| "未找到 powershell.exe".to_string())?;

    let script = format!(
        "$ErrorActionPreference = 'Stop'\r\n\
         Wait-Process -Id {} -ErrorAction SilentlyContinue\r\n\
         $msi = '{}'\r\n\
         Start-Process -FilePath 'msiexec.exe' -ArgumentList @('/i', ('\"' + $msi + '\"'), '/passive', '/norestart') -Wait\r\n",
        std::process::id(),
        ps_quote(&msi.to_string_lossy())
    );
    let encoded = base64_utf16le(&script);
    let outer = format!(
        "try {{ Start-Process -FilePath 'powershell.exe' -Verb RunAs -WindowStyle Hidden \
         -ArgumentList @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass',\
         '-WindowStyle','Hidden','-EncodedCommand','{}') -ErrorAction Stop | Out-Null; \
         exit 0 }} catch {{ Write-Error $_.Exception.Message; exit 1 }}",
        encoded
    );

    let out = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &outer,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("无法启动 PowerShell: {}", e))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!(
            "无法启动安装程序（可能取消了 UAC）{}",
            if err.is_empty() {
                String::new()
            } else {
                format!("：{}", err)
            }
        ));
    }
    Ok(())
}

fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

fn base64_utf16le(text: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = text
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(b2 & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

// ---------------------------------------------------------------------------
// GitHub API / 缓存
// ---------------------------------------------------------------------------

fn fetch_latest(current: &str, path_var: &str) -> Result<UpdateInfo, String> {
    let curl = find_program(path_var, "curl")
        .ok_or_else(|| "未找到 curl.exe（Windows 10 1803+ 自带）".to_string())?;
    let user_agent = format!("Wren/{}", current);
    let out = Command::new(curl)
        .args([
            "-sS",
            "-L",
            "--max-time",
            "8",
            "-H",
            &format!("User-Agent: {}", user_agent),
            "-H",
            "Accept: application/vnd.github+json",
            API_LATEST,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("无法启动 curl: {}", e))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!(
            "无法访问 GitHub Releases{}",
            if err.is_empty() {
                String::new()
            } else {
                format!("：{}", err)
            }
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let tag = json_string_field(&text, "tag_name")
        .ok_or_else(|| "GitHub 返回内容中缺少版本信息（可能尚未发布 Release）".to_string())?;
    let version = normalize_version(&tag);
    if version.is_empty() || !version.chars().any(|c| c.is_ascii_digit()) {
        return Err("无法识别 GitHub Release 的版本号".into());
    }
    let urls = download_urls(&text);
    let url = urls
        .iter()
        .find(|u| is_msi_url(u, true))
        .or_else(|| urls.iter().find(|u| is_msi_url(u, false)))
        .cloned()
        .unwrap_or_else(|| fallback_asset_url(&tag));

    Ok(UpdateInfo { version, url })
}

fn normalize_version(tag: &str) -> String {
    tag.trim()
        .trim_start_matches(|c| c == 'v' || c == 'V')
        .to_string()
}

fn fallback_asset_url(tag: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/{}/Wren-{}-x64.msi",
        REPO, tag, tag
    )
}

fn is_msi_url(url: &str, require_x64: bool) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.ends_with(".msi") && (!require_x64 || lower.contains("x64") || lower.contains("amd64"))
}

/// 从 GitHub Release JSON 中取第一个字符串字段（只处理基本的转义）。
fn json_string_field(text: &str, key: &str) -> Option<String> {
    let marker = format!("\"{}\"", key);
    let start = text.find(&marker)?;
    let rest = &text[start + marker.len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let quote = rest.find('"')?;
    let mut out = String::new();
    let mut chars = rest[quote + 1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some(other) => out.push(other),
                None => break,
            },
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

/// 从 JSON 中收集所有 browser_download_url。
fn download_urls(text: &str) -> Vec<String> {
    let marker = "\"browser_download_url\"";
    let mut urls = Vec::new();
    let mut from = 0;
    while let Some(pos) = text[from..].find(marker) {
        let abs = from + pos;
        if let Some(url) = json_string_field(&text[abs..], "browser_download_url") {
            urls.push(url);
        }
        from = abs + marker.len();
    }
    urls
}

fn cache_path(home: &str) -> PathBuf {
    Path::new(home).join(CACHE_FILE)
}

fn read_cache(home: &str) -> Option<Cache> {
    let text = fs::read_to_string(cache_path(home)).ok()?;
    let mut checked = 0u64;
    let mut latest = String::new();
    let mut url = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("checked=") {
            checked = v.trim().parse().ok()?;
        } else if let Some(v) = line.strip_prefix("latest=") {
            latest = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("url=") {
            url = v.trim().to_string();
        }
    }
    Some(Cache {
        checked,
        latest,
        url,
    })
}

fn write_cache(home: &str, info: Option<&UpdateInfo>) {
    let (latest, url) = match info {
        Some(info) => (info.version.as_str(), info.url.as_str()),
        None => ("", ""),
    };
    let content = format!("checked={}\nlatest={}\nurl={}\n", now(), latest, url);
    let _ = fs::write(cache_path(home), content);
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn disabled() -> bool {
    match std::env::var("WREN_NO_UPDATE_CHECK") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// 版本比较 / 交互
// ---------------------------------------------------------------------------

fn is_newer(candidate: &str, current: &str) -> bool {
    let a = version_segments(candidate);
    let b = version_segments(current);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn version_segments(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn confirm(prompt: &str) -> bool {
    print!("{}", prompt);
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    let answer = input.trim().to_ascii_lowercase();
    answer.is_empty() || answer == "y" || answer == "yes"
}

struct Sink {
    interactive: bool,
    stdout: String,
    stderr: String,
}

impl Sink {
    fn new(interactive: bool) -> Self {
        Self {
            interactive,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn line(&mut self, text: impl AsRef<str>) {
        if self.interactive {
            println!("{}", text.as_ref());
        } else {
            self.stdout.push_str(text.as_ref());
            self.stdout.push('\n');
        }
    }

    fn error(&mut self, text: impl AsRef<str>) {
        if self.interactive {
            eprintln!("{}", text.as_ref());
        } else {
            self.stderr.push_str(text.as_ref());
            self.stderr.push('\n');
        }
    }

    fn finish(self, code: i32) -> Output {
        Output {
            stdout: self.stdout,
            stderr: self.stderr,
            code,
            flow: Flow::Continue,
            raw: false,
        }
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_detection() {
        assert!(is_newer("0.3.0", "0.2.1"));
        assert!(is_newer("0.2.2", "0.2.1"));
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("0.1.1", "0.2.1"));
    }

    #[test]
    fn tag_normalization() {
        assert_eq!(normalize_version("v0.2.1"), "0.2.1");
        assert_eq!(normalize_version("0.2.1"), "0.2.1");
    }

    #[test]
    fn json_fields_are_parsed() {
        let text = r#"{"tag_name":"v1.2.3","assets":[{"browser_download_url":"https://x/Wren-v1.2.3-x64.msi"}]}"#;
        assert_eq!(json_string_field(text, "tag_name").as_deref(), Some("v1.2.3"));
        let urls = download_urls(text);
        assert_eq!(urls.len(), 1);
        assert!(is_msi_url(&urls[0], true));
    }

    #[test]
    fn cache_roundtrip() {
        let dir = std::env::temp_dir().join(format!("wren_update_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let home = dir.to_string_lossy().to_string();
        let info = UpdateInfo {
            version: "9.9.9".into(),
            url: "https://example.com/Wren.msi".into(),
        };
        write_cache(&home, Some(&info));
        let cache = read_cache(&home).unwrap();
        assert_eq!(cache.latest, "9.9.9");
        assert_eq!(cache.url, "https://example.com/Wren.msi");
        assert!(is_newer(&cache.latest, "0.1.0"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_utf16le_works() {
        // "abc" -> UTF-16LE "61 00 62 00 63 00" -> YQBiAGMA
        assert_eq!(base64_utf16le("abc"), "YQBiAGMA");
    }

    #[test]
    fn powershell_quote_doubles_single_quotes() {
        assert_eq!(ps_quote(r"C:\O'Brien\Wren.msi"), r"C:\O''Brien\Wren.msi");
    }
}
