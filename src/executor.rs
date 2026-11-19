//! 执行引擎：串联内置命令与外部进程，支持管道、输入输出重定向与连接符。

use crate::builtins::{self, Context, Flow, Output};
use crate::parser::{Conn, ParsedLine, Pipeline, Stage};
use crate::pathutil;
use crate::translator;
use crate::winutil;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const NOT_FOUND_HINT: &str =
    "（可尝试 `help`；安装 Git for Windows 后可获得 sed/awk/sort/tar 等 GNU 工具）";

/// 去掉 parser 给引号内通配符加的保护标记。
fn unescape(s: &str) -> String {
    s.replace(crate::parser::GLOB_ESCAPE, "")
}

/// 执行一整行（可能包含多条用 ; && || 连接的命令）。
pub fn execute_line(line: &ParsedLine, ctx: &mut Context) -> Flow {
    for (conn, pipeline) in &line.pipelines {
        match conn {
            Conn::Always => {}
            Conn::And if ctx.env.last_status() != 0 => continue,
            Conn::Or if ctx.env.last_status() == 0 => continue,
            _ => {}
        }
        let out = run_pipeline(pipeline, ctx);
        let code = out.code;
        let flow = out.flow;
        emit(&out);
        ctx.env.set_last_status(code);
        if flow == Flow::Exit {
            return Flow::Exit;
        }
    }
    Flow::Continue
}

/// 把输出写到终端（调用方负责在写文件场景跳过）。
fn emit(out: &Output) {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    if !out.stdout.is_empty() {
        let _ = stdout.write_all(out.stdout.as_bytes());
        // 命令未按原样输出且缺少末尾换行时补一个，避免与提示符粘连。
        if !out.raw && !out.stdout.ends_with('\n') {
            let _ = stdout.write_all(b"\n");
        }
        let _ = stdout.flush();
    }
    if !out.stderr.is_empty() {
        let _ = stderr.write_all(out.stderr.as_bytes());
        if !out.stderr.ends_with('\n') {
            let _ = stderr.write_all(b"\n");
        }
        let _ = stderr.flush();
    }
}

