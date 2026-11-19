# Wren

> **Wren**（鹪鹩）—— 一只小巧、敏捷、轻快的鸟。以此命名这个**轻量、无负担**的 Windows Linux 风格命令行兼容层。

一个用 **Rust** 从零实现的、运行在 **Windows 原生控制台**上的 Linux 风格命令行兼容层。

**不依赖 WSL、Cygwin 或 MSYS2**。它提供一个 bash 风格的交互式提示符，把常见 Linux 命令用 Rust 原生实现，并把其它命令自动翻译为 Windows 等价命令执行，同时维护一套 Linux 风格的环境变量（PATH 用 `:` 分隔、HOME/USER 等）并同步给子进程。

像鹪鹩一样：**体积小、启动快、不挑环境**——单个可执行文件，即下即用。

---

## 快速开始

```bash
# 编译（需要 Rust 工具链）
cargo build --release

# 运行交互式 shell（最简单：双击 run.bat，退出时窗口会保留）
run.bat
# 或直接
.\target\release\wren.exe

# 批处理方式运行脚本（需显式加 --batch）
.\target\release\wren.exe --batch < script.sh
echo "ls -la" | .\target\release\wren.exe --batch
```

> 说明：默认始终进入交互式 shell（确保双击/独立窗口启动都能看到提示符）。
> 只有显式传入 `--batch`（或 `-b`）才走批处理模式。若崩溃，错误信息会写入
> `%TEMP%\wren_panic.log`，便于排查。

### 安装（MSI）与 Windows Terminal 集成

