# Wren

> **Wren**（鹪鹩）—— 一只小巧、敏捷、轻快的鸟。以此命名这个**轻量、无负担**的 Windows Linux 风格命令行兼容层。

一个用 **Rust** 从零实现的、运行在 **Windows** 上的 Linux 风格终端与命令行兼容层。

从 **v0.3.0** 起，Wren 提供一个**独立的 Windows 桌面应用**（`wren-app.exe`）：顶部常驻系统状态栏，多标签终端可连接本地 Wren Shell、任意 WSL 发行版以及任意标准 SSH 服务器；登录后状态栏会自动切换并实时显示对应系统的 CPU、内存、磁盘、网络、负载与运行时长。桌面应用不依赖 Windows Terminal，也不需要 WSL 才能运行本地命令。

命令行部分**不依赖 WSL、Cygwin 或 MSYS2**。它提供一个 bash 风格的交互式提示符，把常见 Linux 命令用 Rust 原生实现，并把其它命令自动翻译为 Windows 等价命令执行，同时维护一套 Linux 风格的环境变量（PATH 用 `:` 分隔、HOME/USER 等）并同步给子进程。

像鹪鹩一样：**体积小、启动快、不挑环境**——单个可执行文件，即下即用。

---

## 快速开始

```bash
# 编译（需要 Rust 工具链）
cargo build --release

# 启动桌面应用（推荐）
.\target\release\wren-app.exe
# 或双击 run.bat（优先启动桌面应用）

# 只使用命令行 Shell
.\target\release\wren.exe

# 批处理方式运行脚本（需显式加 --batch）
.\target\release\wren.exe --batch < script.sh
echo "ls -la" | .\target\release\wren.exe --batch
```

> 说明：命令行 Shell 默认始终进入交互式 shell（确保双击/独立窗口启动都能看到提示符）。
> 只有显式传入 `--batch`（或 `-b`）才走批处理模式。若崩溃，错误信息会写入
> `%TEMP%\wren_panic.log`，便于排查。
>
> 桌面应用是纯 GUI 程序（Windows 子系统），不会附带控制台窗口；崩溃与运行日志写入
> `%APPDATA%\Wren\wren-app.log`。

### 安装（MSI）

