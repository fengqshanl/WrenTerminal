//! 命令翻译层：把 Linux 命令/参数映射为 Windows 等价形式。
//!
//! 这里维护一张可扩展的映射表。内置命令由 `builtins` 直接执行，
//! 其余命令在交给 `executor` 启动子进程前先经过本模块翻译。

/// 一条简单的命令名映射。
const COMMAND_MAP: &[(&str, &str)] = &[
    // 系统 / 网络
    ("ifconfig", "ipconfig"),
    ("ip", "ipconfig"),
    ("route", "route"),
    ("netstat", "netstat"),
    ("ps", "tasklist"),
    ("pkill", "taskkill"),
    ("killall", "taskkill"),
    ("top", "taskmgr"),
    ("htop", "taskmgr"),
    ("uname", "uname"),
    // 文本处理（内置已覆盖 grep，这里作为兜底）
    ("grep", "findstr"),
    ("egrep", "findstr"),
    ("fgrep", "findstr"),
    ("clear-screen", "cls"),
    // 开发工具
    ("python3", "python"),
    ("py3", "py"),
    ("cc", "gcc"),
    ("make", "make"),
    ("g++", "g++"),
    // 打开 / 桌面集成
    ("open", "open"),
    ("xdg-open", "open"),
    ("start", "start"),
    // 包管理 / shell 工具（给出提示）
    ("man", "man"),
    ("yum", "yum"),
    ("dnf", "dnf"),
    ("brew", "brew"),
    ("ln", "ln"),
    ("sed", "sed"),
    ("awk", "awk"),
    // 常用跨平台工具（原样透传，Windows 10+ 自带）
    ("git", "git"),
    ("curl", "curl"),
    ("wget", "curl"),
    ("ssh", "ssh"),
    ("scp", "scp"),
    ("ping", "ping"),
    ("nslookup", "nslookup"),
    ("where", "where"),
];

/// 翻译结果。
#[derive(Debug, Clone)]
pub struct Translation {
    pub program: String,
    pub args: Vec<String>,
    /// 给用户的提示（例如某些命令在 Windows 上语义不同）。
    pub note: Option<String>,
}

/// 命令名 -> Windows 可执行程序名（找不到时返回原命令名）。
pub fn map_program(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    COMMAND_MAP
        .iter()
        .find(|(from, _)| *from == lower)
        .map(|(_, to)| (*to).to_string())
        .unwrap_or_else(|| name.to_string())
}

/// 已知命令名列表（用于 Tab 补全）。
pub fn known_commands() -> Vec<String> {
    let mut v: Vec<String> = COMMAND_MAP.iter().map(|(f, _)| f.to_string()).collect();
    v.sort();
    v.dedup();
    v
}

/// 判断命令是否在映射表中。
pub fn is_known(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    COMMAND_MAP.iter().any(|(f, _)| *f == lower)
}

