//! Wren —— Windows 上的 Linux 风格命令行兼容层（轻量、轻快）。

use std::io::Write;

fn main() {
    // 把 panic 写入日志文件，避免独立窗口里崩溃时信息一闪而过看不到。
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();
        let msg = format!(
            "\n==== Wren panic ====\n[{}] thread={} {}\n",
            chrono::Local::now().format("%F %T"),
            thread,
            info
        );
        let log = std::env::temp_dir().join("wren_panic.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
        {
            let _ = f.write_all(msg.as_bytes());
            let _ = f.flush();
        }
        let _ = std::io::stderr().write_all(msg.as_bytes());
    }));

    // 兜底：即使某个子系统出现未预期的 panic，也给出可读提示并返回非零退出码，
    // 而不是让进程直接崩溃、独立窗口瞬间消失。
    let code = match std::panic::catch_unwind(wren::shell::run) {
        Ok(code) => code,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "未知内部错误".to_string());
            eprintln!("\nwren: 发生内部错误，已安全退出。");
            eprintln!("原因: {detail}");
            eprintln!(
                "完整信息已写入: {}",
                std::env::temp_dir().join("wren_panic.log").display()
            );
            1
        }
    };
    std::process::exit(code);
}
