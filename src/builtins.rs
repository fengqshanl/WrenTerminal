//! 用 Rust 原生实现的一组常用 Linux 命令，行为尽量贴近 GNU coreutils。

use crate::env::Env;
use crate::pathutil::{self, to_posix};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 命令执行所需的上下文。
pub struct Context {
    pub env: Env,
    pub cwd: PathBuf,
    pub history: Vec<String>,
    /// 是否给输出着色（管道中间阶段会自动关闭）。
    pub color: bool,
    /// 当前命令是否直接连着终端（决定 apt 等命令能否显示实时进度）。
    pub interactive: bool,
    /// 是否已经提示过 Windows 下 sudo 的语义（每个会话只提示一次）。
    pub sudo_hint_shown: bool,
}

impl Context {
    pub fn new(env: Env) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from(r"C:\"));
        Self {
            env,
            cwd,
            history: Vec::new(),
            color: true,
            interactive: true,
            sudo_hint_shown: false,
        }
    }

    pub fn home(&self) -> PathBuf {
        self.env.home_path()
    }

    pub fn resolve(&self, s: &str) -> PathBuf {
        let home = self.env.home_string();
        pathutil::to_windows(s, &self.cwd, Path::new(&home))
    }
}

/// 控制流：命令是否要求退出 shell。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flow {
    Continue,
    Exit,
}

/// 一条命令的输出。
#[derive(Debug, Clone)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
    pub flow: Flow,
    /// 为 true 时 emit 不再自动补末尾换行（用于 echo -n 等场景）。
    pub raw: bool,
}

impl Output {
    pub fn ok(s: impl Into<String>) -> Self {
        Self {
            stdout: s.into(),
            stderr: String::new(),
            code: 0,
            flow: Flow::Continue,
            raw: false,
        }
    }
    pub fn err(s: impl Into<String>) -> Self {
        Self {
            stdout: String::new(),
            stderr: s.into(),
            code: 1,
            flow: Flow::Continue,
            raw: false,
        }
    }
    pub fn code(code: i32) -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            code,
            flow: Flow::Continue,
            raw: false,
        }
    }
    pub fn exit(code: i32) -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            code,
            flow: Flow::Exit,
            raw: false,
        }
    }
    /// 标记输出为原样（emit 不补换行）。
    pub fn raw(mut self) -> Self {
        self.raw = true;
        self
    }
}

/// 内置命令元信息（用于 help / 补全）。
pub struct BuiltinInfo {
    pub name: &'static str,
    pub usage: &'static str,
    pub desc: &'static str,
}

pub static BUILTINS: &[BuiltinInfo] = &[
    BuiltinInfo { name: "help", usage: "help [命令]", desc: "显示帮助信息" },
    BuiltinInfo { name: "exit", usage: "exit [状态码]", desc: "退出 shell（别名 quit）" },
    BuiltinInfo { name: "clear", usage: "clear", desc: "清屏（别名 cls）" },
    BuiltinInfo { name: "pwd", usage: "pwd", desc: "显示当前目录" },
    BuiltinInfo { name: "cd", usage: "cd [目录|-|~]", desc: "切换目录" },
    BuiltinInfo { name: "ls", usage: "ls [-lah] [路径...]", desc: "列出目录内容（别名 ll/la）" },
    BuiltinInfo { name: "echo", usage: "echo [-n] [内容...]", desc: "输出文本" },
    BuiltinInfo { name: "cat", usage: "cat [-n] [文件...]", desc: "显示文件内容" },
    BuiltinInfo { name: "head", usage: "head [-n N] [文件...]", desc: "显示开头 N 行" },
    BuiltinInfo { name: "tail", usage: "tail [-n N] [文件...]", desc: "显示末尾 N 行" },
    BuiltinInfo { name: "wc", usage: "wc [-lwc] [文件...]", desc: "统计行/词/字节数" },
    BuiltinInfo { name: "grep", usage: "grep [-inv] 模式 [文件...]", desc: "文本搜索" },
    BuiltinInfo { name: "find", usage: "find [路径] -name 模式", desc: "递归查找文件" },
    BuiltinInfo { name: "mkdir", usage: "mkdir [-p] 目录...", desc: "创建目录" },
    BuiltinInfo { name: "rmdir", usage: "rmdir 目录...", desc: "删除空目录" },
    BuiltinInfo { name: "rm", usage: "rm [-rf] 路径...", desc: "删除文件/目录" },
    BuiltinInfo { name: "cp", usage: "cp [-r] 源 目标", desc: "复制文件或目录" },
    BuiltinInfo { name: "mv", usage: "mv 源 目标", desc: "移动或重命名" },
    BuiltinInfo { name: "touch", usage: "touch 文件...", desc: "创建空文件或更新时间" },
    BuiltinInfo { name: "export", usage: "export [NAME=value ...]", desc: "设置环境变量" },
    BuiltinInfo { name: "unset", usage: "unset NAME ...", desc: "删除环境变量" },
    BuiltinInfo { name: "env", usage: "env", desc: "显示环境变量（别名 printenv）" },
    BuiltinInfo { name: "which", usage: "which 命令...", desc: "定位命令位置" },
    BuiltinInfo { name: "history", usage: "history", desc: "显示命令历史" },
    BuiltinInfo { name: "uname", usage: "uname [-a]", desc: "显示系统信息" },
    BuiltinInfo { name: "whoami", usage: "whoami", desc: "显示当前用户" },
    BuiltinInfo { name: "hostname", usage: "hostname", desc: "显示主机名" },
    BuiltinInfo { name: "date", usage: "date", desc: "显示当前日期时间" },
    BuiltinInfo { name: "seq", usage: "seq [起始] 结束", desc: "生成数字序列" },
    BuiltinInfo { name: "true", usage: "true", desc: "返回成功" },
    BuiltinInfo { name: "false", usage: "false", desc: "返回失败" },
    BuiltinInfo { name: "type", usage: "type 命令", desc: "显示命令类型" },
    BuiltinInfo { name: "tree", usage: "tree [路径]", desc: "树形显示目录" },
    BuiltinInfo { name: "sleep", usage: "sleep 秒数", desc: "等待指定秒数" },
    BuiltinInfo { name: "apt", usage: "apt <update|upgrade|install|remove|search|show|list|download> ...", desc: "软件包管理（类似 Ubuntu apt，后端 winget/choco/scoop）" },
    BuiltinInfo { name: "apt-get", usage: "apt-get <命令> ...", desc: "apt 的别名" },
    BuiltinInfo { name: "sudo", usage: "sudo <命令> [参数]", desc: "以当前权限执行命令（Windows 无真正的 sudo）" },
    BuiltinInfo { name: "update", usage: "update [--check] [-y]", desc: "检查并安装 Wren 新版本" },
    BuiltinInfo { name: "man", usage: "man [命令]", desc: "查看内置命令帮助（外部命令可用 --help）" },
    BuiltinInfo { name: "nproc", usage: "nproc", desc: "显示可用 CPU 核心数" },
    BuiltinInfo { name: "free", usage: "free [-h]", desc: "显示内存使用情况" },
    BuiltinInfo { name: "uptime", usage: "uptime [-p]", desc: "显示系统运行时间" },
];

