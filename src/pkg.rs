//! `apt` / `apt-get`：面向 Windows 的软件包管理前端。
//!
//! 语法尽量贴近 Ubuntu 的 apt，后端优先使用 Windows 自带的 **winget**，
//! 找不到时回退到 Chocolatey / Scoop。包名与 Ubuntu 并不完全等价：
//! 会先在所选源里搜索，再按“精确名称 → 精确 moniker → 唯一结果”的顺序
//! 解析成真正的包 ID，因此 `apt install git`、`apt install python3` 都能用。

use crate::builtins::{Context, Flow, Output};
use crate::pathutil;
use crate::winutil::{clean_console_text, decode_console_bytes, find_program};
use std::cmp::Ordering;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DEFAULT_SOURCE: &str = "winget";

// ---------------------------------------------------------------------------
// 后端检测
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Winget,
    Choco,
    Scoop,
}

impl Backend {
    fn program(self) -> &'static str {
        match self {
            Backend::Winget => "winget",
            Backend::Choco => "choco",
            Backend::Scoop => "scoop",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Backend::Winget => "winget（Windows 包管理器）",
            Backend::Choco => "Chocolatey",
            Backend::Scoop => "Scoop",
        }
    }
}

fn detect_backend(ctx: &Context) -> Option<(Backend, PathBuf)> {
    let path = ctx.env.get_or("PATH", "");
    for backend in [Backend::Winget, Backend::Choco, Backend::Scoop] {
        if let Some(exe) = find_program(&path, backend.program()) {
            return Some((backend, exe));
        }
    }
    None
}

fn missing_backend() -> Output {
    Output::err(
        "apt: 未找到可用的 Windows 包管理器（winget / choco / scoop）。\n\
         提示：Windows 11 和较新的 Windows 10 自带 winget，可在 Microsoft Store\n\
         搜索 “应用安装程序 (App Installer)” 安装；安装后新开终端再运行 apt。",
    )
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

/// 运行 apt 命令。`alias` 为实际输入的命令名（apt / apt-get）。
pub fn run(alias: &str, args: &[String], ctx: &mut Context) -> Output {
    let (args, opts) = match parse_global(args) {
        Ok(x) => x,
        Err(e) => return Output::err(e),
    };

    let backend_info = detect_backend(ctx);

    // help / version 不应因为系统没装包管理器而失败。
    if opts.help {
        return help(backend_info.as_ref().map(|(b, _)| *b), alias);
    }
    if opts.version {
        return match backend_info {
            Some((backend, exe)) => version(backend, &exe, ctx),
            None => Output::ok(format!(
                "apt {} (Wren) — Windows 软件包管理前端\n\
                 后端: 未找到（需要 winget / choco / scoop）\n\
                 提示: 安装 “应用安装程序 (App Installer)” 后即可使用 winget。",
                env!("CARGO_PKG_VERSION")
            )),
        };
    }
    if args.is_empty() || args[0].eq_ignore_ascii_case("help") {
        return help(backend_info.as_ref().map(|(b, _)| *b), alias);
    }

    let (backend, exe) = match backend_info {
        Some(x) => x,
        None => return missing_backend(),
    };

    let sub = args[0].to_ascii_lowercase();
    let rest = &args[1..];

    match sub.as_str() {
        "version" => version(backend, &exe, ctx),
        "update" => apt_update(backend, &exe, &opts, ctx),
        "upgrade" => apt_upgrade(backend, &exe, rest, &opts, ctx),
        "install" => apt_install(backend, &exe, rest, &opts, ctx),
        "remove" | "purge" | "uninstall" | "erase" => {
            apt_remove(backend, &exe, rest, ctx)
        }
        "autoremove" => Output::ok(
            "apt autoremove: Windows 包管理器不维护依赖树，无需自动清理。\n\
             不再需要的程序可在“设置 > 应用”中卸载。",
        ),
        "clean" | "autoclean" => Output::ok(
            "apt clean: winget 会在安装后自行管理下载缓存，通常无需手动清理。",
        ),
        "search" => apt_search(backend, &exe, rest, &opts, ctx),
        "show" | "info" => apt_show(backend, &exe, rest, &opts, ctx),
        "list" => apt_list(backend, &exe, rest, ctx),
        "download" => apt_download(backend, &exe, rest, &opts, ctx),
        _ => Output::err(format!(
            "apt: 未知命令 '{}'\n运行 `apt --help` 查看可用命令。",
            args[0]
        )),
    }
}

// ---------------------------------------------------------------------------
// 全局选项
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Debug)]
struct GlobalOpts {
    source: Option<String>,
    assume_yes: bool,
    help: bool,
    version: bool,
}

/// 去掉全局选项（可出现在子命令前后），返回剩余参数。
fn parse_global(args: &[String]) -> Result<(Vec<String>, GlobalOpts), String> {
    let mut rest = Vec::new();
    let mut opts = GlobalOpts::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let lower = a.to_ascii_lowercase();
        match lower.as_str() {
            "-y" | "--yes" | "--assume-yes" => {
                opts.assume_yes = true;
                i += 1;
            }
            "-q" | "--quiet" | "-qq" | "--no-install-recommends" => {
                // winget 使用 --accept-* 自动确认；这些 apt 选项只需接受，不产生行为。
                i += 1;
            }
            "--help" | "-h" | "-?" => {
                opts.help = true;
                i += 1;
            }
            "--version" | "-v" => {
                opts.version = true;
                i += 1;
            }
            "--source" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "apt: --source 需要一个源名称（如 winget / msstore）".to_string())?;
                opts.source = Some(value.clone());
                i += 2;
            }
            _ if lower.starts_with("--source=") => {
                let value = a["--source=".len()..].trim();
                if value.is_empty() {
                    return Err("apt: --source 需要一个源名称".into());
                }
                opts.source = Some(value.to_string());
                i += 1;
            }
            _ => {
                rest.push(a.clone());
                i += 1;
            }
        }
    }
    Ok((rest, opts))
}