从 [Releases](https://github.com/fengqshanl/WrenTerminal/releases) 下载 `Wren-*-x64.msi` 双击安装即可。安装程序会：

- 把 `wren.exe` 安装到 `C:\Program Files\Wren` 并加入系统 PATH
- 创建开始菜单与桌面快捷方式
- **自动把 "Wren Terminal" 注册为 Windows Terminal 的一个 profile**（正式版与预览版都支持，所有用户账户）——安装完后新开一个 Terminal 标签页，下拉菜单里就能直接选到 Wren，无需手动执行任何脚本
- 卸载时自动清理该 profile

> 这个自动集成由 MSI 内置的自定义操作完成（安装时以 SYSTEM 权限遍历各用户的 Terminal 配置），对应的脚本是 `installer/staging/wren-terminal-profile.ps1`；日志写在 `%TEMP%\wren-terminal-profile.log`。
> 如果你用的是免安装的绿色版（直接拷 `wren.exe`），可以手动运行该脚本：`powershell -ExecutionPolicy Bypass -File wren-terminal-profile.ps1 -Mode Install`。

进入后你会看到熟悉的提示符：

```
   _   _   ____    _____   _   _
 | | | | |  _ \  | ____| | \ | |
 | | | | | |_) | |  _|   |  \| |
 | | | | |  _ <  | |___  | |\  |
 \ \_/ / | |_) | |_____| | | \ |
  \___/  |____/          |_|  \_

 Wren v0.2.0  — A lightweight Linux-style shell on Windows (no WSL)
 欢迎, zhangzhe ! 输入 help 查看可用命令，exit 退出。
 提示: 支持管道 | · 重定向 > >> < · 变量 $VAR · 历史 ↑/↓ · 补全 Tab

zhangzhe@ZHE:~$ echo hello
hello
zhangzhe@ZHE:~$
```

---

## 为什么叫 Wren

鹪鹩（Wren）是地球上最小巧的鸟类之一，却以**活泼、机敏、鸣声嘹亮**著称。这个名字寄托了这个项目的设计取向：

- **轻**：单个可执行文件（约 1MB），无需安装运行时或依赖链
- **快**：Rust 原生实现常用命令，启动与响应迅速
- **灵活**：不挑环境，直接在 Windows 原生控制台安家，不依赖 WSL/虚拟机

---

## 功能特性

### 1. Linux 风格交互界面
- 彩色提示符 `user@host:~/path$`（root 显示 `#`）
- 上下方向键翻阅历史、Tab 命令/文件/变量补全
- 启动横幅、Ctrl-C/Ctrl-D 处理
- 历史持久化到 `~/.wren_history`
- 提示符传**纯文本**给行编辑器、颜色由高亮回调添加；用户名显示为 ASCII 拼音（真实中文用户名仍保留给 `whoami`），彻底避免 Windows 控制台宽字符导致的光标定位偏移

### 2. 命令解析
- 管道 `|`、重定向 `>` `>>` `<`
- 连接符 `&&`、`||`、`;`
- 单引号 / 双引号、反斜杠转义
- 变量展开 `$VAR`、`${VAR}`、`$?`（上一条退出码）、`$$`（PID）、`~`

### 3. 内置命令（Rust 原生实现）
`help` `exit` `clear` `pwd` `cd` `ls` `echo` `cat` `head` `tail` `wc` `grep` `find`
`mkdir` `rmdir` `rm` `cp` `mv` `touch` `export` `unset` `env` `which` `history`
`uname` `whoami` `hostname` `date` `seq` `true` `false` `type` `tree` `sleep`
`apt` `apt-get` `sudo`

别名：`ll` → `ls -l`，`la` → `ls -a`，`..` → `cd ..`，`...` → `cd ../..`，`cls` → `clear`

### 4. apt 风格软件包管理

内置 `apt` / `apt-get`，把 Ubuntu 的命令习惯带到 Windows。后端优先使用
Windows 自带的 **winget**（找不到时自动回退到 Chocolatey / Scoop）：

| 命令 | 作用 |
|---|---|
| `apt update` | 刷新软件包索引（`winget source update`） |
| `apt search <关键词>` | 搜索软件包 |
| `apt install <包...>` | 安装软件（自动把 `git`、`python3`、`7zip` 等名称解析为包 ID） |
| `apt show <包>` | 查看软件包信息 |
| `apt list --installed` | 列出已安装软件 |
| `apt list --upgradable` | 列出可升级软件 |
| `apt upgrade [包...]` | 升级全部或指定软件 |
| `apt remove <包...>` | 卸载软件（`purge` / `uninstall` 同义） |
| `apt download <包...> [-d 目录]` | 只下载安装程序，不安装 |

```bash
apt update
apt search git
apt install git
apt install python3          # 自动解析到 Python.Python.3.x
apt download 7zip -d ~/Downloads
sudo apt install 7zip        # sudo 是透明前缀，实际直接执行 apt
```

- 支持 `apt install 包名=版本`、`-y` / `--yes`、`--source winget|msstore`
- `apt upgrade`（不带包名）在交互模式下会先确认；批处理模式需要加 `-y`
- 包名与 Ubuntu 并不完全等价：Wren 会先搜索，再按
  “精确 ID → 精确名称 → 精确 moniker → 唯一搜索结果” 的顺序选择，
  有歧义时会列出候选包 ID
- 交互式终端里会显示 winget 的实时下载/安装进度；在管道或批处理中自动捕获输出
- `sudo` 在 Windows 上没有真正的用户切换：`sudo <命令>` 会以当前权限直接执行，
  需要管理员权限的安装程序通常会自动弹出 UAC，失败时请右键“以管理员身份运行” Wren

### 5. Linux → Windows 命令翻译
非内置命令在启动子进程前自动翻译，映射表见 `src/translator.rs`：

| Linux 命令 | Windows 等价 |
|---|---|
| `ifconfig` / `ip` | `ipconfig` |
| `ps` | `tasklist` |
| `killall` / `pkill` | `taskkill /F /IM` |
| `top` / `htop` | `taskmgr` |
| `open` / `xdg-open` | `cmd /c start ""` |
| `ln -s` | `cmd /c mklink` |
| `python3` | `python` |
| `wget` | `curl` |
| `grep`（兜底） | `findstr`（含参数转换） |
| `yum` / `dnf` / `man` / `sed` 等 | 给出 Windows 替代方案提示（`apt` 与 `sudo` 已是内置命令，见上） |

未命中的命令原样透传，因此 `git`、`curl`、`npm`、`dir`、`ipconfig` 等 Windows 原生/跨平台工具可直接调用。

### 6. 路径与环境
- `/c/Users/foo` ↔ `C:\Users\foo` 自动互转；`~` 展开为主目录
- PATH 在内部以 `:` 展示，启动子进程时转回 `;`
- `export` 的变量立即同步到进程环境，子进程可见

---

## 项目结构

```
src/
├── main.rs          # 程序入口 + panic 日志钩子
├── lib.rs           # 模块声明
├── shell.rs         # REPL 主循环、提示符、启动界面、批处理模式、Windows 控制台初始化
├── parser.rs        # 分词、引号、变量展开、管道/重定向/连接符解析
├── builtins.rs      # 内置命令实现 + 上下文(Env/cwd/history) + 输出类型
├── pkg.rs           # apt 风格包管理前端（winget / choco / scoop）
├── winutil.rs       # 程序查找、管理员权限判断、控制台输出清理
├── translator.rs    # Linux → Windows 命令映射表（可扩展）
├── executor.rs      # 管道/重定向串联、外部进程启动、环境变量注入
├── env.rs           # Linux 风格环境变量与 Windows 进程环境同步
├── pathutil.rs      # POSIX ↔ Windows 路径互转、~ 展开
└── completer.rs     # Tab 补全（命令/文件/变量）
```

---

## 设计说明与边界

这是一个**兼容层 / 便利工具**，不是一个完整的 Linux 用户态。需要如实说明几点：

1. **命令覆盖**：内置命令覆盖日常高频操作；Linux 有成千上万个独立程序，无法全部内置。未内置的命令走"翻译 + 透传"策略——能映射的映射，不能映射的直接交给 Windows（若系统没装则提示未找到）。
2. **语义差异**：部分命令是简化实现（如 `grep` 是子串匹配而非完整正则，`find` 仅支持 `-name`）。复杂的 GNU 工具行为请用 Git Bash / MSYS2 / WSL。
3. **外部程序输出编码**：Windows 原生命令输出的中文在部分场景下可能受代码页影响；交互模式已尝试启用 UTF-8 与虚拟终端。
4. **包管理**：`apt` 是 winget / Chocolatey / Scoop 的前端，不维护 Ubuntu 的 `.deb` 仓库与依赖树；软件包名称、版本和可用性由 Windows 包源决定，和 Ubuntu 并不一一对应。
5. **权限**：Windows 没有真正的 `sudo`；`sudo <命令>` 会以当前权限直接执行。需要管理员权限的安装程序通常会弹出 UAC，失败时请右键“以管理员身份运行” Wren。
6. **路径**：POSIX 绝对路径需写成 `/c/...`、`/d/...` 形式；`/usr/bin` 这类映射到当前盘根目录（仅为便利，不代表真实存在）。

---

## 如何扩展

**添加命令翻译**：在 `src/translator.rs` 的 `COMMAND_MAP` 增加条目，或在 `translate()` 的 `match` 中加专门分支处理参数改写。

```rust
// 例：新增映射
("mytool", "mytool.exe"),
```

**添加内置命令**：
1. 在 `src/builtins.rs` 的 `BUILTINS` 列表登记（用于 help/补全）
2. 在 `is_builtin()` 的匹配中加入命令名
3. 在 `run()` 的分发 `match` 中加入分支并实现 `cmd_xxx()` 函数

---

## 测试

```bash
cargo test
```

当前包含解析器、路径转换、拼音用户名映射、winget 表格解析、`sudo` 参数解析等单元测试，全部通过。

---

## 名称变更说明

本项目原名 **linuxTerminal**，v0.1.0 起正式更名为 **Wren**。更名涉及：

- 包名与可执行文件：`linux_terminal` → `wren`（`wren.exe`）
- 历史文件：`~/.linux_terminal_history` → `~/.wren_history`
- 崩溃日志：`%TEMP%\linux_terminal_panic.log` → `%TEMP%\wren_panic.log`
- 启动脚本：`run.bat` 现指向 `wren.exe`

> 注：物理项目目录仍为 `D:\code\linuxTerminal`（这是工作区路径，改名会影响启动脚本与预览，故保留）。
