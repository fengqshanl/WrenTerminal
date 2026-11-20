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
    ("kill", "taskkill"),
    ("ss", "netstat"),
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

/// 额外参与 Tab 补全的命令名。
///
/// 这些命令要么由 translate() 专门处理，要么由 Git Bash 后备提供。
const EXTRA_COMMANDS: &[&str] = &[
    "sed", "awk", "sort", "uniq", "cut", "tr", "tee", "xargs", "stat", "du", "df", "chmod",
    "chown", "readlink", "realpath", "basename", "dirname", "md5sum", "sha1sum", "sha256sum",
    "base64", "gzip", "gunzip", "bzip2", "xz", "tar", "diff", "patch", "file", "xxd", "od",
    "split", "paste", "join", "comm", "cmp", "nl", "fold", "fmt", "expand", "unexpand", "tac",
    "shuf", "mktemp", "timeout", "less", "vim", "nano", "unzip", "id", "groups", "nproc",
    "locale", "iconv", "dos2unix", "unix2dos", "systemctl", "service", "journalctl", "dmesg",
    "crontab", "iptables", "ufw", "useradd", "usermod", "userdel", "groupadd", "groupdel",
    "su", "lsblk", "lscpu", "lsusb", "lspci", "dmidecode", "watch", "whois", "bc", "nc",
    "netcat", "dpkg", "rpm", "snap", "flatpak", "pgrep", "pidof", "lsof", "dig", "host",
    "traceroute", "tracepath", "zip",
];

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
    v.extend(EXTRA_COMMANDS.iter().map(|s| s.to_string()));
    v.sort();
    v.dedup();
    v
}