fn run_pipeline(pipeline: &Pipeline, ctx: &mut Context) -> Output {
    if pipeline.stages.is_empty() {
        return Output::ok("");
    }

    // 先展开 sudo 前缀：`sudo <命令>` 在当前 shell 内以当前权限执行，
    // 不再像旧实现那样启动一个空的 cmd.exe。
    // 然后展开通配符（`*.rs`、`src/*` 等），与 bash 一致。
    let mut effective_stages: Vec<Stage> = Vec::with_capacity(pipeline.stages.len());
    let status = ctx.env.last_status().to_string();
    for stage in &pipeline.stages {
        match rewrite_sudo(stage, ctx) {
            SudoRewrite::Stage(mut s) => {
                // 填入 `$?` 的真实退出码（解析时只放了占位符）。
                for arg in s.argv.iter_mut() {
                    if arg.contains(crate::env::STATUS_PLACEHOLDER) {
                        *arg = arg.replace(crate::env::STATUS_PLACEHOLDER, &status);
                    }
                }
                s.argv = crate::globber::expand_args(&s.argv, &ctx.cwd);
                s.redirect.input = s.redirect.input.take().map(|f| unescape(&f));
                s.redirect.output = s.redirect.output.take().map(|f| unescape(&f));
                s.redirect.stderr = s.redirect.stderr.take().map(|f| unescape(&f));
                effective_stages.push(s);
            }
            SudoRewrite::Done(out) => return out,
        }
    }
    let stages = &effective_stages;

    let single_external = stages.len() == 1
        && !builtin_active(&stages[0].argv[0], ctx)
        && stages[0].redirect.input.is_none()
        && stages[0].redirect.output.is_none()
        && stages[0].redirect.stderr.is_none()
        && !stages[0].redirect.merge_stderr;

    // 单个外部命令且无重定向：继承终端 stdio，保证交互式程序可用。
    if single_external {
        return run_interactive(&stages[0], ctx);
    }

    let mut stdin_data: Option<String> = None;
    let mut accumulated_err = String::new();
    let mut last = Output::ok("");

    for (i, stage) in stages.iter().enumerate() {
        // 输入重定向（仅对第一阶段生效）
        if i == 0 {
            if let Some(infile) = &stage.redirect.input {
                let path = ctx.resolve(infile);
                match fs::read_to_string(&path) {
                    Ok(s) => stdin_data = Some(s),
                    Err(e) => {
                        return Output::err(format!("{}: {}", infile, e));
                    }
                }
            }
        }

        // 管道中间阶段关闭颜色，避免 ANSI 进入下游
        ctx.color = i + 1 == stages.len() && stage.redirect.output.is_none();
        // 只有“单条内置命令 + 无重定向 + 真正的终端”才允许交互式输出/进度条。
        ctx.interactive = stages.len() == 1
            && stage.redirect.input.is_none()
            && stage.redirect.output.is_none()
            && stage.redirect.stderr.is_none()
            && !stage.redirect.merge_stderr
            && std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal();

        let mut out = if builtin_active(&stage.argv[0], ctx) {
            builtins::run(&stage.argv[0], &stage.argv[1..], ctx, stdin_data.as_deref())
                .unwrap_or_else(|| Output::err(format!("未知内置命令: {}", stage.argv[0])))
        } else {
            run_captured(stage, ctx, stdin_data.as_deref())
        };

        // `2>&1`：把标准错误并入标准输出，随后统一走 stdout 的重定向/管道。
        if stage.redirect.merge_stderr && !out.stderr.is_empty() {
            if !out.stdout.is_empty() && !out.stdout.ends_with('\n') {
                out.stdout.push('\n');
            }
            let err = std::mem::take(&mut out.stderr);
            out.stdout.push_str(&err);
        }

        // `2>` / `2>>`：标准错误单独写文件。
        if let Some(errfile) = &stage.redirect.stderr {
            let path = ctx.resolve(errfile);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let result = if stage.redirect.stderr_append {
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .and_then(|mut f| f.write_all(out.stderr.as_bytes()))
            } else {
                fs::write(&path, out.stderr.as_bytes())
            };
            if let Err(e) = result {
                return Output::err(format!("{}: {}", errfile, e));
            }
            out.stderr.clear();
        }

        if !out.stderr.is_empty() {
            accumulated_err.push_str(&out.stderr);
        }
        if out.flow == Flow::Exit {
            return out;
        }
        last = out;

        // 输出重定向
        if let Some(outfile) = &stage.redirect.output {
            let path = ctx.resolve(outfile);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let result = if stage.redirect.append {
                fs::OpenOptions::new().create(true).append(true).open(&path)
                    .and_then(|mut f| f.write_all(last.stdout.as_bytes()))
            } else {
                fs::write(&path, last.stdout.as_bytes())
            };
            if let Err(e) = result {
                return Output::err(format!("{}: {}", outfile, e));
            }
            last.stdout.clear();
            stdin_data = None;
        } else if i + 1 < stages.len() {
            // 仅当还有后续管道阶段时才把当前输出传给下一级；
            // 最后一级保留 stdout 供最终显示。
            stdin_data = Some(std::mem::take(&mut last.stdout));
        }
    }

    last.stderr = accumulated_err;
    last
}

// ---------------------------------------------------------------------------
// sudo 前缀处理
// ---------------------------------------------------------------------------

enum SudoRewrite {
    Stage(Stage),
    Done(Output),
}

enum SudoAction {
    Run(Vec<String>),
    Info(String),
    Silent,
    Error(String),
}

