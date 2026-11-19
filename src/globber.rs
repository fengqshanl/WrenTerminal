//! Shell 风格的通配符展开（glob）。
//!
//! 支持 `*`、`?`、`[abc]`、`[a-z]`，与 bash 一样：
//! - 只展开“未加引号”的参数（引号内的通配符由 parser 打上保护标记）
//! - 没有匹配项时保留原样
//! - 不会匹配以 `.` 开头的隐藏文件，除非模式本身以 `.` 开头
//! - 结果按名字排序

use crate::parser::GLOB_ESCAPE;
use crate::pathutil;
use std::fs;
use std::path::{Path, PathBuf};

/// 判断字符串是否含有需要展开的通配符。
pub fn has_wildcard(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '*' | '?' | '[')) && !s.contains(GLOB_ESCAPE)
}

/// 展开参数列表中的通配符。第一个元素（命令名）保持原样。
pub fn expand_args(argv: &[String], cwd: &Path) -> Vec<String> {
    if argv.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(argv.len());
    out.push(argv[0].replace(GLOB_ESCAPE, ""));

    for arg in &argv[1..] {
        if arg.contains(GLOB_ESCAPE) {
            out.push(arg.replace(GLOB_ESCAPE, ""));
            continue;
        }
        if !has_wildcard(arg) {
            out.push(arg.clone());
            continue;
        }
        let matches = expand_pattern(arg, cwd);
        if matches.is_empty() {
            out.push(arg.clone());
        } else {
            out.extend(matches);
        }
    }
    out
}

/// 展开单个模式。
pub fn expand_pattern(pattern: &str, cwd: &Path) -> Vec<String> {
    let normalized = pattern.replace('\\', "/");
    if normalized.is_empty() {
        return Vec::new();
    }

    // 计算起始目录与显示前缀。
    let (root, rest, prefix): (PathBuf, String, String) = if let Some(tail) = posix_drive_tail(&normalized)
    {
        let drive = normalized.as_bytes()[1] as char;
        (
            PathBuf::from(format!("{}:\\", drive.to_ascii_uppercase())),
            tail,
            format!("/{}/", drive.to_ascii_lowercase()),
        )
    } else if let Some(tail) = normalized.strip_prefix('/') {
        let drive = current_drive(cwd);
        (
            PathBuf::from(format!("{}:\\", drive)),
            tail.to_string(),
            "/".to_string(),
        )
    } else {
        let (rest, prefix) = match normalized.strip_prefix("./") {
            Some(r) => (r.to_string(), "./".to_string()),
            None => (normalized.clone(), String::new()),
        };
        (cwd.to_path_buf(), rest, prefix)
    };

    let segments: Vec<&str> = rest
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if segments.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();
    walk(&root, &segments, &prefix, &mut results);
    results.sort();
    results
}

fn posix_drive_tail(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/' {
        Some(s[3..].to_string())
    } else if bytes.len() == 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() {
        Some(String::new())
    } else {
        None
    }
}

fn current_drive(cwd: &Path) -> char {
    let s = cwd.to_string_lossy();
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        (b[0] as char).to_ascii_uppercase()
    } else {
        'C'
    }
}

fn walk(base: &Path, segments: &[&str], prefix: &str, results: &mut Vec<String>) {
    if segments.is_empty() {
        return;
    }
    let seg = segments[0];
    let last = segments.len() == 1;

    if !has_wildcard(seg) {
        let literal = seg.replace(GLOB_ESCAPE, "");
        let next = base.join(&literal);
        let shown = format!("{}{}", prefix, literal);
        if last {
            if next.exists() {
                results.push(shown);
            }
        } else if next.is_dir() {
            walk(&next, &segments[1..], &format!("{}/", shown), results);
        }
        return;
    }

    let entries = match fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return,
    };
    let allow_hidden = seg.starts_with('.');
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && !allow_hidden {
                return None;
            }
            if pattern_match(&name, seg) {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    names.sort();

    for name in names {
        let shown = format!("{}{}", prefix, name);
        let path = base.join(&name);
        if last {
            results.push(shown);
        } else if path.is_dir() {
            walk(&path, &segments[1..], &format!("{}/", shown), results);
        }
    }
}

/// 单个路径段的名字匹配：`*`、`?`、`[abc]`、`[a-z]`、`[!...]`。
///
/// Windows 文件名不区分大小写，这里也按不区分大小写匹配。
pub fn pattern_match(name: &str, pattern: &str) -> bool {
    let n: Vec<char> = name.chars().flat_map(|c| c.to_lowercase()).collect();
    let p: Vec<char> = pattern.chars().flat_map(|c| c.to_lowercase()).collect();
    match_chars(&n, &p)
}