/// 判断命令是否为内置命令。
pub fn is_builtin(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(n.as_str(),
        "help" | "exit" | "quit" | "clear" | "cls" | "pwd" | "cd" | "ls" | "ll" | "la" |
        "echo" | "cat" | "head" | "tail" | "wc" | "grep" | "find" | "mkdir" | "rmdir" |
        "rm" | "cp" | "mv" | "touch" | "export" | "unset" | "env" | "printenv" |
        "which" | "history" | "uname" | "whoami" | "hostname" | "date" | "seq" |
        "true" | "false" | "type" | "tree" | "sleep" | "apt" | "apt-get" | "update"
        | "man" | "nproc" | "free" | "uptime"
    )
}

/// 分发执行一条内置命令。返回 None 表示不是内置命令。
pub fn run(name: &str, args: &[String], ctx: &mut Context, stdin: Option<&str>) -> Option<Output> {
    let n = name.to_ascii_lowercase();
    let out = match n.as_str() {
        "help" => cmd_help(args),
        "exit" | "quit" => cmd_exit(args),
        "clear" | "cls" => Output::ok("\x1b[2J\x1b[H"),
        "pwd" => Output::ok(to_posix(&ctx.cwd)),
        "cd" => cmd_cd(args, ctx),
        "ls" | "ll" | "la" => cmd_ls(args, &n, ctx),
        "echo" => cmd_echo(args),
        "cat" => cmd_cat(args, stdin, ctx),
        "head" => cmd_head_tail(args, stdin, ctx, true),
        "tail" => cmd_head_tail(args, stdin, ctx, false),
        "wc" => cmd_wc(args, stdin, ctx),
        "grep" => cmd_grep(args, stdin, ctx),
        "find" => cmd_find(args, ctx),
        "mkdir" => cmd_mkdir(args, ctx),
        "rmdir" => cmd_rmdir(args, ctx),
        "rm" => cmd_rm(args, ctx),
        "cp" => cmd_cp(args, ctx),
        "mv" => cmd_mv(args, ctx),
        "touch" => cmd_touch(args, ctx),
        "export" => cmd_export(args, ctx),
        "unset" => cmd_unset(args, ctx),
        "env" | "printenv" => cmd_env(args, ctx),
        "which" => cmd_which(args, ctx),
        "history" => Output::ok(
            ctx.history
                .iter()
                .enumerate()
                .map(|(i, h)| format!("{:>5}  {}", i + 1, h))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "uname" => cmd_uname(args, ctx),
        "whoami" => Output::ok(ctx.env.get_or("USER", "user")),
        "hostname" => Output::ok(ctx.env.get_or("COMPUTERNAME", "localhost")),
        "date" => Output::ok(cmd_date()),
        "seq" => cmd_seq(args),
        "true" => Output::code(0),
        "false" => Output::code(1),
        "type" => cmd_type(args, ctx),
        "tree" => cmd_tree(args, ctx),
        "sleep" => cmd_sleep(args),
        "apt" | "apt-get" => crate::pkg::run(&n, args, ctx),
        "update" => crate::update::run(ctx, args),
        "man" => cmd_man(args),
        "nproc" => cmd_nproc(),
        "free" => cmd_free(args),
        "uptime" => cmd_uptime(args),
        _ => return None,
    };
    Some(out)
}

// ---------------------------------------------------------------------------
// 各命令实现
// ---------------------------------------------------------------------------

fn cmd_help(args: &[String]) -> Output {
    if let Some(name) = args.first() {
        let lower = name.to_ascii_lowercase();
        if let Some(info) = BUILTINS.iter().find(|b| b.name == lower) {
            return Output::ok(format!("{} - {}\n用法: {}", info.name, info.desc, info.usage));
        }
        return Output::err(format!("help: 未找到命令 '{}'", name));
    }
    let mut s = String::from("Wren 内置命令：\n\n");
    for b in BUILTINS {
        s.push_str(&format!("  {:<10} {}\n", b.name, b.desc));
    }
    s.push_str("\n提示：\n");
    s.push_str("  - 支持管道 |、重定向 > >> <、连接符 && || ;\n");
    s.push_str("  - 支持变量 $VAR / ${VAR} / $? 与 ~ 展开\n");
    s.push_str("  - 非内置命令会自动翻译为 Windows 等价命令执行\n");
    s.push_str("  - 安装 Git for Windows 后，未内置的 GNU 命令会自动交给 Git Bash 执行\n");
    s.push_str("  - 使用 `help <命令>` 查看具体用法\n");
    Output::ok(s)
}

fn cmd_exit(args: &[String]) -> Output {
    let code = args
        .first()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    Output::exit(code)
}

fn cmd_cd(args: &[String], ctx: &mut Context) -> Output {
    let target = if args.is_empty() {
        ctx.env.get_or("HOME", r"C:\Users\Default")
    } else if args[0] == "-" {
        let home = ctx.env.get_or("HOME", r"C:\Users\Default");
        ctx.env.get_or("OLDPWD", &home)
    } else {
        args[0].clone()
    };
    let path = ctx.resolve(&target);
    match fs::metadata(&path) {
        Ok(m) if m.is_dir() => {
            let old = ctx.cwd.to_string_lossy().to_string();
            ctx.cwd = path;
            ctx.env.set("OLDPWD", &old);
            ctx.env.set("PWD", &ctx.cwd.to_string_lossy());
            if args.first().map(|s| s.as_str()) == Some("-") {
                Output::ok(to_posix(&ctx.cwd))
            } else {
                Output::ok("")
            }
        }
        Ok(_) => Output::err(format!("cd: {}: 不是目录", target)),
        Err(e) => Output::err(format!("cd: {}: {}", target, e)),
    }
}

fn cmd_echo(args: &[String]) -> Output {
    let mut no_newline = false;
    let mut interpret = false;
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') && args[i].len() > 1 {
        let flags = &args[i][1..];
        if flags.contains('n') { no_newline = true; }
        if flags.contains('e') { interpret = true; }
        if !flags.chars().all(|c| c == 'n' || c == 'e') { break; }
        i += 1;
    }
    let mut text = args[i..].join(" ");
    if interpret {
        text = text
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace("\\r", "\r")
            .replace("\\\\", "\\");
    }
    if no_newline {
        Output::ok(text).raw()
    } else {
        text.push('\n');
        Output::ok(text)
    }
}