/// 把 `sudo <命令>` 改写为 `<命令>`，在当前 shell/进程里执行。
fn rewrite_sudo(stage: &Stage, ctx: &mut Context) -> SudoRewrite {
    let is_sudo = stage
        .argv
        .first()
        .map(|a| a.eq_ignore_ascii_case("sudo"))
        .unwrap_or(false);
    if !is_sudo {
        return SudoRewrite::Stage(stage.clone());
    }

    match parse_sudo_args(&stage.argv[1..]) {
        SudoAction::Run(argv) => {
            if !winutil::is_elevated() && !ctx.sudo_hint_shown {
                ctx.sudo_hint_shown = true;
                eprintln!(
                    "提示：Windows 没有真正的 sudo，命令将以当前权限执行；\
                     需要管理员权限的安装程序通常会弹出 UAC，失败时请以管理员身份运行 Wren。"
                );
            }
            SudoRewrite::Stage(Stage {
                argv,
                redirect: stage.redirect.clone(),
            })
        }
        SudoAction::Info(text) => SudoRewrite::Done(Output::ok(text)),
        SudoAction::Silent => SudoRewrite::Done(Output::code(0)),
        SudoAction::Error(text) => SudoRewrite::Done(Output::err(text)),
    }
}

fn parse_sudo_args(args: &[String]) -> SudoAction {
    let mut i = 0;
    let mut login_shell = false;
    let mut control_only = false;

    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            i += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        match arg.as_str() {
            "-V" | "--version" => {
                return SudoAction::Info(format!(
                    "sudo (Wren) {} — Windows 上没有真正的 sudo。\n\
                     使用 `sudo <命令>` 会以当前权限直接执行该命令。",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            "-l" | "--list" => return SudoAction::Info(admin_status()),
            "--help" | "-?" => return SudoAction::Info(sudo_help()),
            // 单独 `sudo -h` 按帮助处理；带参数时沿用 sudo 的 -h <host> 语义。
            "-h" if args.len() == 1 => return SudoAction::Info(sudo_help()),
            "-h" | "--host" => {
                i += 2;
                continue;
            }
            "-k" | "-K" | "-v" => {
                control_only = true;
                i += 1;
            }
            "-i" | "-s" | "--login" | "--shell" => {
                login_shell = true;
                i += 1;
            }
            "-u" | "--user" | "-g" | "--group" | "-p" | "--prompt" | "-C"
            | "--close-from" | "-D" | "--chdir" | "-r" | "--role" | "-t"
            | "--type" | "-U" | "--other-user" | "-T" | "--command-timeout" => {
                // 这些选项带一个参数，直接跳过。
                i += 2;
            }
            _ => i += 1,
        }
    }

    if i < args.len() {
        return SudoAction::Run(args[i..].to_vec());
    }
    if control_only {
        return SudoAction::Silent;
    }
    if login_shell {
        return SudoAction::Error(
            "sudo: Wren 不支持 sudo -i/-s 打开管理员子 shell。\n\
             请右键 Wren 选择“以管理员身份运行”，或直接运行 sudo <命令>。"
                .into(),
        );
    }
    SudoAction::Error("sudo: 缺少命令\n用法: sudo <命令> [参数]".into())
}

fn admin_status() -> String {
    let level = if winutil::is_elevated() {
        "管理员"
    } else {
        "普通用户"
    };
    format!(
        "sudo (Wren)：当前进程以{}权限运行。\n\
         Windows 没有 sudo 的用户切换机制；`sudo <命令>` 会以当前权限直接执行。",
        level
    )
}

fn sudo_help() -> String {
    "用法: sudo <命令> [参数]\n\
     \n\
     Wren 在 Windows 上模拟 sudo：去掉 sudo 前缀后直接执行命令。\n\
     如果命令需要管理员权限，安装程序通常会弹出 UAC；若失败，请右键\n\
     以管理员身份运行 Wren。\n\
     \n\
     可用选项:\n\
     \x20 -l, --list      显示当前权限\n\
     \x20 -V, --version   显示版本\n\
     \x20 -k, -v          兼容 sudo 的空操作\n\
     \x20 -u <用户>       接受但忽略（Windows 无用户切换）"
        .into()
}

/// 启动交互式外部程序（继承 stdin/stdout/stderr）。
fn run_interactive(stage: &Stage, ctx: &mut Context) -> Output {
    let (program, args, note) = prepare_command(stage, ctx);
    if let Some(n) = note {
        let _ = std::io::stderr().write_all(format!("{}\n", n).as_bytes());
    }
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .current_dir(&ctx.cwd)
        .envs(ctx.env.spawn_env())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    match cmd.status() {
        Ok(status) => Output::code(status.code().unwrap_or_else(|| if status.success() { 0 } else { 1 })),
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Output::err(format!("{}: 未找到命令{}", program, NOT_FOUND_HINT))
            } else {
                Output::err(format!("{}: {}", program, e))
            }
        }
    }
}