fn source_name(opts: &GlobalOpts) -> String {
    opts.source.clone().unwrap_or_else(|| DEFAULT_SOURCE.to_string())
}

// ---------------------------------------------------------------------------
// 进程执行
// ---------------------------------------------------------------------------

fn interactive_enabled(ctx: &Context) -> bool {
    ctx.interactive && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// 交互式确认（默认 Yes，与 apt 一致）。
fn confirm(prompt: &str) -> bool {
    use std::io::Write;
    print!("{}", prompt);
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    let answer = input.trim().to_ascii_lowercase();
    answer.is_empty() || answer == "y" || answer == "yes"
}

/// 运行后端工具。交互模式下继承 stdio（可看到进度条），否则捕获输出。
fn run_tool(exe: &Path, args: &[String], ctx: &Context, interactive: bool) -> Output {
    let mut program = exe.to_path_buf();
    let mut full_args = args.to_vec();

    // CreateProcess 不能直接执行 .cmd/.bat（例如 scoop 的 shim），需要包一层 cmd /C。
    let ext = exe
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "cmd" || ext == "bat" {
        full_args.insert(0, exe.to_string_lossy().to_string());
        full_args.insert(0, "/C".to_string());
        program = PathBuf::from("cmd.exe");
    }

    let mut cmd = Command::new(&program);
    cmd.args(&full_args)
        .current_dir(&ctx.cwd)
        .envs(ctx.env.spawn_env());

    if interactive {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        match cmd.status() {
            Ok(status) => Output::code(
                status
                    .code()
                    .unwrap_or_else(|| if status.success() { 0 } else { 1 }),
            ),
            Err(e) => Output::err(format!("apt: 无法启动 {}: {}", exe.display(), e)),
        }
    } else {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match cmd.output() {
            Ok(o) => Output {
                stdout: clean_console_text(&decode_console_bytes(&o.stdout)),
                stderr: clean_console_text(&decode_console_bytes(&o.stderr)),
                code: o.status.code().unwrap_or(1),
                flow: Flow::Continue,
                raw: false,
            },
            Err(e) => Output::err(format!("apt: 无法启动 {}: {}", exe.display(), e)),
        }
    }
}

fn combined_text(out: &Output) -> String {
    let mut s = out.stdout.clone();
    if !out.stderr.is_empty() {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&out.stderr);
    }
    s
}

fn winget_base_args(sub: &str) -> Vec<String> {
    vec![sub.to_string()]
}

/// winget 子命令通用参数：自动接受协议，非交互模式下禁用提示。
fn winget_finish_args(args: &mut Vec<String>, interactive: bool) {
    args.push("--accept-source-agreements".into());
    if !interactive {
        args.push("--disable-interactivity".into());
    }
}

// ---------------------------------------------------------------------------
// 输出累加器：交互模式直接打印，非交互模式汇总为 Output
// ---------------------------------------------------------------------------

struct Accum {
    interactive: bool,
    stdout: String,
    stderr: String,
}

impl Accum {
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

