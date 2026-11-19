//! 交互式 Shell：REPL 循环、Linux 风格提示符、历史与启动界面。

use crate::builtins::{Context, Flow};
use crate::completer::ShellHelper;
use crate::env::Env;
use crate::executor;
use crate::parser;
use crate::pathutil;
use crate::update;
use colored::Colorize;
use rustyline::config::Builder;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::CompletionType;
use rustyline::Editor;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// 启动交互式 shell，返回进程退出码。
pub fn run() -> i32 {
    enable_windows_utf8();

    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "Wren".into());
    let env = Env::new(exe);
    let mut ctx = Context::new(env);

    // 默认始终进入交互模式（保证双击 / Start-Process 启动都能看到提示符）。
    // 只有显式传入 --batch / -b 时才走批处理模式（管道或脚本重定向）。
    // 注意：不能用 is_terminal() 自动判断——某些独立控制台窗口里该检测会失效，
    // 导致误入批处理并读到 EOF 立即退出，表现为“闪退”。
    let args: Vec<String> = std::env::args().skip(1).collect();
    let batch = args.iter().any(|a| a == "--batch" || a == "-b");
    if batch {
        return run_batch(&mut ctx);
    }

    print_banner(&ctx);

    // 启动更新检查：先读本地缓存（不阻塞），缓存过期才在后台访问 GitHub。
    let home = ctx.env.home_string();
    let current_version = env!("CARGO_PKG_VERSION");
    let startup = update::startup_status(&home, current_version);
    let mut announced_update: Option<String> = None;
    if let Some(info) = startup.notice {
        update::print_notice(&info, current_version);
        announced_update = Some(info.version);
    }
    let update_rx = if startup.needs_refresh {
        Some(update::spawn_refresh(
            home.clone(),
            current_version.to_string(),
            ctx.env.get_or("PATH", ""),
        ))
    } else {
        None
    };

    let cwd_shared = Arc::new(RwLock::new(ctx.cwd.clone()));
    let helper = ShellHelper::new(cwd_shared.clone(), &ctx.env);
    let config = Builder::new()
        .completion_type(CompletionType::List)
        .history_ignore_space(true)
        .history_ignore_dups(true)
        .unwrap_or_else(|_| Builder::new())
        .build();
    let mut rl: Editor<ShellHelper, FileHistory> = match Editor::with_config(config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("初始化行编辑器失败: {}", e);
            return 1;
        }
    };
    rl.set_helper(Some(helper));

    let hist_path = Path::new(&home).join(".wren_history");
    if rl.load_history(&hist_path).is_ok() {
        for entry in rl.history().iter() {
            ctx.history.push(entry.clone());
        }
    }

    loop {
        // 后台检查完成后，在下一次提示符前显示更新提示，避免打断正在输入的行。
        if let Some(rx) = &update_rx {
            while let Ok(info) = rx.try_recv() {
                if announced_update.as_deref() != Some(info.version.as_str()) {
                    update::print_notice(&info, current_version);
                    announced_update = Some(info.version);
                }
            }
        }
        let prompt = build_prompt(&ctx);
        match rl.readline(&prompt) {
            Ok(line) => {
                if line.trim().is_empty() { continue; }
                let _ = rl.add_history_entry(line.as_str());
                ctx.history.push(line.clone());

                let expanded = expand_alias(&line);
                match parser::parse(&expanded, &ctx.env) {
                    Ok(parsed) => {
                        if parsed.pipelines.is_empty() { continue; }
                        if let Ok(mut g) = cwd_shared.write() { *g = ctx.cwd.clone(); }
                        if executor::execute_line(&parsed, &mut ctx) == Flow::Exit {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("{} {}", "shell:".red(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^{} (使用 exit 退出)", "C".yellow());
            }
            Err(ReadlineError::Eof) => {
                println!("{}", "exit".dimmed());
                break;
            }
            Err(e) => {
                eprintln!("行编辑错误: {}", e);
                break;
            }
        }
    }

    let _ = rl.save_history(&hist_path);
    let _ = cwd_shared;
    println!("{}", "再见！".green());
    0
}

/// 批处理模式：从 stdin 逐行读取命令执行（用于管道输入或脚本重定向）。
fn run_batch(ctx: &mut Context) -> i32 {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        ctx.history.push(line.clone());
        let expanded = expand_alias(&line);
        match parser::parse(&expanded, &ctx.env) {
            Ok(parsed) => {
                if parsed.pipelines.is_empty() {
                    continue;
                }
                if executor::execute_line(&parsed, ctx) == Flow::Exit {
                    break;
                }
            }
            Err(e) => eprintln!("shell: {}", e),
        }
    }
    ctx.env.last_status()
}

/// 构造提示符（纯文本，不含 ANSI 颜色）。
///
/// 颜色由 completer 的 `highlight_prompt` 回调添加。必须保持纯文本，
/// 否则 rustyline 计算可见宽度时会把转义码算进去，导致输入光标偏移。
fn build_prompt(ctx: &Context) -> String {
    // 提示符使用 ASCII 用户名，避免宽字符导致的光标定位偏移。
    let user = ctx.env.get_or("USER_ASCII", "user");
    let host = ctx.env.get_or("COMPUTERNAME", "localhost");
    let display = pathutil::display(&ctx.cwd, Path::new(&ctx.env.home_string()));
    let sym = if user.eq_ignore_ascii_case("root") { "#" } else { "$" };
    format!("{}@{}:{}{} ", user, host, display, sym)
}

/// 常见别名（仅在行首展开）。
fn expand_alias(line: &str) -> String {
    let aliases: &[(&str, &str)] = &[
        ("ll", "ls -l"),
        ("la", "ls -a"),
        ("..", "cd .."),
        ("...", "cd ../.."),
    ];
    let trimmed = line.trim_start();
    let leading = &line[..line.len() - trimmed.len()];
    for (alias, expansion) in aliases {
        if trimmed == *alias {
            return format!("{}{}", leading, expansion);
        }
        if let Some(rest) = trimmed.strip_prefix(&format!("{} ", alias)) {
            return format!("{}{} {}", leading, expansion, rest);
        }
    }
    line.to_string()
}

fn print_banner(ctx: &Context) {
    let version = env!("CARGO_PKG_VERSION");
    let user = ctx.env.get_or("USER", "user");
    println!();
    println!("{}", "   _   _   ____    _____   _   _  ".green());
    println!("{}", " | | | | |  _ \\  | ____| | \\ | | ".green());
    println!("{}", " | | | | | |_) | |  _|   |  \\| | ".green());
    println!("{}", " | | | | |  _ <  | |___  | |\\  | ".green());
    println!("{}", " \\ \\_/ / | |_) | |_____| | | \\ | ".green());
    println!("{}", "  \\___/  |____/          |_|  \\_ ".green());
    println!();
    println!(
        "  {}  {}",
        format!("Wren v{}", version).bold(),
        "— A lightweight Linux-style shell on Windows (no WSL)".dimmed()
    );
    println!("  {} {} {}", "欢迎,".dimmed(), user.green(), "! 输入 help 查看可用命令，exit 退出。".dimmed());
    println!("  {} {}", "提示:".dimmed(), "支持管道 | · 重定向 > >> < · 变量 $VAR · 历史 ↑/↓ · 补全 Tab".yellow());
    println!();
}

/// Windows 控制台启用 UTF-8 代码页与虚拟终端（ANSI 颜色）支持。
#[cfg(windows)]
fn enable_windows_utf8() {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleOutputCP(code_page: u32) -> i32;
        fn SetConsoleCP(code_page: u32) -> i32;
        fn GetStdHandle(std_handle: u32) -> *mut c_void;
        fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: *mut c_void, mode: u32) -> i32;
    }
    unsafe {
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
        const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
        const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0u32;
        if GetConsoleMode(handle, &mut mode) != 0 {
            SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

#[cfg(not(windows))]
fn enable_windows_utf8() {}