fn match_chars(name: &[char], pat: &[char]) -> bool {
    if pat.is_empty() {
        return name.is_empty();
    }
    match pat[0] {
        '*' => {
            // 合并连续的 *，避免指数级回溯。
            let mut i = 1;
            while i < pat.len() && pat[i] == '*' {
                i += 1;
            }
            let rest = &pat[i..];
            if rest.is_empty() {
                return true;
            }
            for k in 0..=name.len() {
                if match_chars(&name[k..], rest) {
                    return true;
                }
            }
            false
        }
        '?' => !name.is_empty() && match_chars(&name[1..], &pat[1..]),
        '[' => {
            if name.is_empty() {
                return false;
            }
            match match_class(name[0], &pat[1..]) {
                Some(consumed) => match_chars(&name[1..], &pat[1 + consumed..]),
                None => name[0] == '[' && match_chars(&name[1..], &pat[1..]),
            }
        }
        c => !name.is_empty() && name[0] == c && match_chars(&name[1..], &pat[1..]),
    }
}

/// 解析 `[...]`，返回 (是否匹配, 字符类占用的长度)。
fn match_class(c: char, pat: &[char]) -> Option<usize> {
    let mut i = 0;
    let negate = if i < pat.len() && (pat[i] == '!' || pat[i] == '^') {
        i += 1;
        true
    } else {
        false
    };
    let mut matched = false;
    let mut first = true;
    while i < pat.len() {
        if pat[i] == ']' && !first {
            return if matched != negate { Some(i + 1) } else { None };
        }
        first = false;
        // 范围写法 a-z
        if i + 2 < pat.len() && pat[i + 1] == '-' && pat[i + 2] != ']' {
            if pat[i] <= c && c <= pat[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if pat[i] == c {
                matched = true;
            }
            i += 1;
        }
    }
    None
}

/// 把结果路径转成 POSIX 显示形式（用于提示/调试）。
pub fn to_display(path: &Path) -> String {
    pathutil::to_posix(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wren_glob_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn simple_star_matches_files() {
        let dir = temp_dir("star");
        fs::write(dir.join("a.rs"), "").unwrap();
        fs::write(dir.join("b.rs"), "").unwrap();
        fs::write(dir.join("c.txt"), "").unwrap();

        let mut got = expand_pattern("*.rs", &dir);
        got.sort();
        assert_eq!(got, vec!["a.rs", "b.rs"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn question_and_class() {
        let dir = temp_dir("class");
        fs::write(dir.join("a1.txt"), "").unwrap();
        fs::write(dir.join("a2.txt"), "").unwrap();
        fs::write(dir.join("ab.txt"), "").unwrap();

        let mut got = expand_pattern("a[0-9].txt", &dir);
        got.sort();
        assert_eq!(got, vec!["a1.txt", "a2.txt"]);

        let got = expand_pattern("a?.txt", &dir);
        assert_eq!(got.len(), 3);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hidden_files_need_explicit_dot() {
        let dir = temp_dir("hidden");
        fs::write(dir.join(".hidden"), "").unwrap();
        fs::write(dir.join("visible"), "").unwrap();

        assert_eq!(expand_pattern("*", &dir), vec!["visible"]);
        assert_eq!(expand_pattern(".*", &dir), vec![".hidden"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_directories() {
        let dir = temp_dir("nested");
        fs::create_dir_all(dir.join("src/sub")).unwrap();
        fs::write(dir.join("src/main.rs"), "").unwrap();
        fs::write(dir.join("src/sub/lib.rs"), "").unwrap();

        let mut got = expand_pattern("src/*.rs", &dir);
        got.sort();
        assert_eq!(got, vec!["src/main.rs"]);

        let mut got = expand_pattern("src/*/*.rs", &dir);
        got.sort();
        assert_eq!(got, vec!["src/sub/lib.rs"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_match_keeps_pattern() {
        let dir = temp_dir("none");
        let argv = vec![
            "ls".to_string(),
            "*.nomatch".to_string(),
            "plain".to_string(),
        ];
        let got = expand_args(&argv, &dir);
        assert_eq!(got, vec!["ls", "*.nomatch", "plain"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn quoted_pattern_is_not_expanded() {
        let dir = temp_dir("quoted");
        fs::write(dir.join("a.rs"), "").unwrap();
        let protected = format!("{}*.rs", GLOB_ESCAPE);
        let argv = vec!["ls".to_string(), protected];
        let got = expand_args(&argv, &dir);
        assert_eq!(got, vec!["ls", "*.rs"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pattern_match_is_case_insensitive() {
        assert!(pattern_match("README.md", "readme.*"));
        assert!(pattern_match("Main.RS", "*.rs"));
    }
}