    fn merge(&mut self, out: Output) {
        if !out.stdout.is_empty() {
            if self.interactive {
                print!("{}", out.stdout);
                if !out.stdout.ends_with('\n') {
                    println!();
                }
            } else {
                self.stdout.push_str(&out.stdout);
            }
        }
        if !out.stderr.is_empty() {
            if self.interactive {
                eprint!("{}", out.stderr);
                if !out.stderr.ends_with('\n') {
                    eprintln!();
                }
            } else {
                self.stderr.push_str(&out.stderr);
            }
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
// apt update
// ---------------------------------------------------------------------------

fn apt_update(backend: Backend, exe: &Path, _opts: &GlobalOpts, ctx: &Context) -> Output {
    let interactive = interactive_enabled(ctx);
    let mut acc = Accum::new(interactive);

    match backend {
        Backend::Winget => {
            acc.line("正在刷新软件包索引（winget source update）...");
            let mut args = vec!["source".to_string(), "update".to_string()];
            if !interactive {
                args.push("--disable-interactivity".into());
            }
            let out = run_tool(exe, &args, ctx, interactive);
            let code = out.code;
            acc.merge(out);
            if code == 0 {
                acc.line("软件包索引已更新。");
                acc.finish(0)
            } else {
                acc.error("apt: 刷新软件包索引失败。请检查网络或稍后重试。");
                acc.finish(code)
            }
        }
        Backend::Choco => Output::ok(
            "Chocolatey 会在安装/升级软件时自动刷新源，无需执行 apt update。",
        ),
        Backend::Scoop => {
            acc.line("正在更新 Scoop 与软件源（scoop update）...");
            let out = run_tool(
                exe,
                &["update".to_string()],
                ctx,
                interactive,
            );
            let code = out.code;
            acc.merge(out);
            if code == 0 {
                acc.line("软件包索引已更新。");
            }
            acc.finish(code)
        }
    }
}

// ---------------------------------------------------------------------------
// apt upgrade
// ---------------------------------------------------------------------------

fn apt_upgrade(
    backend: Backend,
    exe: &Path,
    args: &[String],
    opts: &GlobalOpts,
    ctx: &Context,
) -> Output {
    let packages: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect();
    let interactive = interactive_enabled(ctx);
    let mut acc = Accum::new(interactive);

    // 升级全部软件影响面较大：交互模式先确认，批处理模式要求 -y。
    if packages.is_empty() && !opts.assume_yes {
        if !interactive {
            return Output::err(
                "apt: 非交互模式下升级全部软件需要 -y（--yes）。\n\
                 例如：apt upgrade -y",
            );
        }
        if !confirm("即将升级所有可升级的软件，继续？[Y/n] ") {
            let mut out = Output::ok("已取消。");
            out.code = 1;
            return out;
        }
    }

    match backend {
        Backend::Winget => {
            let source = source_name(opts);
            if packages.is_empty() {
                acc.line("正在升级所有可升级的软件包（winget upgrade --all）...");
                let mut cmd = winget_base_args("upgrade");
                cmd.extend([
                    "--all".into(),
                    "--include-unknown".into(),
                    "--source".into(),
                    source,
                    "--accept-package-agreements".into(),
                ]);
                winget_finish_args(&mut cmd, interactive);
                let out = run_tool(exe, &cmd, ctx, interactive);
                let code = out.code;
                let text = combined_text(&out);
                acc.merge(out);
                if code == 0 {
                    acc.line("升级完成。");
                    acc.finish(0)
                } else if is_no_upgrade(&text) {
                    acc.line("没有可用的升级。");
                    acc.finish(0)
                } else {
                    acc.error("apt: 升级失败。若提示权限不足，请以管理员身份运行 Wren 后重试。");
                    acc.finish(code)
                }
            } else {
                for raw in &packages {
                    let (name, version) = split_name_version(raw);
                    acc.line(format!("正在查找软件包 {}...", name));
                    let chosen = match resolve_winget(exe, &name, &source, ctx) {
                        Ok(p) => p,
                        Err(e) => {
                            acc.merge(e);
                            return acc.finish(1);
                        }
                    };
                    acc.line(format!("正在升级 {} [{}]...", chosen.name, chosen.id));
                    let mut cmd = winget_base_args("upgrade");
                    cmd.extend([
                        "--id".into(),
                        chosen.id.clone(),
                        "-e".into(),
                        "--source".into(),
                        source.clone(),
                        "--accept-package-agreements".into(),
                    ]);
                    if let Some(v) = version {
                        cmd.push("--version".into());
                        cmd.push(v);
                    }
                    winget_finish_args(&mut cmd, interactive);
                    let out = run_tool(exe, &cmd, ctx, interactive);
                    let code = out.code;
                    acc.merge(out);
                    if code != 0 {
                        acc.error(format!("apt: 升级 {} 失败。", chosen.name));
                        return acc.finish(code);
                    }
                    acc.line(format!("已升级 {}。", chosen.name));
                }
                acc.finish(0)
            }
        }
        Backend::Choco => {
            if packages.is_empty() {
                let out = run_tool(
                    exe,
                    &["upgrade".into(), "all".into(), "-y".into()],
                    ctx,
                    interactive,
                );
                let code = out.code;
                acc.merge(out);
                acc.finish(code)
            } else {
                for pkg in &packages {
                    let out = run_tool(
                        exe,
                        &["upgrade".into(), pkg.clone(), "-y".into()],
                        ctx,
                        interactive,
                    );
                    let code = out.code;
                    acc.merge(out);
                    if code != 0 {
                        return acc.finish(code);
                    }
                }
                acc.finish(0)
            }
        }
        Backend::Scoop => {
            let mut cmd: Vec<String> = vec!["update".into()];
            if packages.is_empty() {
                cmd.push("*".into());
            } else {
                cmd.extend(packages.iter().cloned());
            }
            let out = run_tool(exe, &cmd, ctx, interactive);
            let code = out.code;
            acc.merge(out);
            acc.finish(code)
        }
    }
}

// ---------------------------------------------------------------------------
// apt install
// ---------------------------------------------------------------------------

fn apt_install(
    backend: Backend,
    exe: &Path,
    args: &[String],
    opts: &GlobalOpts,
    ctx: &Context,
) -> Output {
    let mut packages = Vec::new();
    let mut reinstall = false;
    for a in args {
        match a.as_str() {
            "--reinstall" => reinstall = true,
            "-y" | "--yes" | "--assume-yes" | "-q" | "--quiet" => {}
            _ if a.starts_with('-') => {
                return Output::err(format!("apt: install 不支持的选项 '{}'", a));
            }
            _ => packages.push(a.clone()),
        }
    }
    if packages.is_empty() {
        return Output::err("apt: install 缺少软件包名\n用法: apt install <软件包...>");
    }

    let interactive = interactive_enabled(ctx);
    let mut acc = Accum::new(interactive);

    match backend {
        Backend::Winget => {
            let source = source_name(opts);
            for raw in &packages {
                let (name, version) = split_name_version(raw);
                acc.line(format!("正在查找软件包 {}...", name));
                let chosen = match resolve_winget(exe, &name, &source, ctx) {
                    Ok(p) => p,
                    Err(e) => {
                        acc.merge(e);
                        return acc.finish(1);
                    }
                };
                acc.line(format!("正在安装 {} [{}]...", chosen.name, chosen.id));
                let mut cmd = winget_base_args("install");
                cmd.extend([
                    "--id".into(),
                    chosen.id.clone(),
                    "-e".into(),
                    "--source".into(),
                    source.clone(),
                    "--accept-package-agreements".into(),
                ]);
                if let Some(v) = version {
                    cmd.push("--version".into());
                    cmd.push(v);
                }
                if reinstall {
                    cmd.push("--force".into());
                }
                winget_finish_args(&mut cmd, interactive);
                let out = run_tool(exe, &cmd, ctx, interactive);
                let code = out.code;
                acc.merge(out);
                if code != 0 {
                    acc.error(format!(
                        "apt: 安装 {} 失败。若提示权限不足，请右键以管理员身份运行 Wren 后重试。",
                        chosen.name
                    ));
                    return acc.finish(code);
                }
                acc.line(format!("已安装 {}。", chosen.name));
            }
            acc.finish(0)
        }
        Backend::Choco => {
            let mut cmd = vec!["install".to_string()];
            cmd.extend(packages.iter().cloned());
            cmd.push("-y".into());
            let out = run_tool(exe, &cmd, ctx, interactive);
            let code = out.code;
            acc.merge(out);
            acc.finish(code)
        }
        Backend::Scoop => {
            let mut cmd = vec!["install".to_string()];
            cmd.extend(packages.iter().cloned());
            let out = run_tool(exe, &cmd, ctx, interactive);
            let code = out.code;
            acc.merge(out);
            acc.finish(code)
        }
    }
}

// ---------------------------------------------------------------------------
// apt remove
// ---------------------------------------------------------------------------

fn apt_remove(backend: Backend, exe: &Path, args: &[String], ctx: &Context) -> Output {
    let packages: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect();
    if packages.is_empty() {
        return Output::err("apt: remove 缺少软件包名\n用法: apt remove <软件包...>");
    }

    let interactive = interactive_enabled(ctx);
    let mut acc = Accum::new(interactive);

    match backend {
        Backend::Winget => {
            for pkg in &packages {
                acc.line(format!("正在卸载 {}...", pkg));
                let mut cmd = winget_base_args("uninstall");
                if pkg.contains('.') {
                    cmd.extend(["--id".into(), pkg.clone(), "-e".into()]);
                } else {
                    cmd.push(pkg.clone());
                }
                winget_finish_args(&mut cmd, interactive);
                let out = run_tool(exe, &cmd, ctx, interactive);
                let code = out.code;
                acc.merge(out);
                if code != 0 {
                    acc.error(format!(
                        "apt: 卸载 {} 失败（可能未安装，或名称不唯一；可用 apt list 查看）。",
                        pkg
                    ));
                    return acc.finish(code);
                }
                acc.line(format!("已卸载 {}。", pkg));
            }
            acc.finish(0)
        }
        Backend::Choco => {
            let mut cmd = vec!["uninstall".to_string()];
            cmd.extend(packages.iter().cloned());
            cmd.push("-y".into());
            let out = run_tool(exe, &cmd, ctx, interactive);
            let code = out.code;
            acc.merge(out);
            acc.finish(code)
        }
        Backend::Scoop => {
            let mut cmd = vec!["uninstall".to_string()];
            cmd.extend(packages.iter().cloned());
            let out = run_tool(exe, &cmd, ctx, interactive);
            let code = out.code;
            acc.merge(out);
            acc.finish(code)
        }
    }
}

// ---------------------------------------------------------------------------
// apt search / show / list
// ---------------------------------------------------------------------------

fn apt_search(
    backend: Backend,
    exe: &Path,
    args: &[String],
    opts: &GlobalOpts,
    ctx: &Context,
) -> Output {
    let query = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if query.trim().is_empty() {
        return Output::err("apt: search 缺少关键词\n用法: apt search <关键词>");
    }

    let interactive = interactive_enabled(ctx);
    let cmd = match backend {
        Backend::Winget => {
            let mut c = vec!["search".to_string(), query];
            c.extend([
                "--source".into(),
                source_name(opts),
                "--accept-source-agreements".into(),
            ]);
            if !interactive {
                c.push("--disable-interactivity".into());
            }
            c
        }
        Backend::Choco => vec!["search".into(), query],
        Backend::Scoop => vec!["search".into(), query],
    };

    run_tool(exe, &cmd, ctx, interactive)
}

fn apt_show(
    backend: Backend,
    exe: &Path,
    args: &[String],
    opts: &GlobalOpts,
    ctx: &Context,
) -> Output {
    let query = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if query.trim().is_empty() {
        return Output::err("apt: show 缺少软件包名\n用法: apt show <软件包>");
    }

    let interactive = interactive_enabled(ctx);
    let mut acc = Accum::new(interactive);

    match backend {
        Backend::Winget => {
            let source = source_name(opts);
            let chosen = match resolve_winget(exe, &query, &source, ctx) {
                Ok(p) => p,
                Err(e) => {
                    acc.merge(e);
                    return acc.finish(1);
                }
            };
            let mut cmd = winget_base_args("show");
            cmd.extend([
                "--id".into(),
                chosen.id,
                "-e".into(),
                "--source".into(),
                source,
            ]);
            winget_finish_args(&mut cmd, interactive);
            let out = run_tool(exe, &cmd, ctx, interactive);
            let code = out.code;
            acc.merge(out);
            acc.finish(code)
        }
        Backend::Choco => {
            let out = run_tool(exe, &["info".into(), query], ctx, interactive);
            let code = out.code;
            acc.merge(out);
            acc.finish(code)
        }
        Backend::Scoop => {
            let out = run_tool(exe, &["info".into(), query], ctx, interactive);
            let code = out.code;
            acc.merge(out);
            acc.finish(code)
        }
    }
}

fn apt_list(backend: Backend, exe: &Path, args: &[String], ctx: &Context) -> Output {
    let mut installed = false;
    let mut upgradable = false;
    let mut query = Vec::new();
    for a in args {
        match a.as_str() {
            "--installed" => installed = true,
            "--upgradable" | "--upgradeable" | "--upgrades" => upgradable = true,
            "--all-versions" | "--verbose" | "-v" => {}
            _ if a.starts_with('-') => {
                return Output::err(format!("apt: list 不支持的选项 '{}'", a));
            }
            _ => query.push(a.clone()),
        }
    }
    let query = query.join(" ");
    let interactive = interactive_enabled(ctx);

    let mut cmd: Vec<String> = match backend {
        Backend::Winget => {
            if upgradable {
                let mut c = vec!["upgrade".into()];
                if !query.is_empty() {
                    c.push(query);
                }
                c
            } else {
                let mut c = vec!["list".into()];
                if !query.is_empty() {
                    c.push(query);
                }
                c
            }
        }
        Backend::Choco => {
            if upgradable {
                vec!["outdated".into()]
            } else {
                vec!["list".into(), "--local-only".into()]
            }
        }
        Backend::Scoop => {
            if upgradable {
                vec!["status".into()]
            } else {
                vec!["list".into()]
            }
        }
    };
    if backend == Backend::Winget {
        cmd.push("--accept-source-agreements".into());
        if !interactive {
            cmd.push("--disable-interactivity".into());
        }
    }
    let _ = installed;

    run_tool(exe, &cmd, ctx, interactive)
}

// ---------------------------------------------------------------------------
// apt download
// ---------------------------------------------------------------------------

fn apt_download(
    backend: Backend,
    exe: &Path,
    args: &[String],
    opts: &GlobalOpts,
    ctx: &Context,
) -> Output {
    let mut packages = Vec::new();
    let mut dir: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-d" | "--dir" | "--directory" | "--download-directory" => {
                let value = match args.get(i + 1) {
                    Some(v) => v.clone(),
                    None => return Output::err("apt: --dir 需要一个目录"),
                };
                dir = Some(value);
                i += 2;
            }
            _ if args[i].starts_with('-') => {
                return Output::err(format!("apt: download 不支持的选项 '{}'", args[i]));
            }
            _ => {
                packages.push(args[i].clone());
                i += 1;
            }
        }
    }
    if packages.is_empty() {
        return Output::err(
            "apt: download 缺少软件包名\n用法: apt download <软件包...> [-d 目录]",
        );
    }
    if backend != Backend::Winget {
        return Output::err("apt: download 目前仅支持 winget 后端。");
    }