fn cmd_ls(args: &[String], name: &str, ctx: &Context) -> Output {
    let mut all = name == "la";
    let mut long = name == "ll";
    let mut human = false;
    let mut reverse = false;
    let mut one_per_line = false;
    let mut paths: Vec<String> = Vec::new();

    for a in args {
        if a.starts_with('-') && a.len() > 1 {
            for c in a[1..].chars() {
                match c {
                    'a' => all = true,
                    'l' => long = true,
                    'h' => human = true,
                    'r' => reverse = true,
                    '1' => one_per_line = true,
                    _ => {}
                }
            }
            if a.contains("all") { all = true; }
        } else {
            paths.push(a.clone());
        }
    }
    if paths.is_empty() { paths.push(".".into()); }

    let mut result = String::new();
    let multi = paths.len() > 1;
    for (idx, p) in paths.iter().enumerate() {
        let dir = ctx.resolve(p);
        let meta = match fs::metadata(&dir) {
            Ok(m) => m,
            Err(e) => {
                result.push_str(&format!("ls: 无法访问 '{}': {}\n", p, e));
                continue;
            }
        };
        if multi {
            if idx > 0 && !result.ends_with('\n') { result.push('\n'); }
            result.push_str(&format!("{}:\n", p));
        }
        if meta.is_file() {
            result.push_str(&format_entry(&dir, long, human, ctx.color));
            result.push('\n');
            continue;
        }

        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                result.push_str(&format!("ls: 无法打开 '{}': {}\n", p, e));
                continue;
            }
        };
        let mut names: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if !all && fname.starts_with('.') { continue; }
            names.push((fname, entry.path()));
        }
        names.sort_by(|a, b| a.0.cmp(&b.0));
        if reverse { names.reverse(); }

        if long {
            for (fname, fpath) in &names {
                result.push_str(&format_entry(fpath, true, human, ctx.color));
                result.push(' ');
                result.push_str(&color_name(fname, fpath, ".", ctx.color));
                result.push('\n');
            }
        } else if one_per_line {
            for (fname, fpath) in &names {
                result.push_str(&color_name(fname, fpath, ".", ctx.color));
                result.push('\n');
            }
        } else {
            let maxw = names.iter().map(|(n, _)| display_width(n)).max().unwrap_or(0);
            let colw = maxw + 2;
            let cols = if colw > 0 { (80 / colw).max(1) } else { 1 };
            for (i, (fname, fpath)) in names.iter().enumerate() {
                let cell = color_name(fname, fpath, ".", ctx.color);
                let pad = colw.saturating_sub(display_width(fname));
                if i + 1 == names.len() || (i + 1) % cols == 0 {
                    result.push_str(&cell);
                    result.push('\n');
                } else {
                    result.push_str(&cell);
                    result.push_str(&" ".repeat(pad));
                }
            }
        }
    }
    Output::ok(result)
}