从 [Releases](https://github.com/fengqshanl/WrenTerminal/releases) 下载 `Wren-*-x64.msi` 双击安装即可。安装程序会：

- 把 `wren-app.exe`（桌面应用）和 `wren.exe`（命令行 Shell）安装到 `C:\Program Files\Wren`
- 创建桌面 / 开始菜单的 **Wren** 快捷方式（启动桌面应用）以及 **Wren 命令行** 快捷方式
- 把安装目录加入系统 PATH
- **自动把 "Wren Terminal" 注册为 Windows Terminal 的一个 profile**（正式版与预览版都支持，所有用户账户）——安装完后新开一个 Terminal 标签页，下拉菜单里就能直接选到 Wren，无需手动执行任何脚本
- 卸载时自动清理该 profile

> 这个自动集成由 MSI 内置的自定义操作完成（安装时以 SYSTEM 权限遍历各用户的 Terminal 配置），对应的脚本是 `installer/staging/wren-terminal-profile.ps1`；日志写在 `%TEMP%\wren-terminal-profile.log`。
> 如果你用的是免安装的绿色版（直接拷可执行文件），可以手动运行该脚本：`powershell -ExecutionPolicy Bypass -File wren-terminal-profile.ps1 -Mode Install`。

命令行版本启动后你会看到熟悉的提示符：

```
   _   _   ____    _____   _   _
 | | | | |  _ \  | ____| | \ | |
 | | | | | |_) | |  _|   |  \| |
 | | | | |  _ <  | |___  | |\  |
 \ \_/ / | |_) | |_____| | | \ |
  \___/  |____/          |_|  \_

 Wren v0.3.1  — A lightweight Linux-style shell for Windows
 欢迎, zhangzhe ! 输入 help 查看可用命令，exit 退出。
 提示: 支持管道 | · 重定向 > >> < · 变量 $VAR · 历史 ↑/↓ · 补全 Tab

zhangzhe@ZHE:~$ echo hello
hello
zhangzhe@ZHE:~$
```

---

## 桌面应用（Wren App）

`wren-app.exe` 是一个独立的 Windows 桌面终端，不依赖 Windows Terminal，也不需要 WSL 即可运行本地 Wren 命令。

### 固定状态栏
- 顶部常驻，不随终端滚动；始终显示当前会话的目标系统
- 指标：CPU 使用率（含每核详情）、内存 / 交换分区、系统盘 / 根分区、网络实时速率、负载、运行时长、进程数、SSH 时延、本地时间
- **登录 WSL 或 SSH 后自动切换**：状态栏会显示远端 `/proc` 中的真实数据，而不是本机数据
- 点击 **详情**（或按 F12）展开右侧面板：CPU 型号与每核心占用、完整磁盘分区、网卡地址与速率、Top 进程、远端时间与时钟偏差、数据更新时间
- 数据刷新：本地 1.5 秒、WSL 4 秒、SSH 8 秒

### 多标签终端
- 本地 Wren Shell、WSL 发行版、任意标准 SSH 服务器可以同时开在多个标签页
- 标签页显示连接状态点；会话退出后保留最后的屏幕内容，可直接“重新连接”
- 终端支持 ANSI 16/256 色、中文宽字符、鼠标选择复制、Ctrl+C / Ctrl+V、滚轮与右侧可拖动的历史滚动条
- 窗口右下角状态与快捷键：
  - `Ctrl+Shift+T` 新建本地终端
  - `Ctrl+Shift+N` 打开连接管理器
  - `Ctrl+Shift+W` 关闭当前标签
  - `Ctrl+Tab` / `Ctrl+Shift+Tab` 切换标签
  - `Alt+1…9` 跳到指定标签
  - `Ctrl+=` / `Ctrl+-` / `Ctrl+0` 调整终端字号
  - `F12` 展开/收起系统详情
  - 状态栏 **置顶** 按钮可手动切换窗口置顶（默认关闭，不会自动置顶）

### 连接 WSL
1. 点击右上角 **＋ 连接** 或按 `Ctrl+Shift+N`
2. 选择 **WSL 发行版**，应用会自动运行 `wsl.exe -l -v` 检测已安装的发行版
3. 点击发行版右侧的 **连接**，或使用“默认发行版”

WSL 会话以 `wsl.exe -d <distro> --cd ~` 启动；状态栏通过 `wsl.exe ... sh -c` 读取远端 `/proc`，无需额外配置。

### 连接 SSH 服务器
1. 打开连接管理器，切换到 **SSH 服务器**
2. 点击 **＋ 新建 SSH 连接**，填写名称、主机、用户名、端口（默认 22），可选私钥路径
3. 保存后即可一键连接；配置保存在 `%APPDATA%\Wren\app.json`

SSH 使用系统自带的 `ssh.exe`，支持密码、密钥、ssh-agent 与首次连接的主机指纹确认。若希望顶部状态栏显示远端系统数据，需要**免密登录**（SSH 密钥或 ssh-agent）：状态采集会为每次轮询建立一个独立的 `ssh` 连接，无法在非交互模式下输入密码。状态栏会显示“状态离线”并在详情面板给出提示；终端本身不受影响。

> 远端状态采集依赖 Linux 的 `/proc`，适用于绝大多数 Linux 服务器。非 Linux 目标仍可正常打开终端，只是状态栏可能显示为离线。

### 配置与日志
- 配置：`%APPDATA%\Wren\app.json`（SSH 服务器、窗口大小、字号、置顶、详情面板状态）
- 日志：`%APPDATA%\Wren\wren-app.log`
- 终端组件基于 [alacritty_terminal](https://github.com/alacritty/alacritty) 与 [egui_term](https://github.com/Harzu/egui-term)（MIT），已按本项目需要做了 Windows 线程与输入修复，许可证见 `src/bin/wren-app/LICENSE-egui_term`。

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
- 上下方向键翻阅历史、Tab 命令/文件/变量补全（命令补全包含 PATH 中的可执行文件）
- 启动横幅、Ctrl-C/Ctrl-D 处理
- 历史持久化到 `~/.wren_history`
- 提示符传**纯文本**给行编辑器、颜色由高亮回调添加；用户名显示为 ASCII 拼音（真实中文用户名仍保留给 `whoami`），彻底避免 Windows 控制台宽字符导致的光标定位偏移

### 2. 命令解析
- 管道 `|`、输入重定向 `<`
- 输出重定向 `>` `>>`，错误重定向 `2>` `2>>` `2>&1`，合并重定向 `&>` `&>>`
- 连接符 `&&`、`||`、`;`
- 通配符展开 `*` `?` `[a-z]`（未加引号时，与 bash 一致；引号内的通配符按字面量处理）
- 命令替换 `$(命令)` 与反引号 `` `命令` ``，在子 shell 中执行，`cd` 不会影响主 shell
- 单引号 / 双引号、反斜杠转义，支持引号拼接（`a"b c"d` → `ab cd`）
- 变量展开 `$VAR`、`${VAR}`、`$?`（上一条命令的退出码）、`$$`（PID）、`~`
- `/tmp` 映射到 Windows 临时目录，`/dev/null` 映射到 `NUL`

### 3. 内置命令（Rust 原生实现）
`help` `exit` `clear` `pwd` `cd` `ls` `echo` `cat` `head` `tail` `wc` `grep` `find`
`mkdir` `rmdir` `rm` `cp` `mv` `touch` `export` `unset` `env` `which` `history`
`uname` `whoami` `hostname` `date` `seq` `true` `false` `type` `tree` `sleep`
`apt` `apt-get` `update` `man` `nproc` `free` `uptime` `alias` `unalias`

别名：`ll` → `ls -l`，`la` → `ls -a`，`..` → `cd ..`，`...` → `cd ../..`，`cls` → `clear`

自定义别名会保存到 `~/.wren_aliases`，下次启动自动加载：

```bash
alias gs='git status'
alias ll='ls -l'
unalias gs
alias                # 列出所有别名
```

`cd` 会像 bash 一样做词法归一化，`cd ../..` 后提示符显示 `/d/code` 而不是 `/d/code/app/../..`。

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

### 5. 可选的 Git Bash 后备（大幅扩展 GNU 工具覆盖）

Wren 本身不依赖 WSL / Cygwin / MSYS2，但如果你安装了 **Git for Windows**
（很多开发机都有），Wren 会自动发现它的 `bash.exe`，并在以下场景把命令交给
Git Bash 执行：

- Windows PATH 中找不到的命令
- Windows 有同名工具、但语义不同的 GNU 命令，例如：
  `sed` `awk` `sort` `uniq` `cut` `tr` `tee` `xargs` `stat` `du` `df`
  `chmod` `chown` `readlink` `realpath` `basename` `dirname`
  `md5sum` `sha1sum` `sha256sum` `base64` `gzip` `gunzip` `bzip2` `xz`
  `tar` `diff` `patch` `file` `xxd` `od` `split` `paste` `join`
  `comm` `cmp` `nl` `fold` `fmt` `expand` `unexpand` `tac` `shuf`
  `mktemp` `timeout` `less` `vim` `nano` `unzip` `id` `groups` `iconv` …

```bash
sed -i 's/foo/bar/g' file.txt
awk '{print $2}' data.txt
sort -k2 -n data.txt | uniq -c
tar -czf backup.tar.gz src/
md5sum README.md
df -h
```

- `npm` / `pnpm` / `yarn` / `npx` 等 `.cmd` shim 会自动经 `cmd /C` 启动，可直接使用
- `git` 自动带上 `-c core.quotepath=false`，中文文件名以 UTF-8 显示，而不是 `\345...` 转义
- `ls` 按 CJK 显示宽度对齐中英文混合文件名
- 关闭后备：`export WREN_NO_GITBASH=1`
- 指定自定义 bash：`export WREN_GIT_BASH='D:\Tools\Git\bin\bash.exe'`

> 说明：这能把常见 GNU 工具覆盖得比较完整，但 Windows 不是 Linux。
> systemd、iptables、内核模块、`/proc`、APT/dpkg 等 Linux 专有机制没有等价物，
> 这类场景仍建议使用 WSL。所谓“95% 命令”取决于日常命令集合：常规文件、文本、
> 网络和开发工具基本可用，系统管理类命令需要逐个确认。

### 6. Wren 自身更新

- 交互式启动时读取 `~/.wren_update_check` 缓存；如果 GitHub Releases 上有更新，
  会在启动横幅后提示（例如）：
  ```
  发现 Wren 新版本 v0.2.2（当前 v0.2.1）
  运行 `update` 下载并安装，或访问 https://github.com/fengqshanl/WrenTerminal/releases
  ```
- 缓存 24 小时；过期后在后台检查，不阻塞提示符。网络失败缓存 6 小时。
- 相关命令：
  ```bash
  update --check    # 只检查最新版本
  update            # 下载并安装（会先确认）
  update -y         # 自动确认（批处理模式必需）
  ```
- 更新流程：下载 MSI 到 `%TEMP%` → 校验文件头 → 请求 UAC → 等待当前 Wren 退出 →
  `msiexec /passive` 安装。安装完成后重新打开 Wren。
- 关闭启动检查：`export WREN_NO_UPDATE_CHECK=1`
- 更新源：[GitHub Releases](https://github.com/fengqshanl/WrenTerminal/releases)

### 7. Linux → Windows 命令翻译
非内置命令在启动子进程前自动翻译，映射表见 `src/translator.rs`：

| Linux 命令 | Windows 等价 |
|---|---|
| `ip address` / `ip addr` | `ipconfig` |
| `ip link` | `ipconfig /all` |
| `ip route` | `route print` |
| `ip neigh` | `arp -a` |
| `ifconfig` / `ifconfig -a` | `ipconfig` / `ipconfig /all` |
| `ps` / `ps aux` / `ps -ef` | `tasklist` |
| `ps -p PID` | `tasklist /FI "PID eq PID"` |
| `kill -9 PID` | `taskkill /F /PID PID` |
| `killall` / `pkill` | `taskkill /F /IM` |
| `netstat -tulpn` / `ss -tulpn` | `netstat -ano` |
| `ping -c N` | `ping -n N` |
| `top` / `htop` | `taskmgr` |
| `open` / `xdg-open` | `cmd /c start ""` |
| `ln -s` | `cmd /c mklink` |
| `python3` | `python` |
| `wget` | `curl -L`（含参数转换） |
| `grep`（内置兜底） | `findstr`（含参数转换） |
| `yum` / `dnf` / `brew` | 提示改用 `apt` |

未命中的命令原样透传；如果安装了 Git for Windows，还会自动交给 Git Bash。
因此 `git`、`curl`、`npm`、`pnpm`、`dir`、`ipconfig` 等可直接调用，
`sed` / `awk` / `tar` / `sort` 等 GNU 工具也可用。

### 8. 路径与环境
- `/c/Users/foo` ↔ `C:\Users\foo` 自动互转；`~` 展开为主目录
- PATH 在内部以 `:` 展示，启动子进程时转回 `;`
- `export` 的变量立即同步到进程环境，子进程可见

---

## 项目结构

```
src/
├── main.rs          # 命令行 Shell 入口 + panic 日志钩子
├── lib.rs           # 命令行模块声明
├── bin/wren-app/    # 独立桌面应用
│   ├── main.rs      # GUI 入口、字体、panic/日志
│   ├── app.rs       # 应用状态、标签页、会话生命周期、快捷键
│   ├── ui.rs        # 顶部状态栏、详情面板、标签栏、终端区域、主题
│   ├── connect.rs   # 本地 / WSL / SSH 连接窗口与 SSH 配置表单
│   ├── session.rs   # 会话创建（wren.exe / wsl.exe / ssh.exe）
│   ├── stats.rs     # 本地 sysinfo 与远端 /proc 状态采集
│   ├── config.rs    # %APPDATA%\Wren\app.json 持久化
│   ├── view.rs 等   # 基于 egui_term / alacritty_terminal 的终端部件
├── shell.rs         # REPL 主循环、提示符、启动界面、批处理模式、Windows 控制台初始化
├── parser.rs        # 分词、引号、变量展开、管道/重定向/连接符解析
├── globber.rs       # shell 风格通配符展开（* ? [a-z]，含隐藏文件规则）
├── builtins.rs      # 内置命令实现 + 上下文(Env/cwd/history) + 输出类型
├── pkg.rs           # apt 风格包管理前端（winget / choco / scoop）
├── update.rs        # 启动更新检查、update 命令、MSI 下载与安装
├── winutil.rs       # 程序查找、Git Bash 检测、权限判断、控制台输出清理
├── translator.rs    # Linux → Windows 命令映射表（可扩展）
├── executor.rs      # 管道/重定向串联、外部进程启动、.cmd shim 与 Git Bash 后备
├── env.rs           # Linux 风格环境变量与 Windows 进程环境同步
├── pathutil.rs      # POSIX ↔ Windows 路径互转、~ 展开
└── completer.rs     # Tab 补全（命令/文件/变量）
```

---

## 设计说明与边界

这是一个**兼容层 / 便利工具**，不是一个完整的 Linux 用户态。需要如实说明几点：

1. **命令覆盖**：内置命令覆盖日常高频操作；Linux 有成千上万个独立程序，无法全部内置。未内置的命令走“翻译 + 透传 + 可选 Git Bash 后备”策略——能映射的映射，不能映射的交给 Windows；装了 Git for Windows 时再交给 Git Bash。**“95% 命令”不是可保证的硬指标**：常规文件、文本、网络、开发工具覆盖较好，systemd / iptables / 内核 / `/proc` / APT 等 Linux 专有机制仍需 WSL。
2. **语义差异**：部分内置命令是简化实现（如 `grep` 是子串匹配而非完整正则，`find` 仅支持 `-name`）。需要完整 GNU 行为时，安装 Git for Windows 后 Wren 会优先使用 `sed` / `awk` / `sort` / `tar` 等 GNU 工具；更完整的 Linux 用户态请用 WSL。
3. **外部程序输出编码**：Windows 原生命令输出的中文在部分场景下可能受代码页影响；交互模式已尝试启用 UTF-8 与虚拟终端。
4. **包管理**：`apt` 是 winget / Chocolatey / Scoop 的前端，不维护 Ubuntu 的 `.deb` 仓库与依赖树；软件包名称、版本和可用性由 Windows 包源决定，和 Ubuntu 并不一一对应。
5. **权限**：Windows 没有真正的 `sudo`；`sudo <命令>` 会以当前权限直接执行。需要管理员权限的安装程序通常会弹出 UAC，失败时请右键“以管理员身份运行” Wren。
6. **自更新**：`update` 依赖 GitHub Releases 上有比当前版本更新的正式 Release；启动检查每 24 小时最多访问一次 GitHub API，可用 `WREN_NO_UPDATE_CHECK=1` 关闭。
7. **路径**：POSIX 绝对路径需写成 `/c/...`、`/d/...` 形式；`/usr/bin` 这类映射到当前盘根目录（仅为便利，不代表真实存在）。

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

当前包含解析器、路径转换、拼音用户名映射、winget 表格解析、`sudo` 参数解析、
`ip` / `ps` / `kill` / `ping` 参数翻译、更新缓存与版本比较、Git Bash 后备等单元测试，全部通过。

---

## 名称变更说明

本项目原名 **linuxTerminal**，v0.1.0 起正式更名为 **Wren**。更名涉及：

- 包名与可执行文件：`linux_terminal` → `wren`（`wren.exe`）
- 历史文件：`~/.linux_terminal_history` → `~/.wren_history`
- 崩溃日志：`%TEMP%\linux_terminal_panic.log` → `%TEMP%\wren_panic.log`
- 启动脚本：`run.bat` 现指向 `wren.exe`

> 注：物理项目目录仍为 `D:\code\linuxTerminal`（这是工作区路径，改名会影响启动脚本与预览，故保留）。