/// 判断命令是否在映射表中。
pub fn is_known(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    COMMAND_MAP.iter().any(|(f, _)| *f == lower)
        || EXTRA_COMMANDS.iter().any(|f| *f == lower)
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
        "ip" => translate_ip(args),
        "ifconfig" => translate_ifconfig(args),
        "ps" => translate_ps(args),
        "kill" => translate_kill(args),
        "netstat" => translate_netstat(args),
        "ss" => translate_ss(args),
        "ping" => translate_ping(args),
        "systemctl" => translate_systemctl(args),
        "service" => translate_service(args),
        "pgrep" | "pidof" => translate_pgrep(args, program),
        "lsof" => Translation {
            program: "netstat".into(),
            args: vec!["-ano".into()],
            note: Some("提示：lsof 已近似映射为 netstat -ano（仅显示连接与 PID）".into()),
        },
        "dig" | "host" => {
            let out: Vec<String> = args
                .iter()
                .filter(|a| !a.starts_with('+'))
                .cloned()
                .collect();
            Translation {
                program: "nslookup".into(),
                args: out,
                note: Some("提示：dig/host 已映射为 nslookup（+short 等选项被忽略）".into()),
            }
        }
        "traceroute" | "tracepath" => Translation {
            program: "tracert".into(),
            args: args.to_vec(),
            note: Some("提示：traceroute 已映射为 tracert（参数格式不同）".into()),
        },
        "zip" => {
            let mut out = vec!["-a".to_string(), "-c".to_string(), "-f".to_string()];
            out.extend(args.iter().cloned());
            Translation {
                program: "tar".into(),
                args: out,
                note: Some("提示：zip 已映射为 Windows tar -a（创建 zip 包）".into()),
            }
        }
        "journalctl" | "dmesg" => linux_hint(
            program,
            "请使用事件查看器 eventvwr.msc 或 PowerShell Get-WinEvent",
        ),
        "crontab" => linux_hint(
            "crontab",
            "计划任务请使用 schtasks 或“任务计划程序”",
        ),
        "iptables" | "ufw" => linux_hint(
            program,
            "防火墙请使用 netsh advfirewall 或 Windows Defender 防火墙",
        ),
        "useradd" | "usermod" | "userdel" | "groupadd" | "groupdel" => linux_hint(
            program,
            "请使用 net user / net localgroup 或 PowerShell 的 *-LocalUser / *-LocalGroup",
        ),
        "su" => linux_hint("su", "Windows 使用 runas 或右键“以管理员身份运行”"),
        "lsblk" => linux_hint("lsblk", "请使用 PowerShell Get-Disk / Get-Partition / Get-Volume"),
        "lscpu" => linux_hint("lscpu", "请使用 PowerShell Get-CimInstance Win32_Processor"),
        "lsusb" => linux_hint("lsusb", "请使用 PowerShell Get-PnpDevice -Class USB"),
        "lspci" => linux_hint("lspci", "请使用 PowerShell Get-PnpDevice"),
        "dmidecode" => linux_hint(
            "dmidecode",
            "请使用 PowerShell Get-CimInstance Win32_BIOS / Win32_BaseBoard",
        ),
        "watch" => linux_hint("watch", "可使用 PowerShell 循环，或安装 Git Bash"),
        "whois" => linux_hint("whois", "可使用在线 whois 服务"),
        "bc" => linux_hint("bc", "请使用 PowerShell 计算表达式"),
        "nc" | "netcat" => linux_hint(program, "网络连通性可用 PowerShell Test-NetConnection"),
        "dpkg" | "rpm" | "snap" | "flatpak" => {
            linux_hint(program, "Windows 请使用 apt（winget 后端）")
        }
        "git" => {
            // 让 git 直接输出 UTF-8 文件路径，而不是 "docs/\350\277..." 这种转义。
            let mut out = vec!["-c".to_string(), "core.quotepath=false".to_string()];
            out.extend(args.iter().cloned());
            Translation {
                program: "git".into(),
                args: out,
                note: None,
            }
        }

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

fn unsupported(what: &str, alternative: &str) -> Translation {
    Translation {
        program: "echo".into(),
        args: vec![],
        note: Some(format!("提示：Windows 不支持 {}；{}", what, alternative)),
    }
}

fn linux_hint(what: &str, alternative: &str) -> Translation {
    unsupported(what, alternative)
}

fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

fn systemctl_usage() -> Translation {
    Translation {
        program: "echo".into(),
        args: vec![],
        note: Some(
            "用法: systemctl <status|start|stop|restart|enable|disable|is-active|list-units> [服务名]"
                .into(),
        ),
    }
}

fn translate_systemctl(args: &[String]) -> Translation {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("status");
    let name = args.get(1).map(|s| ps_quote(s));
    let note = Some(
        "提示：systemctl 已映射为 PowerShell 服务命令（Windows 服务名与 Linux 不同）".into(),
    );

    match sub {
        "status" => {
            let script = match &name {
                Some(n) => format!("Get-Service -Name '{}' | Format-List *", n),
                None => "Get-Service | Sort-Object Status, DisplayName | Format-Table -AutoSize"
                    .to_string(),
            };
            Translation {
                program: "powershell".into(),
                args: vec!["-NoProfile".into(), "-Command".into(), script],
                note,
            }
        }
        "start" | "stop" | "restart" => {
            let Some(n) = name else {
                return systemctl_usage();
            };
            let verb = match sub {
                "start" => "Start-Service",
                "stop" => "Stop-Service",
                _ => "Restart-Service",
            };
            let script = format!(
                "{} -Name '{}'; Get-Service -Name '{}' | Format-Table -AutoSize",
                verb, n, n
            );
            Translation {
                program: "powershell".into(),
                args: vec!["-NoProfile".into(), "-Command".into(), script],
                note,
            }
        }
        "enable" | "disable" => {
            let Some(n) = name else {
                return systemctl_usage();
            };
            let kind = if sub == "enable" { "Automatic" } else { "Manual" };
            Translation {
                program: "powershell".into(),
                args: vec![
                    "-NoProfile".into(),
                    "-Command".into(),
                    format!("Set-Service -Name '{}' -StartupType {}", n, kind),
                ],
                note: Some(
                    "提示：systemctl enable/disable 已映射为 Set-Service -StartupType（可能需要管理员权限）"
                        .into(),
                ),
            }
        }
        "is-active" => {
            let Some(n) = name else {
                return systemctl_usage();
            };
            Translation {
                program: "powershell".into(),
                args: vec![
                    "-NoProfile".into(),
                    "-Command".into(),
                    format!("(Get-Service -Name '{}').Status", n),
                ],
                note,
            }
        }
        "list-units" | "list-unit-files" | "list" => Translation {
            program: "powershell".into(),
            args: vec![
                "-NoProfile".into(),
                "-Command".into(),
                "Get-Service | Sort-Object Status, DisplayName | Format-Table -AutoSize".into(),
            ],
            note,
        },
        "daemon-reload" => Translation {
            program: "echo".into(),
            args: vec![],
            note: Some("提示：Windows 服务不需要 systemctl daemon-reload".into()),
        },
        _ => systemctl_usage(),
    }
}

fn translate_service(args: &[String]) -> Translation {
    if args.len() == 1 && (args[0] == "--status-all" || args[0] == "status") {
        return translate_systemctl(&["list-units".into()]);
    }
    if args.len() >= 2 {
        let name = args[0].clone();
        // `len() >= 2` 已经保证存在最后一个参数；这里用 if-let 再兜底一次，
        // 以后若调整上面的判断也不会因为 unwrap 而 panic。
        if let Some(action) = args.last().cloned() {
            if matches!(
                action.as_str(),
                "start" | "stop" | "restart" | "status" | "enable" | "disable" | "is-active"
            ) {
                return translate_systemctl(&[action, name]);
            }
        }
    }
    Translation {
        program: "echo".into(),
        args: vec![],
        note: Some(
            "用法: service <服务名> <start|stop|restart|status> 或 service --status-all".into(),
        ),
    }
}

fn translate_pgrep(args: &[String], label: &str) -> Translation {
    let mut name = None;
    for a in args {
        if a == "-f" || a == "--full" {
            return unsupported(
                &format!("{} -f", label),
                "Windows 无法按完整命令行筛选，请使用 tasklist /FI 或 PowerShell Get-Process",
            );
        }
        if a.starts_with('-') {
            continue;
        }
        name = Some(a.clone());
        break;
    }
    let Some(name) = name else {
        return Translation {
            program: "echo".into(),
            args: vec![],
            note: Some(format!("用法: {} <进程名>", label)),
        };
    };
    let image = if name.to_ascii_lowercase().ends_with(".exe") {
        name
    } else {
        format!("{}.exe", name)
    };
    Translation {
        program: "tasklist".into(),
        args: vec!["/FI".into(), format!("IMAGENAME eq {}", image)],
        note: Some(format!(
            "提示：{} 已映射为 tasklist /FI \"IMAGENAME eq {}\"",
            label, image
        )),
    }
}

fn translate_ip(args: &[String]) -> Translation {
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        // ip -c/--color 可能带一个参数
        if args[i] == "-c" || args[i] == "-color" {
            i += 2;
        } else {
            i += 1;
        }
    }
    let sub = args.get(i).map(|s| s.as_str()).unwrap_or("");
    let rest: &[String] = if i + 1 < args.len() {
        &args[i + 1..]
    } else {
        &[]
    };
    let changing = rest.iter().any(|a| {
        matches!(
            a.as_str(),
            "add" | "del" | "delete" | "replace" | "change" | "flush" | "set" | "append"
        )
    });

    match sub {
        "" | "a" | "addr" | "address" | "addrlabel" => Translation {
            program: "ipconfig".into(),
            args: vec![],
            note: Some("提示：ip address 已映射为 ipconfig".into()),
        },
        "link" | "l" => {
            if changing {
                unsupported(
                    "ip link set/add/del",
                    "请使用 PowerShell 的 Get-NetAdapter / Set-NetAdapter 或 netsh interface",
                )
            } else {
                Translation {
                    program: "ipconfig".into(),
                    args: vec!["/all".into()],
                    note: Some("提示：ip link 已映射为 ipconfig /all".into()),
                }
            }
        }
        "route" | "r" | "ro" => {
            if changing {
                unsupported(
                    "ip route 的修改操作",
                    "请使用 route add/delete 或 PowerShell New-NetRoute / Remove-NetRoute",
                )
            } else {
                Translation {
                    program: "route".into(),
                    args: vec!["print".into()],
                    note: Some("提示：ip route 已映射为 route print".into()),
                }
            }
        }
        "neigh" | "neighbor" | "n" => {
            if changing {
                unsupported(
                    "ip neigh 的修改操作",
                    "请使用 netsh interface ip add/delete neighbors",
                )
            } else {
                Translation {
                    program: "arp".into(),
                    args: vec!["-a".into()],
                    note: Some("提示：ip neigh 已映射为 arp -a".into()),
                }
            }
        }
        "help" => Translation {
            program: "echo".into(),
            args: vec![],
            note: Some(
                "ip: 支持 address / link / route / neigh；在 Windows 下分别映射为 \
                 ipconfig / ipconfig /all / route print / arp -a"
                    .into(),
            ),
        },
        other => unsupported(
            &format!("ip {}", other),
            "可用子命令为 address / link / route / neigh",
        ),
    }
}

