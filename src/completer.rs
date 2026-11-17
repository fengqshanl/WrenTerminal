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
}

impl ShellHelper {
    pub fn new(cwd: Arc<RwLock<PathBuf>>, env: &Env) -> Self {
        let mut env_names: Vec<String> = env.all().into_iter().map(|(k, _)| k).collect();
        env_names.sort();
        env_names.dedup();
        Self { cwd, env_names }
    }

    /// 所有可补全的命令名。
    fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = builtins::BUILTINS.iter().map(|b| b.name.to_string()).collect();
        names.extend(["quit", "cls", "ll", "la", "printenv", "egrep", "fgrep"].iter().map(|s| s.to_string()));
        names.extend(translator::known_commands());
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
            Some(i) => (&word[..=i], &word[i + 1..]),
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

impl Completer for ShellHelper {
    type Candidate = String;

    fn complete(&self, line: &str, pos: usize, _ctx: &RlContext<'_>) -> Result<(usize, Vec<String>), ReadlineError> {
        let before = &line[..pos];
        let start = before
            .rfind(|c: char| c.is_whitespace() || c == '|' || c == '>' || c == '<' || c == ';')
            .map(|i| i + 1)
            .unwrap_or(0);
        let word = &before[start..];

        let candidates = if word.starts_with('$') && word.len() > 1 {
            self.complete_env(&word[1..])
        } else if start == 0 || before[..start].trim().is_empty() {
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