fn color_name(name: &str, path: &Path, _dot: &str, color: bool) -> String {
    if !color { return name.to_string(); }
    let is_dir = fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false);
    if is_dir {
        format!("{}", name.blue().bold())
    } else {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".bat") || lower.ends_with(".cmd")
            || lower.ends_with(".ps1") || lower.ends_with(".sh") || lower.ends_with(".py")
        {
            format!("{}", name.green())
        } else {
            name.to_string()
        }
    }
}

fn format_entry(path: &Path, long: bool, human: bool, color: bool) -> String {
    if !long {
        return String::new();
    }
    let meta = match fs::metadata(path) { Ok(m) => m, Err(_) => return String::new() };
    let is_dir = meta.is_dir();
    let perm = if is_dir { "drwxr-xr-x" } else { "-rw-r--r--" };
    let size = if human { human_size(meta.len()) } else { meta.len().to_string() };
    let time = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| format_ls_time(d.as_secs() as i64))
        .unwrap_or_else(|| "Jan  1 00:00".into());
    let p = if color && is_dir {
        format!("{}", perm.blue())
    } else {
        perm.to_string()
    };
    format!("{} {:>8} {} ", p, size, time)
}

fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// 终端显示宽度：CJK 字符和常见 emoji 占两列。
fn char_width(c: char) -> usize {
    let u = c as u32;
    if matches!(
        u,
        0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1FAFF
            | 0x20000..=0x3FFFD
    ) {
        2
    } else {
        1
    }
}

fn human_size(n: u64) -> String {
    const UNITS: &[&str] = &["", "K", "M", "G", "T"];
    let mut val = n as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{}", n) } else { format!("{:.1}{}", val, UNITS[unit]) }
}

/// 读取若干文件内容（`-` 或缺失表示 stdin）。
fn read_sources(files: &[String], stdin: Option<&str>, ctx: &Context) -> Result<Vec<(String, String)>, String> {
    let targets: Vec<String> = if files.is_empty() { vec!["-".into()] } else { files.to_vec() };
    let mut out = Vec::new();
    for f in targets {
        if f == "-" {
            out.push(("<stdin>".into(), stdin.unwrap_or("").to_string()));
        } else {
            let p = ctx.resolve(&f);
            match fs::read_to_string(&p) {
                Ok(s) => out.push((f, s)),
                Err(e) => return Err(format!("{}: {}", f, e)),
            }
        }
    }
    Ok(out)
}

fn split_lines_keep(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split_inclusive('\n').map(|l| l.to_string()).collect();
    if v.is_empty() { v.push(String::new()); }
    v
}

fn cmd_cat(args: &[String], stdin: Option<&str>, ctx: &Context) -> Output {
    let mut number = false;
    let mut files = Vec::new();
    for a in args {
        if a == "-n" || a == "--number" { number = true; }
        else { files.push(a.clone()); }
    }
    match read_sources(&files, stdin, ctx) {
        Ok(sources) => {
            let mut result = String::new();
            let mut line_no = 1;
            for (_, text) in sources {
                for line in split_lines_keep(&text) {
                    if number {
                        result.push_str(&format!("{:>6}\t", line_no));
                        line_no += 1;
                    }
                    result.push_str(&line);
                }
            }
            Output::ok(result)
        }
        Err(e) => Output::err(format!("cat: {}", e)),
    }
}

fn cmd_head_tail(args: &[String], stdin: Option<&str>, ctx: &Context, head: bool) -> Output {
    let cmd = if head { "head" } else { "tail" };
    let mut count: usize = 10;
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-n" && i + 1 < args.len() {
            match args[i + 1].parse::<usize>() {
                Ok(n) => count = n,
                Err(_) => return Output::err(format!("{}: 无效的行数 '{}'", cmd, args[i + 1])),
            }
            i += 2;
        } else if args[i].starts_with("-n") && args[i].len() > 2 {
            match args[i][2..].parse::<usize>() {
                Ok(n) => count = n,
                Err(_) => return Output::err(format!("{}: 无效的行数", cmd)),
            }
            i += 1;
        } else {
            files.push(args[i].clone());
            i += 1;
        }
    }
    match read_sources(&files, stdin, ctx) {
        Ok(sources) => {
            let mut result = String::new();
            for (idx, (_, text)) in sources.iter().enumerate() {
                let lines = split_lines_keep(text);
                let chosen: Vec<&String> = if head {
                    lines.iter().take(count).collect()
                } else {
                    let start = lines.len().saturating_sub(count);
                    lines.iter().skip(start).collect()
                };
                if sources.len() > 1 {
                    if idx > 0 { result.push('\n'); }
                    result.push_str(&format!("==> {} <==\n", files.get(idx).cloned().unwrap_or_default()));
                }
                for line in chosen { result.push_str(line); }
            }
            Output::ok(result)
        }
        Err(e) => Output::err(format!("{}: {}", cmd, e)),
    }
}