    let target = match dir {
        Some(d) => ctx.resolve(&d),
        None => ctx.cwd.clone(),
    };
    if let Err(e) = fs::create_dir_all(&target) {
        return Output::err(format!("apt: 无法创建下载目录 '{}': {}", target.display(), e));
    }

    let interactive = interactive_enabled(ctx);
    let mut acc = Accum::new(interactive);
    let source = source_name(opts);

    for raw in &packages {
        let (name, version) = split_name_version(raw);
        acc.line(format!("正在查找软件包 {}...", name));
        let chosen = match resolve_winget(exe, &name, &source, ctx) {
            Ok(p) => p,
            Err(e) => {
                acc.merge(e);
                return acc.finish(1);
            }
        };
        acc.line(format!("正在下载 {} [{}]...", chosen.name, chosen.id));
        let mut cmd = winget_base_args("download");
        cmd.extend([
            "--id".into(),
            chosen.id.clone(),
            "-e".into(),
            "--source".into(),
            source.clone(),
            "--download-directory".into(),
            target.to_string_lossy().to_string(),
            "--accept-package-agreements".into(),
        ]);
        if let Some(v) = version {
            cmd.push("--version".into());
            cmd.push(v);
        }
        winget_finish_args(&mut cmd, interactive);
        let out = run_tool(exe, &cmd, ctx, interactive);
        let code = out.code;
        acc.merge(out);
        if code != 0 {
            acc.error(format!("apt: 下载 {} 失败。", chosen.name));
            return acc.finish(code);
        }
        acc.line(format!(
            "已下载 {} 到 {}。",
            chosen.name,
            pathutil::to_posix(&target)
        ));
    }
    acc.finish(0)
}