fn translate_ifconfig(args: &[String]) -> Translation {
    if args.iter().any(|a| a == "up" || a == "down") {
        return unsupported(
            "ifconfig <接口> up/down",
            "请使用 netsh interface set interface（需要管理员权限）",
        );
    }
    let all = args.iter().any(|a| a == "-a" || a == "--all");
    Translation {
        program: "ipconfig".into(),
        args: if all { vec!["/all".into()] } else { vec![] },
        note: Some(if all {
            "提示：ifconfig -a 已映射为 ipconfig /all".into()
        } else {
            "提示：ifconfig 已映射为 ipconfig（显示所有网卡）".into()
        }),
    }
}

fn translate_ps(args: &[String]) -> Translation {
    let mut tasklist_args = Vec::new();
    let mut note = "提示：ps 已映射为 tasklist（Windows 进程列表，参数格式不同）".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "--pid" => {
                if let Some(pid) = args.get(i + 1) {
                    tasklist_args.push("/FI".into());
                    tasklist_args.push(format!("PID eq {}", pid));
                    note = format!("提示：ps -p {} 已映射为 tasklist /FI \"PID eq {}\"", pid, pid);
                    i += 2;
                    continue;
                }
            }
            "-C" | "--comm" => {
                if let Some(name) = args.get(i + 1) {
                    let image = if name.to_ascii_lowercase().ends_with(".exe") {
                        name.clone()
                    } else {
                        format!("{}.exe", name)
                    };
                    tasklist_args.push("/FI".into());
                    tasklist_args.push(format!("IMAGENAME eq {}", image));
                    note = format!(
                        "提示：ps -C {} 已映射为 tasklist /FI \"IMAGENAME eq {}\"",
                        name, image
                    );
                    i += 2;
                    continue;
                }
            }
            "--help" | "-h" => {
                return Translation {
                    program: "echo".into(),
                    args: vec![],
                    note: Some(
                        "用法: ps [aux|-ef|-p PID|-C 进程名]（Windows 下映射为 tasklist）".into(),
                    ),
                };
            }
            _ => {}
        }
        i += 1;
    }
    Translation {
        program: "tasklist".into(),
        args: tasklist_args,
        note: Some(note),
    }
}