/// 启动外部程序并捕获输出，用于管道或重定向场景。
fn run_captured(stage: &Stage, ctx: &Context, stdin_data: Option<&str>) -> Output {
    let (program, args, note) = prepare_command(stage, ctx);
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .current_dir(&ctx.cwd)
        .envs(ctx.env.spawn_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                format!("{}: 未找到命令{}", program, NOT_FOUND_HINT)
            } else {
                format!("{}: {}", program, e)
            };
            return Output::err(msg);
        }
    };

    if let Some(data) = stdin_data {
        if let Some(mut sin) = child.stdin.take() {
            let _ = sin.write_all(data.as_bytes());
        }
    } else {
        drop(child.stdin.take());
    }

    match child.wait_with_output() {
        Ok(o) => {
            let mut stderr = String::new();
            if let Some(n) = note { stderr.push_str(&n); stderr.push('\n'); }
            stderr.push_str(&String::from_utf8_lossy(&o.stderr));
            Output {
                stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                stderr,
                code: o.status.code().unwrap_or(1),
                flow: Flow::Continue,
                raw: false,
            }
        }
        Err(e) => Output::err(format!("{}: {}", program, e)),
    }
}

/// 翻译命令并把 POSIX 风格路径参数转换为 Windows 路径。
fn prepare_command(stage: &Stage, ctx: &Context) -> (String, Vec<String>, Option<String>) {
    let name = &stage.argv[0];
    let t = translator::translate(name, &stage.argv[1..]);
    let home = ctx.env.home_string();
    let mut args = Vec::new();
    for a in t.args {
        let converted = if pathutil::looks_like_posix_path(&a) {
            pathutil::to_windows(&a, &ctx.cwd, Path::new(&home))
                .to_string_lossy()
                .to_string()
        } else if a == "~" || a.starts_with("~/") {
            pathutil::to_windows(&a, &ctx.cwd, Path::new(&home))
                .to_string_lossy()
                .to_string()
        } else {
            a
        };
        args.push(converted);
    }
    let requested = stage.argv[0].clone();
    let translated = t.program.clone();
    let (program, args, found) = resolve_executable(t.program, args, ctx);
    // 交给 Git Bash 时必须使用用户原始参数，否则 grep 等被翻译过的参数
    // （例如 -E → /R）会被 GNU 工具误读。
    if let Some((program, args)) =
        git_bash_fallback(&requested, &translated, &stage.argv[1..], found, ctx)
    {
        return (program, args, t.note);
    }
    (program, args, t.note)
}

/// 执行一整行并捕获标准输出，用于 `$(命令)` 命令替换。
pub fn capture_line(input: &str, ctx: &mut Context) -> Result<String, String> {
    let parsed = crate::parser::parse(input, &ctx.env)?;
    let mut output = String::new();
    for (conn, pipeline) in &parsed.pipelines {
        match conn {
            Conn::And if ctx.env.last_status() != 0 => continue,
            Conn::Or if ctx.env.last_status() == 0 => continue,
            _ => {}
        }
        let out = run_pipeline(pipeline, ctx);
        ctx.env.set_last_status(out.code);
        output.push_str(&out.stdout);
        if !out.stderr.is_empty() {
            let mut err = out.stderr.clone();
            if !err.ends_with('\n') {
                err.push('\n');
            }
            let _ = std::io::stderr().write_all(err.as_bytes());
        }
        if out.flow == Flow::Exit {
            break;
        }
    }
    Ok(output)
}