/// 把一条外部命令翻译为 Windows 等价命令。
pub fn translate(program: &str, args: &[String]) -> Translation {
    let lower = program.to_ascii_lowercase();

    match lower.as_str() {
        // findstr 的参数风格：Linux -i/-v/-n/-r -> Windows /I /V /N /R
        "findstr" | "grep" | "egrep" | "fgrep" => {
            let mut out = Vec::new();
            let mut note = None;
            for a in args {
                match a.as_str() {
                    "-i" | "--ignore-case" => out.push("/I".into()),
                    "-v" | "--invert-match" => out.push("/V".into()),
                    "-n" | "--line-number" => out.push("/N".into()),
                    "-r" | "-E" | "--regexp" => out.push("/R".into()),
                    "-l" | "--files-with-matches" => out.push("/M".into()),
                    "-c" if args.len() > 1 => out.push(a.clone()),
                    _ => out.push(a.clone()),
                }
            }
            if lower != "grep" && lower != "egrep" && lower != "fgrep" {
                note = Some("提示：findstr 的功能弱于 grep，复杂正则建议使用内置 grep".into());
            }
            Translation {
                program: "findstr".into(),
                args: out,
                note,
            }
        }

        // open / xdg-open -> cmd /c start "" <target>
        "open" | "xdg-open" => {
            if args.is_empty() {
                Translation {
                    program: "cmd".into(),
                    args: vec!["/c".into(), "start".into(), "".into()],
                    note: Some("用法：open <文件|目录|URL>".into()),
                }
            } else {
                let mut out = vec!["/c".into(), "start".into(), "".into()];
                out.extend(args.iter().cloned());
                Translation {
                    program: "cmd".into(),
                    args: out,
                    note: None,
                }
            }
        }

        // killall / pkill name -> taskkill /F /IM name
        "killall" | "pkill" => {
            let mut out = vec!["/F".into(), "/IM".into()];
            out.extend(args.iter().cloned());
            Translation {
                program: "taskkill".into(),
                args: out,
                note: Some("提示：Windows 下按进程名结束任务（taskkill /F /IM）".into()),
            }
        }

        // ln -s target link -> cmd /c mklink link target
        "ln" => {
            if args.iter().any(|a| a == "-s" || a == "--symbolic") {
                let rest: Vec<String> = args
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .collect();
                if rest.len() >= 2 {
                    let target = &rest[0];
                    let link = &rest[1];
                    Translation {
                        program: "cmd".into(),
                        args: vec![
                            "/c".into(),
                            "mklink".into(),
                            link.clone(),
                            target.clone(),
                        ],
                        note: Some("提示：已翻译为 Windows mklink（符号链接）".into()),
                    }
                } else {
                    Translation {
                        program: "cmd".into(),
                        args: vec!["/c".into(), "mklink".into(), "/?".into()],
                        note: Some("用法：ln -s <目标> <链接名>".into()),
                    }
                }
            } else {
                Translation {
                    program: "cmd".into(),
                    args: vec!["/c".into(), "mklink".into(), "/H".into()],
                    note: Some("提示：Windows 硬链接使用 mklink /H".into()),
                }
            }
        }

        // 没有 Windows 原生等价的命令：给出说明，避免用户困惑。
        "man" => Translation {
            program: "help".into(),
            args: vec![],
            note: Some(format!(
                "提示：{} 在本环境中映射为内置 help；可用 `help <命令>` 查看内置命令",
                program
            )),
        },
        "yum" | "dnf" | "brew" => Translation {
            program: "echo".into(),
            args: vec![],
            note: Some(format!(
                "提示：{} 是 Linux/macOS 包管理器；Wren 中请使用 apt（后端为 winget），例如：apt install <软件包>",
                program
            )),
        },
        "top" | "htop" => Translation {
            program: "taskmgr".into(),
            args: vec![],
            note: Some("提示：已打开 Windows 任务管理器（top 的等价工具）".into()),
        },
        "sed" | "awk" => Translation {
            program: "echo".into(),
            args: vec![],
            note: Some(format!(
                "提示：{} 无直接 Windows 等价；可用 PowerShell 或安装 GNU 工具（Git Bash / MSYS2）",
                program
            )),
        },
        "wget" => {
            let mut out: Vec<String> = vec!["-L".into()];
            let mut has_output = false;
            let mut i = 0;
            while i < args.len() {
                let a = &args[i];
                match a.as_str() {
                    "-O" | "--output-document" => {
                        out.push("-o".into());
                        if let Some(v) = args.get(i + 1) {
                            out.push(v.clone());
                            has_output = true;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    "-q" | "--quiet" => {
                        out.push("-s".into());
                        i += 1;
                    }
                    "-nv" | "--no-verbose" => {
                        out.push("-sS".into());
                        i += 1;
                    }
                    "-c" | "--continue" => {
                        out.push("-C".into());
                        out.push("-".into());
                        i += 1;
                    }
                    "--no-check-certificate" => {
                        out.push("-k".into());
                        i += 1;
                    }
                    "-P" | "--directory-prefix" => {
                        out.push("--output-dir".into());
                        if let Some(v) = args.get(i + 1) {
                            out.push(v.clone());
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    _ if a.starts_with("--output-document=") => {
                        out.push("-o".into());
                        out.push(a["--output-document=".len()..].to_string());
                        has_output = true;
                        i += 1;
                    }
                    _ if a.starts_with("--header=") => {
                        out.push("-H".into());
                        out.push(a["--header=".len()..].to_string());
                        i += 1;
                    }
                    _ if a.starts_with("--user-agent=") => {
                        out.push("-A".into());
                        out.push(a["--user-agent=".len()..].to_string());
                        i += 1;
                    }
                    _ => {
                        out.push(a.clone());
                        i += 1;
                    }
                }
            }
            if !has_output {
                // wget 默认按 URL 中的文件名保存，curl 用 -O 表达同样行为。
                out.push("-O".into());
            }
            Translation {
                program: "curl".into(),
                args: out,
                note: Some(
                    "提示：wget 已映射为 curl -L（默认保存到当前目录，可用 -O 指定文件名）"
                        .into(),
                ),
            }
        }

        _ => Translation {
            program: map_program(program),
            args: args.to_vec(),
            note: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn wget_maps_output_and_quiet() {
        let t = translate(
            "wget",
            &args(&["-q", "-O", "out.zip", "https://example.com/file.zip"]),
        );
        assert_eq!(t.program, "curl");
        assert!(t.args.contains(&"-L".to_string()));
        assert!(t.args.contains(&"-s".to_string()));
        assert!(t.args.contains(&"-o".to_string()));
        assert!(t.args.contains(&"out.zip".to_string()));
        // -O 已在 wget 参数中出现，不应再追加 curl 的 -O。
        assert!(!t.args.contains(&"-O".to_string()));
    }

    #[test]
    fn wget_defaults_to_remote_name() {
        let t = translate("wget", &args(&["https://example.com/file.zip"]));
        assert_eq!(t.program, "curl");
        assert!(t.args.contains(&"-O".to_string()));
    }

    #[test]
    fn wget_continue_maps_to_curl_range() {
        let t = translate("wget", &args(&["-c", "https://example.com/file.zip"]));
        let pos = t.args.iter().position(|a| a == "-C").unwrap();
        assert_eq!(t.args.get(pos + 1).map(|s| s.as_str()), Some("-"));
    }
}
