//! Windows 相关的小工具：查找可执行程序、判断管理员权限、清理控制台输出。
//!
//! 这里不依赖任何第三方 crate，保持 Wren 单文件、零运行时依赖的特点。

use std::path::PathBuf;

/// 在 PATH 中查找可执行程序。
///
/// - `path_var` 可以是 Windows 风格的 `;` 分隔，也可以是 Linux 风格的 `:` 分隔。
/// - 名称既可以是 `winget`，也可以带扩展名（`winget.exe`）。
/// - 依次尝试 `.exe`、`.cmd`、`.bat`、`.com` 与无扩展名。
pub fn find_program(path_var: &str, name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }

    // 名字本身带路径时直接检查。
    if name.contains('/') || name.contains('\\') {
        let direct = PathBuf::from(name);
        return executable_candidates(&direct)
            .into_iter()
            .find(|p| p.is_file());
    }

    let dirs = split_path(path_var);
    for dir in dirs {
        if dir.trim().is_empty() {
            continue;
        }
        let base = PathBuf::from(dir.trim().trim_matches('"'));
        for candidate in executable_candidates(&base.join(name)) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_candidates(base: &std::path::Path) -> Vec<PathBuf> {
    let has_ext = base
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            matches!(e.as_str(), "exe" | "cmd" | "bat" | "com")
        })
        .unwrap_or(false);
    if has_ext {
        return vec![base.to_path_buf()];
    }
    // Windows 文件系统不区分大小写，这里用常见的小写扩展名即可。
    vec![
        base.with_extension("exe"),
        base.with_extension("cmd"),
        base.with_extension("bat"),
        base.with_extension("com"),
        base.to_path_buf(),
    ]
}

/// 拆分 PATH：同时兼容 Windows 的 `;` 和 Linux 风格的 `:`。
///
/// 关键点是不要把 `C:\` 里的冒号当成分隔符。这里用“当前段起始位置”
/// 判断盘符冒号，因此单个 `C:\Tools`、`C:\A;C:\B` 以及 `/c/a:/d/b`
/// 都能正确拆分。
pub fn split_path(path_var: &str) -> Vec<String> {
    let chars: Vec<char> = path_var.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut seg_start = 0usize;

    for (i, &c) in chars.iter().enumerate() {
        let drive_colon = c == ':'
            && i == seg_start + 1
            && chars
                .get(seg_start)
                .map(|ch| ch.is_ascii_alphabetic())
                .unwrap_or(false);
        if c == ';' || (c == ':' && !drive_colon) {
            parts.push(current.clone());
            current.clear();
            seg_start = i + 1;
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts
}

/// 当前进程是否以管理员权限运行。
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use std::ffi::c_void;

    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(
            process_handle: *mut c_void,
            desired_access: u32,
            token_handle: *mut *mut c_void,
        ) -> i32;
        fn GetTokenInformation(
            token_handle: *mut c_void,
            token_information_class: i32,
            token_information: *mut c_void,
            token_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(object: *mut c_void) -> i32;
    }

    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_ELEVATION: i32 = 20;

    unsafe {
        let mut token: *mut c_void = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: u32 = 0;
        let mut ret_len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TOKEN_ELEVATION,
            &mut elevation as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elevation != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

/// 把子进程输出的字节解码为字符串。
///
/// Windows 控制台程序重定向输出时通常是 UTF-8，但少数程序会写 UTF-16LE；
/// 这里对 BOM 和常见的 UTF-16LE 特征做一次兼容。
pub fn decode_console_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16le(&bytes[2..]);
    }
    if bytes.len() >= 4 && bytes.iter().take(64).filter(|b| **b == 0).count() * 4 > bytes.len().min(64) {
        // ASCII 居多的 UTF-16LE 输出，每个高位字节都是 0。
        return decode_utf16le(bytes);
    }
    String::from_utf8_lossy(bytes).to_string()
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// 清理 winget 等程序在管道输出里的进度动画与 ANSI 控制符。
///
/// - `\r` 用于原地刷新进度，只保留最后一次刷新后的内容（CRLF 不受影响）。
/// - 去掉 ANSI 转义序列、退格等控制字符。
/// - 保留换行与制表符，保留表格列之间的空格。
pub fn clean_console_text(input: &str) -> String {
    let mut out = String::new();
    let mut line = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    // CRLF：忽略 \r，由下面的 \n 负责换行。
                    continue;
                }
                // 进度条原地刷新：丢弃本行已有内容。
                line.clear();
            }
            '\n' => {
                out.push_str(line.trim_end());
                out.push('\n');
                line.clear();
            }
            '\x1b' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    // CSI：参数字节 0x30-0x3F，中间字节 0x20-0x2F，终止字节 0x40-0x7E。
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    // OSC：以 BEL 或 ESC \ 结束。
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\x08' => {
                line.pop();
            }
            '\t' => line.push('\t'),
            c if (c as u32) < 0x20 => {}
            _ => line.push(c),
        }
    }
    if !line.is_empty() {
        out.push_str(line.trim_end());
    }
    out
}

/// 查找可用的 Git Bash / MSYS2 bash，用于可选的 GNU 工具后备。
///
/// 只认常见安装位置，避免误把 `C:\Windows\System32\bash.exe`（WSL 启动器）
/// 当成 Git Bash。可用 `WREN_GIT_BASH` 指定自定义路径。
pub fn find_git_bash(path_var: &str) -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("WREN_GIT_BASH") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }

    let mut roots = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(base) = std::env::var(var) {
            roots.push(PathBuf::from(base).join("Git"));
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Programs").join("Git"));
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        roots.push(
            PathBuf::from(&home)
                .join("scoop")
                .join("apps")
                .join("git")
                .join("current"),
        );
    }
    roots.push(PathBuf::from(r"C:\Program Files\Git"));
    roots.push(PathBuf::from(r"C:\msys64"));

    for root in roots {
        for candidate in [
            root.join("bin").join("bash.exe"),
            root.join("usr").join("bin").join("bash.exe"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 最后看 PATH，但排除 WSL 的 System32\bash.exe。
    if let Some(p) = find_program(path_var, "bash") {
        let lower = p.to_string_lossy().to_ascii_lowercase();
        if !lower.contains("\\windows\\system32\\") && !lower.contains("windowsapps") {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_removes_progress_and_ansi() {
        let s = "\x1b[?25l正在下载 10%\r\x1b[K正在下载 80%\r正在下载 100%\n完成\n";
        assert_eq!(clean_console_text(s), "正在下载 100%\n完成\n");
    }

    #[test]
    fn clean_keeps_crlf_tables() {
        let s = "名称   ID\r\nGit    Git.Git\r\n";
        assert_eq!(clean_console_text(s), "名称   ID\nGit    Git.Git\n");
    }

    #[test]
    fn clean_handles_backspace() {
        assert_eq!(clean_console_text("abc\x08d\n"), "abd\n");
    }

    #[test]
    fn split_path_handles_drive_letters() {
        let parts = split_path(r"C:\Windows;C:\Tools");
        assert_eq!(parts, vec![r"C:\Windows", r"C:\Tools"]);

        // 单个 Windows 路径里没有分号时，不能把盘符冒号当分隔符。
        assert_eq!(split_path(r"C:\Tools"), vec![r"C:\Tools"]);

        // Linux 风格的冒号 PATH 仍然可以拆分。
        assert_eq!(
            split_path("/c/foo:/d/bar"),
            vec!["/c/foo", "/d/bar"]
        );
    }

    #[test]
    fn find_missing_program_returns_none() {
        assert!(find_program(r"C:\definitely-not-here", "nope").is_none());
    }
}
