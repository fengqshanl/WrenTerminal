//! 路径工具：在 Linux 风格路径（`/c/Users/foo`、`~`）与 Windows 路径之间转换。

use std::path::{Component, Path, PathBuf};

/// Windows 下把路径统一成正斜杠形式，便于显示与比较。
fn normalize_slashes(s: &str) -> String {
    s.replace('\\', "/")
}

/// 判断某个字节是否为盘符字母。
fn is_drive_letter(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// Linux 的 `/tmp` 对应的 Windows 目录。
///
/// 优先使用 `%LOCALAPPDATA%\Temp`，这样和 Git Bash / MSYS 的 `/tmp` 保持一致；
/// 取不到时回退到 `std::env::temp_dir()`。
pub fn linux_tmp_dir() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local).join("Temp");
        if p.is_dir() {
            return p;
        }
    }
    std::env::temp_dir()
}

/// 把用户输入的路径（可能是 `/c/foo`、`~/x`、相对路径）解析为 Windows `PathBuf`。
///
/// 规则：
/// - `~` / `~/...` 展开为用户主目录
/// - `/c/Users/foo` -> `C:\Users\foo`（POSIX 风格盘符）
/// - `/foo` -> 当前盘符根目录下的 `\foo`
/// - 普通相对路径基于 `cwd` 解析
pub fn to_windows(input: &str, cwd: &Path, home: &Path) -> PathBuf {
    if input.is_empty() {
        return cwd.to_path_buf();
    }

    let normalized = normalize_slashes(input);

    // ~ 展开
    let normalized = if normalized == "~" {
        home.to_string_lossy().to_string()
    } else if let Some(rest) = normalized.strip_prefix("~/") {
        let h = normalize_slashes(&home.to_string_lossy());
        format!("{}/{}", h.trim_end_matches('/'), rest)
    } else {
        normalized
    };

    let bytes = normalized.as_bytes();

    // Linux 伪文件：/dev/null（以及 /dev/zero）映射到 Windows 的 NUL 设备。
    let lower = normalized.to_ascii_lowercase();
    if lower == "/dev/null" || lower == "/dev/zero" {
        return PathBuf::from("NUL");
    }

    // /tmp、/var/tmp 映射到 Windows 临时目录。
    if lower == "/tmp" || lower.starts_with("/tmp/") {
        let rest = &normalized[4..];
        let base = linux_tmp_dir();
        return if rest.is_empty() {
            base
        } else {
            base.join(rest.trim_start_matches('/').replace('/', "\\"))
        };
    }

    // POSIX 盘符路径：/c/foo -> C:\foo；单独的 /d -> D:\
    if bytes.len() == 2 && bytes[0] == b'/' && is_drive_letter(bytes[1]) {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        return PathBuf::from(format!("{}:\\", drive));
    }
    if bytes.len() >= 3 && bytes[0] == b'/' && is_drive_letter(bytes[1]) && bytes[2] == b'/' {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        let rest = &normalized[3..];
        let rest = rest.trim_start_matches('/').replace('/', "\\");
        if rest.is_empty() {
            return PathBuf::from(format!("{}:\\", drive));
        }
        return PathBuf::from(format!("{}:\\{}", drive, rest));
    }

    // 以 / 开头的其它绝对路径：放到当前盘根目录
    if bytes.first() == Some(&b'/') {
        let drive = current_drive(cwd);
        let rest = normalized.trim_start_matches('/').replace('/', "\\");
        if rest.is_empty() {
            return PathBuf::from(format!("{}:\\", drive));
        }
        return PathBuf::from(format!("{}:\\{}", drive, rest));
    }

    let p = PathBuf::from(normalized.replace('/', "\\"));
    if p.is_absolute() {
        p
    } else {
        cwd.join(p)
    }
}

/// 取当前工作目录所在盘符（大写字母），取不到时回退到 `C`。
fn current_drive(cwd: &Path) -> char {
    let s = cwd.to_string_lossy();
    let b = s.as_bytes();
    if b.len() >= 2 && is_drive_letter(b[0]) && b[1] == b':' {
        (b[0] as char).to_ascii_uppercase()
    } else {
        'C'
    }
}

/// 把 Windows 路径转换为 POSIX 风格显示（`C:\Users` -> `/c/Users`）。
pub fn to_posix(path: &Path) -> String {
    let s = normalize_slashes(&path.to_string_lossy());
    let b = s.as_bytes();
    if b.len() >= 2 && is_drive_letter(b[0]) && b[1] == b':' {
        let drive = (b[0] as char).to_ascii_lowercase();
        let tail = &s[2..];
        let trimmed = tail.trim_matches('/');
        if trimmed.is_empty() {
            format!("/{}", drive)
        } else if tail.starts_with('/') {
            format!("/{}/{}", drive, trimmed)
        } else {
            format!("/{}{}", drive, tail)
        }
    } else if s.is_empty() {
        "/".to_string()
    } else {
        s
    }
}

/// 用于提示符/显示的路径：位于 home 下时显示为 `~` / `~/sub`。
pub fn display(path: &Path, home: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(home) {
        if rel.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~/{}", to_posix(rel).trim_start_matches('/'));
    }
    to_posix(path)
}

/// 规范化路径（去掉 `.` 与 `..`），不要求路径真实存在。
pub fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => out.push(c),
        }
    }
    out
}

/// 判断一个 token 是否看起来是需要做 POSIX->Windows 转换的路径。
/// 只转换以 `/x/...` 形式出现的绝对路径，避免误伤 `-c` 之类的选项。
pub fn looks_like_posix_path(token: &str) -> bool {
    let b = token.as_bytes();
    b.len() >= 3 && b[0] == b'/' && is_drive_letter(b[1]) && b[2] == b'/'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_drive_conversion() {
        let cwd = PathBuf::from(r"C:\Users\me");
        let home = PathBuf::from(r"C:\Users\me");
        assert_eq!(
            to_windows("/c/Windows/System32", &cwd, &home),
            PathBuf::from(r"C:\Windows\System32")
        );
        assert_eq!(to_posix(Path::new(r"C:\Users\me")), "/c/Users/me");
    }

    #[test]
    fn home_expansion() {
        let cwd = PathBuf::from(r"C:\Users\me");
        let home = PathBuf::from(r"C:\Users\me");
        assert_eq!(to_windows("~", &cwd, &home), home);
        assert_eq!(to_windows("~/proj", &cwd, &home), PathBuf::from(r"C:\Users\me\proj"));
    }
}