fn is_force_signal(signal: &str) -> bool {
    let upper = signal.trim().to_ascii_uppercase();
    matches!(upper.as_str(), "KILL" | "SIGKILL" | "9" | "SIG9")
}

fn translate_kill(args: &[String]) -> Translation {
    let mut force = false;
    let mut check_only = false;
    let mut list = false;
    let mut pids = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-l" || a == "--list" {
            list = true;
            i += 1;
            continue;
        }
        if a == "-s" || a == "--signal" {
            if let Some(sig) = args.get(i + 1) {
                if sig == "0" {
                    check_only = true;
                } else if is_force_signal(sig) {
                    force = true;
                }
                i += 2;
                continue;
            }
        }
        if a == "-0" || a.eq_ignore_ascii_case("-SIG0") {
            check_only = true;
            i += 1;
            continue;
        }
        if a == "-9" || a.eq_ignore_ascii_case("-KILL") || a.eq_ignore_ascii_case("-SIGKILL") {
            force = true;
            i += 1;
            continue;
        }
        if a.starts_with('-') {
            // 其它 POSIX 信号在 Windows 上没有对应语义，忽略。
            i += 1;
            continue;
        }
        pids.push(a.clone());
        i += 1;
    }

    if list {
        return Translation {
            program: "echo".into(),
            args: vec![],
            note: Some(
                "HUP INT QUIT ILL TRAP ABRT BUS FPE KILL USR1 SEGV USR2 PIPE ALRM TERM CHLD \
                 CONT STOP TSTP TTIN TTOU URG XCPU XFSZ VTALRM PROF WINCH IO PWR SYS"
                    .into(),
            ),
        };
    }
    if pids.is_empty() {
        return Translation {
            program: "echo".into(),
            args: vec![],
            note: Some("kill: 用法: kill [-9] <PID>...".into()),
        };
    }
    if check_only {
        return Translation {
            program: "echo".into(),
            args: vec![],
            note: Some(format!(
                "提示：Windows 不支持 kill -0；可用 tasklist /FI \"PID eq {}\" 检查进程是否存在",
                pids[0]
            )),
        };
    }
    if let Some(bad) = pids.iter().find(|p| !p.chars().all(|c| c.is_ascii_digit())) {
        return Translation {
            program: "echo".into(),
            args: vec![],
            note: Some(format!("kill: 无效 PID '{}'", bad)),
        };
    }

    let mut out = Vec::new();
    if force {
        out.push("/F".into());
    }
    for pid in &pids {
        out.push("/PID".into());
        out.push(pid.clone());
    }
    Translation {
        program: "taskkill".into(),
        args: out,
        note: Some(if force {
            "提示：已映射为 taskkill /F /PID（强制结束）".into()
        } else {
            "提示：已映射为 taskkill /PID（请求进程关闭）".into()
        }),
    }
}

