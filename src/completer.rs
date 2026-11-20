//! Tab 补全：命令名、文件名与环境变量补全，基于 rustyline。

use crate::builtins;
use crate::env::Env;
use crate::pathutil;
use crate::translator;
use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context as RlContext, Helper};
use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// rustyline 的 Helper 实现，持有当前目录的共享引用以便补全文件。
pub struct ShellHelper {
    pub cwd: Arc<RwLock<PathBuf>>,
    env_names: Vec<String>,
    /// PATH 中可执行文件的命令名（启动时扫描一次）。
    path_commands: Vec<String>,
}

impl ShellHelper {
    pub fn new(cwd: Arc<RwLock<PathBuf>>, env: &Env) -> Self {
        let mut env_names: Vec<String> = env.all().into_iter().map(|(k, _)| k).collect();
        env_names.sort();
        env_names.dedup();
        let path_commands = scan_path_commands(&env.get_or("PATH", ""));
        Self {
            cwd,
            env_names,
            path_commands,
        }
    }

    /// 所有可补全的命令名。
    fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = builtins::BUILTINS.iter().map(|b| b.name.to_string()).collect();
        names.extend(["quit", "cls", "ll", "la", "printenv", "egrep", "fgrep"].iter().map(|s| s.to_string()));
        names.extend(translator::known_commands());
        names.extend(self.path_commands.iter().cloned());
        names.sort();
        names.dedup();
        names
    }

    fn complete_env(&self, name: &str) -> Vec<String> {
        self.env_names
            .iter()
            .filter(|n| n.starts_with(name))
            .map(|n| format!("${}", n))
            .collect()
    }

    fn complete_command(&self, word: &str) -> Vec<String> {
        self.command_names()
            .into_iter()
            .filter(|c| c.starts_with(word))
            .collect()
    }

    fn complete_file(&self, word: &str) -> Vec<String> {
        let (dir_part, name_part) = match word.rfind('/') {
            // rfind 返回 '/' 的字节位置，切片天然在边界上；get 再兜底一层。
            Some(i) => (
                word.get(..=i).unwrap_or(word),
                word.get(i + 1..).unwrap_or(""),
            ),
            None => ("", word),
        };
        let cwd = self.cwd.read().map(|g| g.clone()).unwrap_or_default();
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        let base = {
            let expanded = if dir_part == "~/" { home.clone() } else { dir_part.to_string() };
            pathutil::to_windows(&expanded, &cwd, std::path::Path::new(&home))
        };
        let entries = match fs::read_dir(&base) { Ok(e) => e, Err(_) => return vec![] };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(name_part) { continue; }
            if name.starts_with('.') && !name_part.starts_with('.') { continue; }
            let suffix = if entry.path().is_dir() { "/" } else { "" };
            out.push(format!("{}{}{}", dir_part, name, suffix));
        }
        out.sort();
        out
    }
}

/// 扫描 PATH，收集可执行文件名（不含扩展名）。
fn scan_path_commands(path_var: &str) -> Vec<String> {
    let mut names = Vec::new();
    for dir in crate::winutil::split_path(path_var) {
        let dir = dir.trim().trim_matches('"');
        if dir.is_empty() {
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            let lower = fname.to_ascii_lowercase();
            for ext in [".exe", ".cmd", ".bat", ".com"] {
                if lower.ends_with(ext) {
                    let stem = &fname[..fname.len() - ext.len()];
                    if !stem.is_empty() {
                        names.push(stem.to_string());
                    }
                    break;
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// 计算光标前最后一个分隔符之后的起始字节位置。
///
/// 分隔符本身可能是多字节字符（例如中文全角空格 U+3000），
/// 因此必须加上 `len_utf8()` 而不是 `1`，否则会落在字符中间导致切片 panic。
fn word_start(before: &str) -> usize {
    before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace() || matches!(c, '|' | '>' | '<' | ';'))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0)
}

impl Completer for ShellHelper {
    type Candidate = String;

    fn complete(&self, line: &str, pos: usize, _ctx: &RlContext<'_>) -> Result<(usize, Vec<String>), ReadlineError> {
        // rustyline 给的 pos 理论上总是字符边界；这里用 get 兜底，
        // 万一边界不合法（多字节字符/宽字符被截断）也不会 panic。
        let before = line.get(..pos).unwrap_or(line);
        let start = word_start(before);
        let word = before.get(start..).unwrap_or("");

        let candidates = if word.starts_with('$') && word.len() > 1 {
            self.complete_env(word.get(1..).unwrap_or(""))
        } else if start == 0 || before.get(..start).unwrap_or("").trim().is_empty() {
            self.complete_command(word)
        } else {
            self.complete_file(word)
        };

        if candidates.is_empty() { Ok((pos, vec![])) } else { Ok((start, candidates)) }
    }
}

impl Highlighter for ShellHelper {
    /// 给纯文本提示符上色。rustyline 保证高亮版本与原 prompt 显示宽度一致，
    /// 因此这里只插入零宽度的 ANSI 颜色码，不会导致光标定位错误。
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        use colored::Colorize;
        // 纯文本格式：user@host:path$ （末尾可能带空格）
        let trimmed = prompt.trim_end();
        let Some(at) = trimmed.find('@') else {
            return Cow::Borrowed(prompt);
        };
        let Some(colon) = trimmed[at + 1..].find(':').map(|i| at + 1 + i) else {
            return Cow::Borrowed(prompt);
        };
        let user = &trimmed[..at];
        let host = &trimmed[at + 1..colon];
        let rest = &trimmed[colon + 1..];
        // 末尾的 $ 或 # 符号
        let sym_pos = rest.rfind(['$', '#']).unwrap_or(rest.len());
        let path = &rest[..sym_pos];
        let sym = &rest[sym_pos..];
        let trailing = &prompt[trimmed.len()..];
        Cow::Owned(format!(
            "{}@{}:{}{}{}",
            user.green().bold(),
            host.cyan().bold(),
            path.blue().bold(),
            sym,
            trailing
        ))
    }
}
impl Hinter for ShellHelper {
    type Hint = String;
}
impl Validator for ShellHelper {}
impl Helper for ShellHelper {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_start_stays_on_char_boundary() {
        let cases = [
            "回显\u{3000}中文",
            "a|b",
            "cmd > out",
            "中文",
            "",
            "😀 cmd",
            "a\u{3000}b",
        ];
        for s in cases {
            let i = word_start(s);
            assert!(s.is_char_boundary(i), "not a char boundary for {s:?}");
            assert!(i <= s.len());
        }
        let line = "回显\u{3000}中文";
        assert_eq!(&line[word_start(line)..], "中文");
    }

    #[test]
    fn complete_file_tolerates_missing_dirs_and_cjk() {
        let helper = ShellHelper {
            cwd: Arc::new(RwLock::new(std::env::temp_dir())),
            env_names: Vec::new(),
            path_commands: Vec::new(),
        };
        // 不存在的目录、中文路径都只应返回空列表，不能 panic。
        assert!(helper.complete_file("不存在的目录/中文").is_empty());
        assert!(helper.complete_file("~/完全不存在的目录/x").is_empty());
    }
}