// ---------------------------------------------------------------------------
// winget 搜索结果解析与包名解析
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub name: String,
    pub id: String,
    pub version: String,
    pub match_info: String,
    pub source: String,
}

fn resolve_winget(
    exe: &Path,
    query: &str,
    source: &str,
    ctx: &Context,
) -> Result<Package, Output> {
    let query = query.trim();
    if query.is_empty() {
        return Err(Output::err("apt: 空软件包名"));
    }

    let mut cmd = vec!["search".to_string(), query.to_string()];
    cmd.extend([
        "--source".into(),
        source.to_string(),
        "--accept-source-agreements".into(),
        "--disable-interactivity".into(),
    ]);
    let out = run_tool(exe, &cmd, ctx, false);
    let text = combined_text(&out);
    let rows = parse_winget_table(&text);

    if rows.is_empty() {
        let detail = if out.code != 0 && !text.trim().is_empty() && !is_no_results(&text) {
            format!("\n{}", text.trim())
        } else {
            String::new()
        };
        return Err(Output::err(format!(
            "apt: 在源 '{}' 中未找到软件包 '{}'。可先运行 `apt search {}` 查看。{}",
            source, query, query, detail
        )));
    }

    match choose_package(&rows, query) {
        Some(p) => Ok(p),
        None => Err(Output::err(ambiguous_message(query, &rows))),
    }
}