fn cmd_wc(args: &[String], stdin: Option<&str>, ctx: &Context) -> Output {
    let mut show_lines = false;
    let mut show_words = false;
    let mut show_bytes = false;
    for a in args {
        if a.starts_with('-') {
            for c in a[1..].chars() {
                match c {
                    'l' => show_lines = true,
                    'w' => show_words = true,
                    'c' => show_bytes = true,
                    _ => {}
                }
            }
        }
    }
    if !show_lines && !show_words && !show_bytes {
        show_lines = true;
        show_words = true;
        show_bytes = true;
    }
    let files: Vec<String> = args.iter().filter(|a| !a.starts_with('-')).cloned().collect();
    match read_sources(&files, stdin, ctx) {
        Ok(sources) => {
            let mut result = String::new();
            let mut totals = (0usize, 0usize, 0usize);
            for (name, text) in &sources {
                let lines = text.lines().count();
                let words = text.split_whitespace().count();
                let bytes = text.as_bytes().len();
                totals.0 += lines; totals.1 += words; totals.2 += bytes;
                let mut parts = Vec::new();
                if show_lines { parts.push(format!("{:>6}", lines)); }
                if show_words { parts.push(format!("{:>6}", words)); }
                if show_bytes { parts.push(format!("{:>6}", bytes)); }
                parts.push(name.clone());
                result.push_str(&parts.join(" "));
                result.push('\n');
            }
            if sources.len() > 1 {
                let mut parts = Vec::new();
                if show_lines { parts.push(format!("{:>6}", totals.0)); }
                if show_words { parts.push(format!("{:>6}", totals.1)); }
                if show_bytes { parts.push(format!("{:>6}", totals.2)); }
                parts.push("total".into());
                result.push_str(&parts.join(" "));
                result.push('\n');
            }
            Output::ok(result)
        }
        Err(e) => Output::err(format!("wc: {}", e)),
    }
}

fn cmd_grep(args: &[String], stdin: Option<&str>, ctx: &Context) -> Output {
    let mut ignore_case = false;
    let mut invert = false;
    let mut line_number = false;
    let mut only_matching = false;
    let mut pattern: Option<String> = None;
    let mut files = Vec::new();
    for a in args {
        if a.starts_with('-') && a.len() > 1 {
            for c in a[1..].chars() {
                match c {
                    'i' => ignore_case = true,
                    'v' => invert = true,
                    'n' => line_number = true,
                    'o' => only_matching = true,
                    _ => {}
                }
            }
        } else if pattern.is_none() {
            pattern = Some(a.clone());
        } else {
            files.push(a.clone());
        }
    }
    let pattern = match pattern {
        Some(p) => p,
        None => return Output::err("grep: 缺少搜索模式"),
    };
    let needle = if ignore_case { pattern.to_ascii_lowercase() } else { pattern.clone() };
    match read_sources(&files, stdin, ctx) {
        Ok(sources) => {
            let mut result = String::new();
            let multi = sources.len() > 1;
            for (name, text) in &sources {
                for (i, line) in text.lines().enumerate() {
                    let hay = if ignore_case { line.to_ascii_lowercase() } else { line.to_string() };
                    let matched = hay.contains(&needle);
                    if matched != invert {
                        if multi { result.push_str(name); result.push(':'); }
                        if line_number { result.push_str(&format!("{}:", i + 1)); }
                        if only_matching {
                            // 输出匹配到的子串（可多次出现）
                            let mut pos = 0;
                            while let Some(idx) = hay[pos..].find(&needle) {
                                let start = pos + idx;
                                result.push_str(&line[start..start + needle.len()]);
                                result.push('\n');
                                pos = start + needle.len().max(1);
                            }
                            if !hay.contains(&needle) { result.push_str(line); result.push('\n'); }
                        } else {
                            result.push_str(line);
                            result.push('\n');
                        }
                    }
                }
            }
            Output::ok(result)
        }
        Err(e) => Output::err(format!("grep: {}", e)),
    }
}

fn cmd_find(args: &[String], ctx: &Context) -> Output {
    let mut root = ".".to_string();
    let mut pattern: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-name" || args[i] == "-iname" {
            if i + 1 < args.len() { pattern = Some(args[i + 1].clone()); i += 2; continue; }
        }
        if args[i] == "-type" { i += 2; continue; }
        if !args[i].starts_with('-') && pattern.is_none() { root = args[i].clone(); i += 1; continue; }
        i += 1;
    }
    let pat = match pattern { Some(p) => p, None => "*".to_string() };
    let start = ctx.resolve(&root);
    let mut result = String::new();
    walk_find(&start, &pat, &mut result);
    Output::ok(result)
}

fn walk_find(dir: &Path, pattern: &str, out: &mut String) {
    let entries = match fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if glob_match(&name, pattern) {
            out.push_str(&to_posix(&path));
            out.push('\n');
        }
        if path.is_dir() { walk_find(&path, pattern, out); }
    }
}

/// 简单 glob：`*` 匹配任意字符，`?` 匹配一个字符。
fn glob_match(text: &str, pat: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    fn go(t: &[char], p: &[char]) -> bool {
        if p.is_empty() { return t.is_empty(); }
        match p[0] {
            '*' => go(t, &p[1..]) || (!t.is_empty() && go(&t[1..], p)),
            '?' => !t.is_empty() && go(&t[1..], &p[1..]),
            c => !t.is_empty() && t[0] == c && go(&t[1..], &p[1..]),
        }
    }
    go(&t, &p)
}

fn cmd_mkdir(args: &[String], ctx: &Context) -> Output {
    let mut parents = false;
    let mut paths = Vec::new();
    for a in args {
        if a == "-p" || a == "--parents" { parents = true; }
        else { paths.push(a.clone()); }
    }
    if paths.is_empty() { return Output::err("mkdir: 缺少目录名"); }
    for p in &paths {
        let path = ctx.resolve(p);
        let res = if parents { fs::create_dir_all(&path) } else { fs::create_dir(&path) };
        if let Err(e) = res { return Output::err(format!("mkdir: 无法创建 '{}': {}", p, e)); }
    }
    Output::ok("")
}

fn cmd_rmdir(args: &[String], ctx: &Context) -> Output {
    if args.is_empty() { return Output::err("rmdir: 缺少目录名"); }
    for p in args {
        let path = ctx.resolve(p);
        if let Err(e) = fs::remove_dir(&path) {
            return Output::err(format!("rmdir: 无法删除 '{}': {}", p, e));
        }
    }
    Output::ok("")
}