fn translate_netstat(args: &[String]) -> Translation {
    let mut out = Vec::new();
    let mut converted = false;
    for a in args {
        if a.starts_with('-') && a.len() > 1 {
            let flags = &a[1..];
            // Linux 的 -t/-u/-l 在 Windows netstat 中没有对应含义，
            // 常见的 -tulpn 统一转成显示所有连接和 PID 的 -ano。
            if flags.chars().any(|c| matches!(c, 't' | 'u' | 'l')) {
                out = vec!["-ano".into()];
                converted = true;
                continue;
            }
        }
        out.push(a.clone());
    }
    Translation {
        program: "netstat".into(),
        args: out,
        note: if converted {
            Some("提示：Linux netstat 参数已转换为 Windows 的 -ano".into())
        } else {
            None
        },
    }
}

fn translate_ss(args: &[String]) -> Translation {
    let mut t = translate_netstat(args);
    t.note = Some(match t.note {
        Some(note) => format!("提示：ss 已映射为 netstat；{}", note),
        None => "提示：ss 已映射为 netstat".into(),
    });
    t
}

fn translate_ping(args: &[String]) -> Translation {
    let mut out = Vec::new();
    let mut converted = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-c" | "--count" => {
                if let Some(v) = args.get(i + 1) {
                    out.push("-n".into());
                    out.push(v.clone());
                    converted = true;
                    i += 2;
                } else {
                    out.push(args[i].clone());
                    i += 1;
                }
            }
            "-s" | "--size" => {
                if let Some(v) = args.get(i + 1) {
                    out.push("-l".into());
                    out.push(v.clone());
                    converted = true;
                    i += 2;
                } else {
                    out.push(args[i].clone());
                    i += 1;
                }
            }
            "-W" | "--timeout" => {
                if let Some(v) = args.get(i + 1) {
                    if let Ok(secs) = v.parse::<u64>() {
                        out.push("-w".into());
                        out.push(secs.saturating_mul(1000).to_string());
                        converted = true;
                        i += 2;
                    } else {
                        out.push(args[i].clone());
                        i += 1;
                    }
                } else {
                    out.push(args[i].clone());
                    i += 1;
                }
            }
            "-i" | "--interval" => {
                // Linux 的发包间隔在 Windows ping 中没有等价选项。
                if i + 1 < args.len() {
                    converted = true;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "-t" | "--ttl" => {
                if let Some(v) = args.get(i + 1) {
                    if v.parse::<u64>().is_ok() {
                        out.push("-i".into());
                        out.push(v.clone());
                        i += 2;
                    } else {
                        i += 1;
                    }
                    converted = true;
                } else {
                    converted = true;
                    i += 1;
                }
            }
            _ => {
                out.push(args[i].clone());
                i += 1;
            }
        }
    }
    Translation {
        program: "ping".into(),
        args: out,
        note: if converted {
            Some("提示：ping 参数已转换为 Windows 格式（-c → -n，-s → -l，-W → -w）".into())
        } else {
            None
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

    #[test]
    fn ip_address_maps_to_ipconfig() {
        let t = translate("ip", &args(&["address"]));
        assert_eq!(t.program, "ipconfig");
        assert!(t.args.is_empty());
        let t = translate("ip", &args(&["-br", "addr", "show"]));
        assert_eq!(t.program, "ipconfig");
    }

    #[test]
    fn ip_route_maps_to_route_print() {
        let t = translate("ip", &args(&["route"]));
        assert_eq!(t.program, "route");
        assert_eq!(t.args, vec!["print"]);
    }

    #[test]
    fn ip_neigh_maps_to_arp() {
        let t = translate("ip", &args(&["neigh"]));
        assert_eq!(t.program, "arp");
        assert_eq!(t.args, vec!["-a"]);
    }

    #[test]
    fn ps_maps_to_tasklist() {
        let t = translate("ps", &args(&["aux"]));
        assert_eq!(t.program, "tasklist");
        assert!(t.args.is_empty());

        let t = translate("ps", &args(&["-p", "1234"]));
        assert_eq!(t.program, "tasklist");
        assert_eq!(t.args, vec!["/FI", "PID eq 1234"]);
    }

    #[test]
    fn kill_maps_to_taskkill() {
        let t = translate("kill", &args(&["-9", "123"]));
        assert_eq!(t.program, "taskkill");
        assert_eq!(t.args, vec!["/F", "/PID", "123"]);
    }

    #[test]
    fn kill_zero_is_not_a_kill() {
        let t = translate("kill", &args(&["-0", "123"]));
        assert_eq!(t.program, "echo");
    }

    #[test]
    fn netstat_linux_flags_become_ano() {
        let t = translate("netstat", &args(&["-tulpn"]));
        assert_eq!(t.program, "netstat");
        assert_eq!(t.args, vec!["-ano"]);
    }

    #[test]
    fn ping_count_maps_to_n() {
        let t = translate("ping", &args(&["-c", "4", "example.com"]));
        assert_eq!(t.program, "ping");
        assert_eq!(t.args, vec!["-n", "4", "example.com"]);
    }

    #[test]
    fn ifconfig_all_maps_to_ipconfig_all() {
        let t = translate("ifconfig", &args(&["-a"]));
        assert_eq!(t.program, "ipconfig");
        assert_eq!(t.args, vec!["/all"]);
    }

    #[test]
    fn systemctl_maps_to_powershell() {
        let t = translate("systemctl", &args(&["status", "sshd"]));
        assert_eq!(t.program, "powershell");
        assert!(t.args.iter().any(|a| a.contains("Get-Service -Name 'sshd'")));

        let t = translate("service", &args(&["sshd", "start"]));
        assert_eq!(t.program, "powershell");
        assert!(t.args.iter().any(|a| a.contains("Start-Service")));
    }

    #[test]
    fn pgrep_maps_to_tasklist_filter() {
        let t = translate("pgrep", &args(&["ssh"]));
        assert_eq!(t.program, "tasklist");
        assert_eq!(t.args, vec!["/FI", "IMAGENAME eq ssh.exe"]);
    }

    #[test]
    fn dig_maps_to_nslookup() {
        let t = translate("dig", &args(&["+short", "example.com"]));
        assert_eq!(t.program, "nslookup");
        assert_eq!(t.args, vec!["example.com"]);
    }

    #[test]
    fn zip_maps_to_tar() {
        let t = translate("zip", &args(&["a.zip", "a.txt"]));
        assert_eq!(t.program, "tar");
        assert_eq!(t.args, vec!["-a", "-c", "-f", "a.zip", "a.txt"]);
    }

    #[test]
    fn git_disables_path_quoting() {
        let t = translate("git", &args(&["status"]));
        assert_eq!(t.program, "git");
        assert_eq!(t.args[0], "-c");
        assert_eq!(t.args[1], "core.quotepath=false");
        assert_eq!(t.args[2], "status");
    }
}