fn parse_winget_table(text: &str) -> Vec<Package> {
    let cleaned = clean_console_text(text);
    let mut in_table = false;
    let mut rows = Vec::new();
    for line in cleaned.lines() {
        let trimmed = line.trim();
        if !in_table {
            if is_table_separator(trimmed) {
                in_table = true;
            }
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        // 表格结束后的汇总/提示行。
        if trimmed.starts_with("已找到")
            || trimmed.starts_with("找到")
            || trimmed.starts_with("未找到")
            || trimmed.starts_with("No package")
            || trimmed.starts_with("No installed package")
            || trimmed.starts_with("Found ")
        {
            break;
        }
        if let Some(pkg) = parse_table_row(line) {
            rows.push(pkg);
        }
    }
    rows
}

fn is_table_separator(s: &str) -> bool {
    let mut dashes = 0;
    for c in s.chars() {
        if c == '-' {
            dashes += 1;
        } else if !c.is_whitespace() {
            return false;
        }
    }
    dashes >= 10
}

fn token_spans(line: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                spans.push((s, idx));
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(s) = start {
        spans.push((s, line.len()));
    }
    spans
}

fn parse_table_row(line: &str) -> Option<Package> {
    let spans = token_spans(line);
    if spans.len() < 2 {
        return None;
    }
    let tokens: Vec<&str> = spans.iter().map(|(s, e)| &line[*s..*e]).collect();

    let last_lower = tokens.last()?.to_ascii_lowercase();
    let source = match last_lower.as_str() {
        "winget" => "winget",
        "msstore" => "msstore",
        _ => "",
    };
    // 没有源列时，把整行都当作表格内容；有源列时排除最后一列。
    let end = if source.is_empty() {
        tokens.len()
    } else {
        tokens.len() - 1
    };
    if end < 2 {
        return None;
    }

    // ID 紧挨着版本列。取最后一个“像 ID 且后面跟着版本”的位置，
    // 这样即使名称里带点（如 Node.js）或列宽溢出也能正确识别。
    let mut found: Option<usize> = None;
    for i in 1..end.saturating_sub(1) {
        if is_id_token(tokens[i], source) && is_version_token(tokens[i + 1]) {
            found = Some(i);
        }
    }
    let id_idx = found?;
    let version_idx = id_idx + 1;

    let name = line[..spans[id_idx].0].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let match_info = if version_idx + 1 < end {
        tokens[version_idx + 1..end].join(" ")
    } else {
        String::new()
    };

    Some(Package {
        name,
        id: tokens[id_idx].to_string(),
        version: tokens[version_idx].to_string(),
        match_info,
        source: source.to_string(),
    })
}

fn is_id_token(token: &str, source: &str) -> bool {
    if source == "msstore" {
        return token.len() >= 6
            && token.chars().all(|c| c.is_ascii_alphanumeric())
            && token.chars().any(|c| c.is_ascii_uppercase());
    }
    token.contains('.') && token.chars().any(|c| c.is_ascii_alphabetic())
}

fn is_version_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower == "unknown" || lower == "latest" || token == "未知" {
        return true;
    }
    let mut chars = token.chars();
    match chars.next() {
        Some(c) if c.is_ascii_digit() => true,
        Some('v' | 'V') => chars.next().map(|c| c.is_ascii_digit()).unwrap_or(false),
        _ => false,
    }
}

fn choose_package(rows: &[Package], query: &str) -> Option<Package> {
    // 1. 直接给了包 ID。
    if let Some(p) = rows.iter().find(|p| p.id.eq_ignore_ascii_case(query)) {
        return Some(p.clone());
    }
    // 2. 名称精确匹配（例如 git -> Git）。
    let exact_name: Vec<&Package> = rows
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(query))
        .collect();
    if !exact_name.is_empty() {
        return Some(best_package(&exact_name, query));
    }
    // 3. moniker 精确匹配（例如 python3 / nodejs / code / 7zip）。
    let exact_moniker: Vec<&Package> = rows
        .iter()
        .filter(|p| match_value(&p.match_info, "Moniker").map_or(false, |v| v.eq_ignore_ascii_case(query)))
        .collect();
    if !exact_moniker.is_empty() {
        return Some(best_package(&exact_moniker, query));
    }
    // 4. 命令名精确匹配（例如 python3 可能来自 Command: 字段）。
    let exact_command: Vec<&Package> = rows
        .iter()
        .filter(|p| match_value(&p.match_info, "Command").map_or(false, |v| v.eq_ignore_ascii_case(query)))
        .collect();
    if !exact_command.is_empty() {
        return Some(best_package(&exact_command, query));
    }
    // 5. 唯一结果直接使用。
    if rows.len() == 1 {
        return Some(rows[0].clone());
    }
    None
}