fn cmd_rm(args: &[String], ctx: &Context) -> Output {
    let mut recursive = false;
    let mut force = false;
    let mut paths = Vec::new();
    for a in args {
        if a.starts_with('-') {
            for c in a[1..].chars() {
                match c { 'r' | 'R' => recursive = true, 'f' => force = true, _ => {} }
            }
        } else { paths.push(a.clone()); }
    }
    if paths.is_empty() { return Output::err("rm: 缺少操作数"); }
    for p in &paths {
        let path = ctx.resolve(p);
        // 安全护栏：禁止递归删除根目录或 home 根目录
        if recursive && (path == ctx.cwd || path == ctx.home()) {
            return Output::err(format!("rm: 拒绝递归删除 '{}'（根目录保护）", p));
        }
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                if force { continue; }
                return Output::err(format!("rm: 无法访问 '{}': {}", p, e));
            }
        };
        let res = if meta.is_dir() {
            if recursive { fs::remove_dir_all(&path) } else {
                return Output::err(format!("rm: 无法删除 '{}': 是目录（使用 -r）", p));
            }
        } else {
            fs::remove_file(&path)
        };
        if let Err(e) = res {
            if !force { return Output::err(format!("rm: 无法删除 '{}': {}", p, e)); }
        }
    }
    Output::ok("")
}

fn cmd_cp(args: &[String], ctx: &Context) -> Output {
    let mut recursive = false;
    let mut items: Vec<String> = args.iter().filter(|a| {
        if a.starts_with('-') { for c in a[1..].chars() { if c == 'r' || c == 'R' { recursive = true; } } false }
        else { true }
    }).cloned().collect();
    if items.len() < 2 { return Output::err("cp: 用法: cp [-r] 源 目标"); }
    let dst = items.pop().unwrap();
    let dst_path = ctx.resolve(&dst);
    let multi = items.len() > 1;
    for src in items {
        let src_path = ctx.resolve(&src);
        let target = if multi || dst_path.is_dir() {
            let name = src_path.file_name().unwrap_or_default();
            dst_path.join(name)
        } else { dst_path.clone() };
        if let Err(e) = copy_path(&src_path, &target, recursive) {
            return Output::err(format!("cp: '{}' -> '{}': {}", src, dst, e));
        }
    }
    Output::ok("")
}

fn copy_path(src: &Path, dst: &Path, recursive: bool) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        if !recursive { return Err(std::io::Error::new(std::io::ErrorKind::Other, "是目录（使用 -r）")); }
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let name = entry.file_name();
            copy_path(&entry.path(), &dst.join(name), recursive)?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() { fs::create_dir_all(parent)?; }
        fs::copy(src, dst).map(|_| ())
    }
}

fn cmd_mv(args: &[String], ctx: &Context) -> Output {
    if args.len() < 2 { return Output::err("mv: 用法: mv 源 目标"); }
    let dst = args.last().unwrap();
    let dst_path = ctx.resolve(dst);
    let multi = args.len() > 2;
    for src in &args[..args.len() - 1] {
        let src_path = ctx.resolve(src);
        let target = if multi || dst_path.is_dir() {
            dst_path.join(src_path.file_name().unwrap_or_default())
        } else { dst_path.clone() };
        if let Err(e) = fs::rename(&src_path, &target) {
            // 跨盘符时回退为复制+删除
            if let Err(e2) = copy_path(&src_path, &target, true) {
                return Output::err(format!("mv: '{}' -> '{}': {} / {}", src, dst, e, e2));
            }
            let _ = if src_path.is_dir() { fs::remove_dir_all(&src_path) } else { fs::remove_file(&src_path) };
        }
    }
    Output::ok("")
}

fn cmd_touch(args: &[String], ctx: &Context) -> Output {
    if args.is_empty() { return Output::err("touch: 缺少文件名"); }
    for a in args {
        let path = ctx.resolve(a);
        match fs::OpenOptions::new().write(true).create(true).open(&path) {
            Ok(f) => {
                let _ = f.set_modified(SystemTime::now());
            }
            Err(e) => return Output::err(format!("touch: '{}': {}", a, e)),
        }
    }
    Output::ok("")
}

