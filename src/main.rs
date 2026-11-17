//! Wren —— Windows 上的 Linux 风格命令行兼容层（轻量、轻快）。

use std::io::Write;

fn main() {
    // 把 panic 写入日志文件，避免独立窗口里崩溃时信息一闪而过看不到。
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("\n==== Wren panic ====\n{}\n", info);
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

    let code = wren::shell::run();
    std::process::exit(code);
}