fn match_value<'a>(info: &'a str, kind: &str) -> Option<&'a str> {
    let mut parts = info.split_whitespace();
    while let Some(part) = parts.next() {
        if part.eq_ignore_ascii_case(&format!("{}:", kind)) {
            return parts.next();
        }
    }
    None
}

fn best_package(candidates: &[&Package], query: &str) -> Package {
    let mut best = candidates[0];
    for cand in &candidates[1..] {
        if package_better(cand, best, query) {
            best = cand;
        }
    }
    (*best).clone()
}

/// 同名候选之间怎么选：先看“是否官方同名包”，再看来源、版本。
///
/// 例：搜索 `git` 会同时命中 `Git.Git` 和 `Microsoft.Git`，
/// 前者是 Git for Windows 的官方包，应当优先。
fn package_better(a: &Package, b: &Package, query: &str) -> bool {
    let sa = package_score(a, query);
    let sb = package_score(b, query);
    if sa != sb {
        return sa > sb;
    }
    let ra = source_rank(a);
    let rb = source_rank(b);
    if ra != rb {
        return ra < rb;
    }
    version_cmp(&a.version, &b.version) == Ordering::Greater
}

fn package_score(p: &Package, query: &str) -> i32 {
    let query_lower = query.to_ascii_lowercase();
    let id_lower = p.id.to_ascii_lowercase();
    let name_lower = p.name.to_ascii_lowercase();
    let mut score = 0;

    // 官方同名包：ID 形如 Publisher.Package，且两段相同。
    if is_self_titled(&p.id) {
        score += 100;
    }
    // ID 最后一段与查询词一致（Git.Git、7zip.7zip、OpenJS.NodeJS ...）。
    if id_lower.rsplit('.').next() == Some(query_lower.as_str()) {
        score += 20;
    }
    // 名称与查询完全一致。
    if name_lower == query_lower {
        score += 30;
    }
    // 同分时更短的 ID 更可能是主包。
    score -= (p.id.len() / 8) as i32;
    // 避免自动选中预览/每夜构建。
    for unstable in ["nightly", "canary", "beta", "preview", "insider", "unstable"] {
        if name_lower.contains(unstable) {
            score -= 40;
            break;
        }
    }
    score
}

fn is_self_titled(id: &str) -> bool {
    let mut parts = id.split('.');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(b), None) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

fn source_rank(p: &Package) -> u8 {
    match p.source.as_str() {
        "winget" => 0,
        "msstore" => 1,
        _ => 2,
    }
}