/// 展开 `$(命令)` 与反引号命令替换。
///
/// 单引号内不展开（与 bash 一致）；算术展开 `$((...))` 原样保留。
pub fn substitute_commands(input: &str, ctx: &mut Context) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < chars.len() {
        let c = chars[i];

        if c == '\\' && i + 1 < chars.len() && !in_single {
            out.push(c);
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            out.push(c);
            i += 1;
            continue;
        }
        if in_single {
            out.push(c);
            i += 1;
            continue;
        }

        if c == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
            if let Some(end) = find_closing_paren(&chars, i + 2) {
                let inner: String = chars[i + 2..end].iter().collect();
                let raw: String = chars[i..=end].iter().collect();
                if inner.trim_start().starts_with('(') {
                    // 算术展开 $((...))，暂不支持，原样保留。
                    out.push_str(&raw);
                } else {
                    out.push_str(&run_substitution(&inner, ctx, &raw));
                }
                i = end + 1;
                continue;
            }
        }

        if c == '`' {
            if let Some(offset) = chars[i + 1..].iter().position(|&ch| ch == '`') {
                let end = i + 1 + offset;
                let inner: String = chars[i + 1..end].iter().collect();
                let raw: String = chars[i..=end].iter().collect();
                out.push_str(&run_substitution(&inner, ctx, &raw));
                i = end + 1;
                continue;
            }
        }

        out.push(c);
        i += 1;
    }
    out
}

fn find_closing_paren(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 1;
    let mut i = start;
    let mut single = false;
    let mut double = false;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if !single => {
                i += 2;
                continue;
            }
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '(' if !single => depth += 1,
            ')' if !single => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn run_substitution(command: &str, ctx: &mut Context, fallback: &str) -> String {
    // 先递归处理嵌套的 $(...) / 反引号，再在“子 shell”里执行：
    // 克隆上下文，避免 cd / export 影响主 shell（与 bash 的子 shell 行为一致）。
    let expanded = substitute_commands(command, ctx);
    let mut sub = ctx.clone();
    match capture_line(&expanded, &mut sub) {
        Ok(text) => text
            .trim_end_matches(['\n', '\r'])
            .replace(['\n', '\r'], " "),
        Err(e) => {
            eprintln!("shell: {}", e);
            fallback.to_string()
        }
    }
}

/// 有 Git Bash 时，少数内置命令改用 GNU 版本以获得完整参数/正则支持。
fn builtin_active(name: &str, ctx: &Context) -> bool {
    if !builtins::is_builtin(name) {
        return false;
    }
    !builtin_bypassed(name, ctx)
}

fn builtin_bypassed(name: &str, ctx: &Context) -> bool {
    let lower = name.to_ascii_lowercase();
    if !matches!(
        lower.as_str(),
        "grep" | "find" | "head" | "tail" | "wc" | "cat"
    ) {
        return false;
    }
    if git_bash_disabled() {
        return false;
    }
    winutil::find_git_bash(&ctx.env.get_or("PATH", "")).is_some()
}

/// 把命令名解析为 PATH 中的真实程序。
///
/// Windows 的 `CreateProcess` 不能直接执行 `.cmd` / `.bat`（npm、pnpm、yarn、
/// npx 等都是这种 shim），必须经 `cmd.exe /C` 启动；同时显式解析 PATH，
/// 避免只装了 `.cmd` 的命令被误报“未找到”。
fn resolve_executable(
    program: String,
    args: Vec<String>,
    ctx: &Context,
) -> (String, Vec<String>, bool) {
    if !cfg!(windows) {
        return (program, args, true);
    }
    let lower = program.to_ascii_lowercase();
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        let (p, a) = wrap_batch(program, args);
        return (p, a, true);
    }
    // 显式路径（含目录）交给 cmd/系统处理，避免基于错误的工作目录做判断。
    if program.contains('/') || program.contains('\\') {
        return (program, args, true);
    }

    let path_var = ctx.env.get_or("PATH", "");
    let Some(found) = winutil::find_program(&path_var, &program) else {
        return (program, args, false);
    };
    let ext = found
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "cmd" | "bat" => {
            let (p, a) = wrap_batch(found.to_string_lossy().to_string(), args);
            (p, a, true)
        }
        "exe" | "com" => (found.to_string_lossy().to_string(), args, true),
        // 无扩展名的裸文件无法由 CreateProcess 可靠启动，交给 Git Bash 后备。
        _ => (program, args, false),
    }
}

