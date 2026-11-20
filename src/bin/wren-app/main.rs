//! Wren 桌面应用入口。
//!
//! 这个二进制是独立的 Windows 应用窗口：顶部常驻系统状态栏，下方是多标签终端，
//! 可以连接本地 Wren Shell、任意 WSL 发行版，以及任意 SSH 服务器。

// 桌面应用始终使用 Windows 子系统：双击启动不会附带控制台窗口，
// 也不会因为控制台被关闭而退出。崩溃信息写到 %APPDATA%\Wren\wren-app.log。
#![windows_subsystem = "windows"]
// 终端组件（backend/bindings/view 等）是从 egui_term 完整纳入的，
// 其中鼠标上报、自定义绑定等能力保留给后续扩展。
#![allow(dead_code)]

mod app;
mod backend;
mod bindings;
mod config;
mod connect;
mod font;
mod format;
mod session;
mod stats;
mod theme;
mod types;
mod ui;
mod view;

use std::io::Write;

/// 把 panic 与运行日志写到 `%APPDATA%\Wren\wren-app.log`。
/// GUI 程序在 Release 下没有控制台，崩溃信息只能落到文件里。
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();
        let msg = format!(
            "[{}] panic (thread={}): {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            thread,
            info
        );
        if let Some(path) = config::log_file() {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = f.write_all(msg.as_bytes());
                let _ = f.flush();
            }
        }
        let _ = std::io::stderr().write_all(msg.as_bytes());
    }));
}

fn install_fonts(ctx: &eframe::egui::Context) {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;

    let mut fonts = FontDefinitions::default();

    // Windows 上使用等宽字体 Cascadia Mono / Consolas；缺失时退回 egui 自带 Hack。
    let mono = [
        r"C:\Windows\Fonts\CascadiaMono.ttf",
        r"C:\Windows\Fonts\consola.ttf",
    ]
    .iter()
    .find_map(|p| std::fs::read(p).ok());
    if let Some(data) = mono {
        fonts
            .font_data
            .insert("wren-mono".into(), Arc::new(FontData::from_owned(data)));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
            list.insert(0, "wren-mono".into());
        }
    }

    // 界面字体优先使用 Segoe UI，贴近 Windows 原生观感。
    if let Ok(data) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts
            .font_data
            .insert("wren-ui".into(), Arc::new(FontData::from_owned(data)));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "wren-ui".into());
        }
    }

    // 中文回退字体：DengXian 是 TTF（egui 对 TTC 支持有限），几乎所有 Win10/11 都有。
    let cjk = [
        r"C:\Windows\Fonts\Deng.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
    ]
    .iter()
    .find_map(|p| std::fs::read(p).ok());
    if let Some(data) = cjk {
        fonts
            .font_data
            .insert("wren-cjk".into(), Arc::new(FontData::from_owned(data)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("wren-cjk".into());
            }
        }
    }

    ctx.set_fonts(fonts);
}

fn load_icon() -> Option<eframe::egui::IconData> {
    // 图标同时被 MSI 快捷方式使用；这里嵌入可执行文件，保证窗口/任务栏图标一致。
    let bytes: &[u8] = include_bytes!("../../../installer/staging/wren.ico");
    let dir = ico::IconDir::read(std::io::Cursor::new(bytes)).ok()?;
    let entry = dir
        .entries()
        .iter()
        .max_by_key(|e| e.width().saturating_mul(e.height()))?;
    let image = entry.decode().ok()?;
    Some(eframe::egui::IconData {
        rgba: image.rgba_data().to_vec(),
        width: image.width(),
        height: image.height(),
    })
}

fn main() -> eframe::Result<()> {
    install_panic_hook();

    // 让 PTY 里的 shell / ssh / wsl 知道终端支持颜色。
    std::env::set_var("TERM", "xterm-256color");
    std::env::set_var("COLORTERM", "truecolor");

    let cfg = config::AppConfig::load();
    let initial_size = cfg
        .window_size
        .map(|[w, h]| [w.max(880.0), h.max(540.0)])
        .unwrap_or([1280.0, 820.0]);

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Wren")
        .with_inner_size(initial_size)
        .with_min_inner_size([880.0, 540.0]);
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    // 启动失败（显卡驱动异常、窗口创建失败等）时记录日志并返回错误，
    // 而不是让进程无声无息地消失。
    let result = eframe::run_native(
        "Wren",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(app::WrenApp::new(cc, cfg)))
        }),
    );
    if let Err(e) = &result {
        config::log(&format!("桌面应用启动/运行失败: {e}"));
        let _ = std::io::stderr().write_all(format!("Wren 启动失败: {e}\n").as_bytes());
    }
    result
}