fn version_cmp(a: &str, b: &str) -> Ordering {
    let pa = version_segments(a);
    let pb = version_segments(b);
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

fn version_segments(s: &str) -> Vec<u64> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn ambiguous_message(query: &str, rows: &[Package]) -> String {
    let mut s = format!("apt: 找到多个匹配 '{}' 的软件包：\n\n", query);
    for p in rows.iter().take(15) {
        s.push_str(&format!("  {:<46} {} {}\n", p.id, p.version, p.name));
    }
    if rows.len() > 15 {
        s.push_str(&format!("  …… 其余 {} 个结果已省略\n", rows.len() - 15));
    }
    s.push_str(&format!(
        "\n请使用完整包 ID 安装，例如：apt install {}\n",
        rows[0].id
    ));
    s
}

/// 支持 `包名=版本` 的写法。
fn split_name_version(raw: &str) -> (String, Option<String>) {
    match raw.split_once('=') {
        Some((name, version)) if !name.is_empty() && !version.is_empty() => {
            (name.to_string(), Some(version.to_string()))
        }
        _ => (raw.to_string(), None),
    }
}

fn is_no_results(text: &str) -> bool {
    text.contains("找不到与输入条件匹配")
        || text.contains("未找到与输入条件匹配")
        || text.contains("找不到与输入")
        || text.to_ascii_lowercase().contains("no package found")
}

fn is_no_upgrade(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains("没有可用的升级")
        || text.contains("没有可升级")
        || text.contains("无可用升级")
        || lower.contains("no applicable upgrade")
        || lower.contains("no upgrades")
        || lower.contains("no updated package")
}

// ---------------------------------------------------------------------------
// 帮助 / 版本
// ---------------------------------------------------------------------------

fn help(backend: Option<Backend>, alias: &str) -> Output {
    let backend_label = match backend {
        Some(b) => b.label().to_string(),
        None => "未检测到（需要 winget / choco / scoop）".to_string(),
    };
    let s = format!(
        "apt — Wren 的软件包管理器（当前后端：{}）\n\
         \n\
         用法: {alias} [选项] <命令> [参数]\n\
         \n\
         命令:\n\
         \x20 update                     刷新软件包索引\n\
         \x20 upgrade [包...]            升级全部或指定软件\n\
         \x20 install <包...>            安装软件（支持 包名=版本）\n\
         \x20 remove <包...>             卸载软件（别名 purge/uninstall）\n\
         \x20 search <关键词>            搜索软件包\n\
         \x20 show <包>                  显示软件包信息\n\
         \x20 list [--installed|--upgradable] [关键词]\n\
         \x20                            列出已安装 / 可升级的软件\n\
         \x20 download <包...> [-d 目录] 仅下载安装程序（默认当前目录）\n\
         \x20 clean | autoremove         清理提示（Windows 下为无操作）\n\
         \n\
         选项:\n\
         \x20 -y, --yes                  自动确认（主要用于 apt upgrade 全部升级）\n\
         \x20 --source <名称>            指定包源（默认 winget，也可用 msstore）\n\
         \x20 -h, --help                 显示本帮助\n\
         \x20 -v, --version              显示版本与后端信息\n\
         \n\
         示例:\n\
         \x20 apt update\n\
         \x20 apt search git\n\
         \x20 apt install git\n\
         \x20 apt install python3\n\
         \x20 apt upgrade\n\
         \x20 apt download 7zip -d ~/Downloads\n\
         \n\
         说明：软件包来自 Windows 包管理器，包名与 Ubuntu 不完全相同；\n\
         解析顺序为“精确 ID → 精确名称 → 精确 moniker → 唯一搜索结果”。",
        backend_label,
        alias = alias
    );
    Output::ok(s)
}

fn version(backend: Backend, exe: &Path, ctx: &Context) -> Output {
    let out = run_tool(exe, &["--version".to_string()], ctx, false);
    let first_line = combined_text(&out)
        .lines()
        .map(|l| l.trim().to_string())
        .find(|l| !l.is_empty())
        .unwrap_or_else(|| "未知版本".into());
    Output::ok(format!(
        "apt {} (Wren) — Windows 软件包管理前端\n\
         后端: {} ({})\n\
         默认源: {}\n\
         提示: 使用 `apt --help` 查看用法。",
        env!("CARGO_PKG_VERSION"),
        backend.label(),
        first_line,
        DEFAULT_SOURCE
    ))
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_SAMPLE: &str = "\
名称                   ID                         版本           匹配             源\n\
------------------------------------------------------------------------------------------------\n\
Git                    Git.Git                    2.56.0.4                         winget\n\
Python 3.14            Python.Python.3.14         3.14.10        Moniker: python3 winget\n\
Python 3.15            Python.Python.3.15         3.15.2         Moniker: python3 winget\n\
Node.js                OpenJS.NodeJS              26.12.1        Moniker: nodejs  winget\n\
7-Zip                  7zip.7zip                  24.09          Moniker: 7zip    winget\n\
Microsoft Visual C++ v14 Redistributable (x64) - 14.50.35719 Microsoft.VCRedist.2015+.x64 14.50.35719.0 14.52.37029.0 winget\n";

    #[test]
    fn parses_winget_table() {
        let rows = parse_winget_table(SEARCH_SAMPLE);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].id, "Git.Git");
        assert_eq!(rows[0].name, "Git");
        assert_eq!(rows[1].name, "Python 3.14");
        assert_eq!(rows[1].match_info, "Moniker: python3");
        assert_eq!(rows[5].id, "Microsoft.VCRedist.2015+.x64");
        assert_eq!(rows[5].name, "Microsoft Visual C++ v14 Redistributable (x64) - 14.50.35719");
    }

    #[test]
    fn chooses_exact_name() {
        let rows = parse_winget_table(SEARCH_SAMPLE);
        let chosen = choose_package(&rows, "git").unwrap();
        assert_eq!(chosen.id, "Git.Git");
    }

    #[test]
    fn prefers_official_self_titled_package() {
        let text = "\
名称      ID             版本          源\n\
--------------------------------------------------\n\
Git       Git.Git        2.56.0.4      winget\n\
Git       Microsoft.Git  2.56.1.0.1    winget\n";
        let rows = parse_winget_table(text);
        assert_eq!(rows.len(), 2);
        let chosen = choose_package(&rows, "git").unwrap();
        assert_eq!(chosen.id, "Git.Git");
    }

    #[test]
    fn chooses_moniker_highest_version() {
        let rows = parse_winget_table(SEARCH_SAMPLE);
        let chosen = choose_package(&rows, "python3").unwrap();
        assert_eq!(chosen.id, "Python.Python.3.15");
    }

    #[test]
    fn ambiguous_returns_error_message() {
        let rows = parse_winget_table(SEARCH_SAMPLE);
        assert!(choose_package(&rows, "zzz").is_none());
        let msg = ambiguous_message("zzz", &rows);
        assert!(msg.contains("apt install Git.Git"));
    }

    #[test]
    fn version_compare_works() {
        assert_eq!(version_cmp("3.15.2", "3.14.10"), Ordering::Greater);
        assert_eq!(version_cmp("1.2", "1.2.0"), Ordering::Equal);
    }

    #[test]
    fn global_options_are_extracted() {
        let args: Vec<String> = ["-y", "--source", "msstore", "install", "git"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (rest, opts) = parse_global(&args).unwrap();
        assert_eq!(rest, vec!["install", "git"]);
        assert_eq!(opts.source.as_deref(), Some("msstore"));
    }

    #[test]
    fn package_version_split() {
        assert_eq!(
            split_name_version("git=2.4.0"),
            ("git".to_string(), Some("2.4.0".to_string()))
        );
        assert_eq!(split_name_version("git"), ("git".to_string(), None));
    }

    #[test]
    fn no_upgrade_detection() {
        assert!(is_no_upgrade("没有可用的升级。"));
        assert!(is_no_upgrade("No applicable upgrade found."));
        assert!(!is_no_upgrade("installation failed"));
    }

    #[test]
    fn no_results_detection() {
        assert!(is_no_results("找不到与输入条件匹配的程序包。"));
        assert!(is_no_results("No package found matching input criteria."));
        assert!(!is_no_results("网络连接失败"));
    }
}