fn wrap_batch(program: String, args: Vec<String>) -> (String, Vec<String>) {
    let mut new_args = Vec::with_capacity(args.len() + 2);
    new_args.push("/C".to_string());
    new_args.push(program);
    new_args.extend(args);
    ("cmd.exe".to_string(), new_args)
}

/// 命令找不到（或需要 GNU 语义）时，尝试经 Git Bash 执行。
///
/// 这是可选后备：没装 Git for Windows 时保持原行为，不影响 Wren 独立运行。
fn git_bash_fallback(
    requested: &str,
    translated: &str,
    args: &[String],
    found: bool,
    ctx: &Context,
) -> Option<(String, Vec<String>)> {
    if !cfg!(windows) || git_bash_disabled() {
        return None;
    }
    if requested.contains('/') || requested.contains('\\') {
        return None;
    }
    let identity = requested.eq_ignore_ascii_case(translated);
    let use_git_bash = if identity {
        !found || prefer_git_bash(requested)
    } else {
        prefer_git_bash(requested)
    };
    if !use_git_bash {
        return None;
    }

    let bash = winutil::find_git_bash(&ctx.env.get_or("PATH", ""))?;
    let mut bash_args = Vec::with_capacity(args.len() + 4);
    bash_args.push("-c".into());
    bash_args.push("exec \"$@\"".into());
    bash_args.push("--".into());
    bash_args.push(requested.to_string());
    bash_args.extend(args.iter().cloned());
    Some((bash.to_string_lossy().to_string(), bash_args))
}