fn cmd_export(args: &[String], ctx: &mut Context) -> Output {
    if args.is_empty() {
        let mut vars: Vec<(String, String)> = ctx.env.all();
        vars.retain(|(k, _)| !k.starts_with('_'));
        let s = vars.iter()
            .map(|(k, v)| format!("declare -x {}=\"{}\"", k, v.replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join("\n");
        return Output::ok(s);
    }
    for a in args {
        if let Some((k, v)) = a.split_once('=') {
            ctx.env.export(k, v);
        } else if let Some(v) = ctx.env.get(a) {
            ctx.env.export(a, &v);
        } else {
            return Output::err(format!("export: '{}': 未定义", a));
        }
    }
    Output::ok("")
}

fn cmd_unset(args: &[String], ctx: &mut Context) -> Output {
    for a in args { ctx.env.unset(a); }
    Output::ok("")
}

fn cmd_env(args: &[String], ctx: &Context) -> Output {
    if let Some(name) = args.first() {
        return match ctx.env.get(name) {
            Some(v) => Output::ok(format!("{}\n", v)),
            None => Output::code(1),
        };
    }
    Output::ok(ctx.env.all().iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join("\n"))
}

fn cmd_which(args: &[String], ctx: &Context) -> Output {
    if args.is_empty() { return Output::err("which: 缺少命令名"); }
    let mut result = String::new();
    let mut missing = false;
    let path = ctx.env.get_or("PATH", "");
    for name in args {
        if is_builtin(name) || name.eq_ignore_ascii_case("sudo") {
            result.push_str(&format!("{}: shell 内置命令\n", name));
            continue;
        }
        let mut found = false;
        for dir in crate::winutil::split_path(&path) {
            let dir = dir.trim().trim_matches('"');
            if dir.is_empty() {
                continue;
            }
            let d = ctx.resolve(dir);
            for ext in [".exe", ".cmd", ".bat", ".com", ""] {
                let candidate = d.join(format!("{}{}", name, ext));
                if candidate.is_file() {
                    result.push_str(&to_posix(&candidate));
                    result.push('\n');
                    found = true;
                    break;
                }
            }
            if found { break; }
        }
        if !found { missing = true; }
    }
    if missing && result.is_empty() { Output::code(1) } else { Output::ok(result) }
}

fn cmd_uname(args: &[String], ctx: &Context) -> Output {
    let all = args.iter().any(|a| a == "-a" || a == "--all");
    let release = concat!(env!("CARGO_PKG_VERSION"), "-Wren");
    let machine = if std::env::consts::ARCH == "x86_64" { "x86_64" } else { std::env::consts::ARCH };
    if all {
        Output::ok(format!(
            "Linux {} {} {} GNU/Linux",
            ctx.env.get_or("COMPUTERNAME", "host"), release, machine
        ))
    } else if args.iter().any(|a| a == "-r") {
        Output::ok(release)
    } else if args.iter().any(|a| a == "-n") {
        Output::ok(ctx.env.get_or("COMPUTERNAME", "host"))
    } else if args.iter().any(|a| a == "-m") {
        Output::ok(machine)
    } else {
        Output::ok("Linux")
    }
}

fn cmd_man(args: &[String]) -> Output {
    let Some(name) = args.first() else {
        return cmd_help(&[]);
    };
    let lower = name.to_ascii_lowercase();
    if let Some(info) = BUILTINS.iter().find(|b| b.name == lower) {
        return Output::ok(format!(
            "{} - {}\n用法: {}",
            info.name, info.desc, info.usage
        ));
    }
    Output::ok(format!(
        "{} 不是 Wren 内置命令。\n可运行 `{} --help` 查看外部工具帮助（Git Bash 提供的 GNU 工具通常支持 --help）。",
        name, name
    ))
}

fn cmd_nproc() -> Output {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    Output::ok(n.to_string())
}

#[repr(C)]
#[allow(dead_code)]
struct MemoryStatusEx {
    length: u32,
    memory_load: u32,
    total_phys: u64,
    avail_phys: u64,
    total_page_file: u64,
    avail_page_file: u64,
    total_virtual: u64,
    avail_virtual: u64,
    avail_extended_virtual: u64,
}

#[cfg(windows)]
fn memory_status() -> Option<MemoryStatusEx> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }
    unsafe {
        let mut status: MemoryStatusEx = std::mem::zeroed();
        status.length = std::mem::size_of::<MemoryStatusEx>() as u32;
        if GlobalMemoryStatusEx(&mut status) != 0 {
            Some(status)
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn memory_status() -> Option<MemoryStatusEx> {
    None
}

fn cmd_free(args: &[String]) -> Output {
    let human = args.iter().any(|a| a == "-h" || a == "--human");
    let Some(status) = memory_status() else {
        return Output::err("free: 无法读取 Windows 内存信息");
    };
    let total = status.total_phys;
    let available = status.avail_phys;
    let used = total.saturating_sub(available);
    let swap_total = status.total_page_file.saturating_sub(total);
    let swap_available = status
        .avail_page_file
        .saturating_sub(available)
        .min(swap_total);
    let swap_used = swap_total.saturating_sub(swap_available);

    let fmt = |bytes: u64| {
        if human {
            human_size(bytes)
        } else {
            (bytes / 1024).to_string()
        }
    };
    let header =
        "               total        used        free      shared  buff/cache   available";
    let mem = format!(
        "Mem:    {:>12} {:>11} {:>11} {:>12} {:>11} {:>11}",
        fmt(total),
        fmt(used),
        fmt(available),
        "0",
        "0",
        fmt(available)
    );
    let swap = format!(
        "Swap:   {:>12} {:>11} {:>11}",
        fmt(swap_total),
        fmt(swap_used),
        fmt(swap_available)
    );
    Output::ok(format!("{}\n{}\n{}", header, mem, swap))
}

#[cfg(windows)]
fn uptime_seconds() -> u64 {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount64() -> u64;
    }
    unsafe { GetTickCount64() / 1000 }
}

#[cfg(not(windows))]
fn uptime_seconds() -> u64 {
    0
}

fn cmd_uptime(args: &[String]) -> Output {
    let secs = uptime_seconds();
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;

    if args.iter().any(|a| a == "-p" || a == "--pretty") {
        let mut parts = Vec::new();
        if days > 0 {
            parts.push(format!("{} day{}", days, if days == 1 { "" } else { "s" }));
        }
        if hours > 0 {
            parts.push(format!(
                "{} hour{}",
                hours,
                if hours == 1 { "" } else { "s" }
            ));
        }
        if minutes > 0 || parts.is_empty() {
            parts.push(format!(
                "{} minute{}",
                minutes,
                if minutes == 1 { "" } else { "s" }
            ));
        }
        return Output::ok(format!("up {}", parts.join(", ")));
    }

    let up = if days > 0 {
        format!(
            "{} day{}, {:02}:{:02}",
            days,
            if days == 1 { "" } else { "s" },
            hours,
            minutes
        )
    } else {
        format!("{:02}:{:02}", hours, minutes)
    };
    Output::ok(format!(
        " {} up {}, 1 user, load average: 0.00, 0.00, 0.00",
        cmd_date(),
        up
    ))
}

fn cmd_seq(args: &[String]) -> Output {
    let nums: Vec<i64> = args.iter().filter_map(|s| s.parse::<i64>().ok()).collect();
    let (start, step, end) = match nums.len() {
        1 => (1, 1, nums[0]),
        2 => (nums[0], 1, nums[1]),
        3 => (nums[0], nums[1], nums[2]),
        _ => return Output::err("seq: 用法: seq [起始] [步长] 结束"),
    };
    let mut result = String::new();
    if step == 0 { return Output::err("seq: 步长不能为 0"); }
    let mut v = start;
    if step > 0 {
        while v <= end { result.push_str(&format!("{}\n", v)); v += step; }
    } else {
        while v >= end { result.push_str(&format!("{}\n", v)); v += step; }
    }
    Output::ok(result)
}

fn cmd_type(args: &[String], ctx: &Context) -> Output {
    let name = match args.first() { Some(n) => n, None => return Output::err("type: 缺少命令名") };
    if is_builtin(name) || name.eq_ignore_ascii_case("sudo") {
        let lower = name.to_ascii_lowercase();
        let gnu = matches!(
            lower.as_str(),
            "grep" | "find" | "head" | "tail" | "wc" | "cat"
        ) && crate::winutil::find_git_bash(&ctx.env.get_or("PATH", "")).is_some();
        if gnu {
            Output::ok(format!(
                "{} 是 shell 内置命令（检测到 Git Bash 时将优先使用 GNU 版本）",
                name
            ))
        } else {
            Output::ok(format!("{} 是 shell 内置命令", name))
        }
    } else {
        let t = super::translator::translate(name, &[]);
        Output::ok(format!("{} 是外部命令（将执行 {}）", name, t.program))
    }
}

fn cmd_tree(args: &[String], ctx: &Context) -> Output {
    let root_arg = args.first().cloned().unwrap_or_else(|| ".".into());
    let root = ctx.resolve(&root_arg);
    let mut result = String::new();
    result.push_str(&to_posix(&root));
    result.push('\n');
    tree_walk(&root, "", &mut result, 0);
    Output::ok(result)
}

fn tree_walk(dir: &Path, prefix: &str, out: &mut String, depth: usize) {
    if depth > 20 { return; }
    let entries: Vec<_> = match fs::read_dir(dir) {
        Ok(e) => e.flatten().collect(),
        Err(_) => return,
    };
    for (i, entry) in entries.iter().enumerate() {
        let last = i + 1 == entries.len();
        let branch = if last { "└── " } else { "├── " };
        let name = entry.file_name().to_string_lossy().to_string();
        out.push_str(prefix);
        out.push_str(branch);
        out.push_str(&name);
        out.push('\n');
        if entry.path().is_dir() {
            let next = format!("{}{}", prefix, if last { "    " } else { "│   " });
            tree_walk(&entry.path(), &next, out, depth + 1);
        }
    }
}

fn cmd_sleep(args: &[String]) -> Output {
    let secs = args.first().and_then(|s| s.parse::<f64>().ok()).unwrap_or(1.0);
    std::thread::sleep(std::time::Duration::from_secs_f64(secs.max(0.0)));
    Output::ok("")
}

// ---------------------------------------------------------------------------
// 时间格式化（无外部依赖）
// ---------------------------------------------------------------------------

/// Howard Hinnant 的 civil_from_days 算法：days since 1970-01-01 -> (年,月,日)。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn format_ls_time(secs: i64) -> String {
    const MONTHS: &[&str] = &["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (_, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    format!("{} {:>2} {:02}:{:02}", MONTHS[(m - 1) as usize], d, hh, mm)
}

fn cmd_date() -> String {
    const WD: &[&str] = &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: &[&str] = &["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let secs = now_unix();
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let wd = WD[((days + 4).rem_euclid(7)) as usize];
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    format!("{} {} {:>2} {:02}:{:02}:{:02} {} {}", wd, MONTHS[(m - 1) as usize], d, hh, mm, ss, y, "CST")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_works() {
        assert!(glob_match("main.rs", "*.rs"));
        assert!(glob_match("a.txt", "?.txt"));
        assert!(!glob_match("main.rs", "*.txt"));
    }

    #[test]
    fn echo_flags() {
        let out = cmd_echo(&["-n".into(), "hello".into(), "world".into()]);
        assert_eq!(out.stdout, "hello world");
    }

    #[test]
    fn date_format() {
        let s = cmd_date();
        assert!(s.contains(':'));
    }

    #[test]
    fn nproc_is_positive() {
        let out = cmd_nproc();
        assert!(out.stdout.parse::<usize>().unwrap() > 0);
    }

    #[test]
    fn display_width_counts_cjk_as_two() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("a中b"), 4);
    }

    #[cfg(windows)]
    #[test]
    fn free_reports_memory() {
        let out = cmd_free(&[]);
        assert_eq!(out.code, 0);
        assert!(out.stdout.contains("Mem:"));
        assert!(out.stdout.contains("Swap:"));
    }

    #[cfg(windows)]
    #[test]
    fn uptime_reports_uptime() {
        let out = cmd_uptime(&[]);
        assert!(out.stdout.contains(" up "));
    }
}
