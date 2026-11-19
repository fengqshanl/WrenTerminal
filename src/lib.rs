//! Wren —— 在 Windows 上提供 Linux 风格命令行体验的轻量兼容层。
//!
//! 模块划分：
//! - `pathutil`: POSIX <-> Windows 路径互转、~ 展开
//! - `env`: Linux 风格环境变量（PATH 用 `:` 分隔）与 Windows 环境同步
//! - `parser`: 分词、引号、变量展开、管道与重定向解析
//! - `builtins`: 用 Rust 原生实现的常用 Linux 命令
//! - `pkg`: apt 风格的软件包管理前端（winget / choco / scoop 后端）
//! - `winutil`: 查找程序、权限判断、控制台输出清理
//! - `translator`: 非内置命令的 Linux -> Windows 映射表
//! - `executor`: 管道、重定向与外部进程执行
//! - `shell`: REPL 循环与交互界面

pub mod builtins;
pub mod completer;
pub mod env;
pub mod executor;
pub mod parser;
pub mod pathutil;
pub mod pkg;
pub mod shell;
pub mod translator;
pub mod winutil;