fn git_bash_disabled() -> bool {
    match std::env::var("WREN_NO_GITBASH") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Windows 自带同名工具但语义不同、应优先使用 GNU 版本的命令。
fn prefer_git_bash(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "sed"
            | "awk"
            | "grep"
            | "egrep"
            | "fgrep"
            | "find"
            | "cat"
            | "head"
            | "tail"
            | "wc"
            | "sort"
            | "uniq"
            | "cut"
            | "tr"
            | "tee"
            | "xargs"
            | "stat"
            | "du"
            | "df"
            | "chmod"
            | "chown"
            | "readlink"
            | "realpath"
            | "basename"
            | "dirname"
            | "md5sum"
            | "sha1sum"
            | "sha256sum"
            | "sha512sum"
            | "base64"
            | "gzip"
            | "gunzip"
            | "bzip2"
            | "xz"
            | "tar"
            | "diff"
            | "patch"
            | "file"
            | "xxd"
            | "od"
            | "split"
            | "paste"
            | "join"
            | "comm"
            | "cmp"
            | "nl"
            | "fold"
            | "fmt"
            | "expand"
            | "unexpand"
            | "tac"
            | "shuf"
            | "mktemp"
            | "timeout"
            | "less"
            | "id"
            | "groups"
            | "nproc"
            | "locale"
            | "iconv"
            | "dos2unix"
            | "unix2dos"
            | "unzip"
            | "vim"
            | "nano"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sudo_strips_prefix() {
        match parse_sudo_args(&args(&["apt", "update"])) {
            SudoAction::Run(cmd) => assert_eq!(cmd, args(&["apt", "update"])),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn sudo_skips_value_options() {
        match parse_sudo_args(&args(&["-u", "root", "-E", "apt", "install", "git"])) {
            SudoAction::Run(cmd) => assert_eq!(cmd, args(&["apt", "install", "git"])),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn sudo_without_command_is_an_error() {
        assert!(matches!(
            parse_sudo_args(&args(&[])),
            SudoAction::Error(_)
        ));
    }

    #[test]
    fn sudo_k_is_silent() {
        assert!(matches!(
            parse_sudo_args(&args(&["-k"])),
            SudoAction::Silent
        ));
    }

    #[test]
    fn sudo_list_reports_permission() {
        assert!(matches!(
            parse_sudo_args(&args(&["-l"])),
            SudoAction::Info(_)
        ));
    }

    #[test]
    fn sudo_login_shell_is_rejected() {
        assert!(matches!(
            parse_sudo_args(&args(&["-i"])),
            SudoAction::Error(_)
        ));
    }

    #[test]
    fn sudo_echo_executes_inside_shell() {
        use crate::env::Env;
        use crate::parser;
        let mut ctx = Context::new(Env::new("test".into()));
        let parsed = parser::parse("sudo echo hi", &ctx.env).unwrap();
        let out = run_pipeline(&parsed.pipelines[0].1, &mut ctx);
        assert_eq!(out.stdout, "hi\n");
        assert_eq!(out.code, 0);
    }

    #[test]
    fn sudo_alone_is_not_a_cmd_shell() {
        use crate::env::Env;
        use crate::parser;
        let mut ctx = Context::new(Env::new("test".into()));
        let parsed = parser::parse("sudo", &ctx.env).unwrap();
        let out = run_pipeline(&parsed.pipelines[0].1, &mut ctx);
        assert_ne!(out.code, 0);
        assert!(out.stderr.contains("缺少命令"));
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_is_launched_through_cmd() {
        use crate::env::Env;
        let dir = std::env::temp_dir().join(format!("wren_cmd_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let shim = dir.join("hello.cmd");
        std::fs::write(&shim, "@echo hello\r\n").unwrap();

        let mut ctx = Context::new(Env::new("test".into()));
        ctx.env.set("PATH", &dir.to_string_lossy());
        let (program, args, found) =
            resolve_executable("hello".into(), vec!["-v".into()], &ctx);
        assert!(found);
        assert_eq!(program, "cmd.exe");
        assert_eq!(args[0], "/C");
        assert!(args[1].ends_with("hello.cmd"));
        assert_eq!(args[2], "-v");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_substitution_runs_in_subshell() {
        use crate::env::Env;
        let mut ctx = Context::new(Env::new("test".into()));
        let before = ctx.cwd.clone();

        let expanded = substitute_commands("echo $(echo hi)", &mut ctx);
        assert_eq!(expanded, "echo hi");

        // 子 shell 里的 cd 不应改变主 shell 的 cwd。
        let _ = substitute_commands("$(cd /c)", &mut ctx);
        assert_eq!(ctx.cwd, before);
    }

    #[test]
    fn single_quotes_block_substitution() {
        use crate::env::Env;
        let mut ctx = Context::new(Env::new("test".into()));
        let expanded = substitute_commands("echo '$(echo hi)'", &mut ctx);
        assert_eq!(expanded, "echo '$(echo hi)'");
    }

    #[test]
    fn backticks_are_substituted() {
        use crate::env::Env;
        let mut ctx = Context::new(Env::new("test".into()));
        let expanded = substitute_commands("echo `echo hi`", &mut ctx);
        assert_eq!(expanded, "echo hi");
    }
}
