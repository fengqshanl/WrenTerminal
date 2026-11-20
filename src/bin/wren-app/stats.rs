//! 系统状态采集：本地（sysinfo）与远端（WSL / SSH 上读取 /proc）。
//!
//! 每个终端会话对应一个后台采集线程，把最新快照放进 `StatusHandle`，
//! UI 只读快照，不阻塞渲染；会话关闭时线程会自行退出。

use crate::config::SshProfile;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// 一条保存下来的 SSH 服务器配置在采集时需要的目标信息。
#[derive(Debug, Clone)]
pub enum RemoteTarget {
    Wsl { distro: Option<String> },
    Ssh { profile: SshProfile },
}

impl RemoteTarget {
    pub fn label(&self) -> String {
        match self {
            RemoteTarget::Wsl { distro } => distro
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| "默认发行版".into()),
            RemoteTarget::Ssh { profile } => profile.target(),
        }
    }

    fn is_ssh(&self) -> bool {
        matches!(self, RemoteTarget::Ssh { .. })
    }
}

#[derive(Debug, Clone, Default)]
pub struct CpuCore {
    pub usage: f32,
}

#[derive(Debug, Clone, Default)]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub file_system: String,
    pub removable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NetIface {
    pub name: String,
    pub addrs: Vec<String>,
    pub rx_rate: f64,
    pub tx_rate: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub mem_pct: f32,
}

/// 状态栏与详情面板共用的只读快照。
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub connected: bool,
    pub error: Option<String>,
    pub updated: Option<Instant>,
    pub latency: Option<Duration>,

    pub host: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub cpu_model: String,
    pub cores: Vec<CpuCore>,
    pub cpu_usage: f32,
    pub load: Option<[f64; 3]>,

    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,

    pub disks: Vec<DiskInfo>,
    pub disk_total: u64,
    pub disk_used: u64,

    pub net_rx_rate: f64,
    pub net_tx_rate: f64,
    pub net_rx_total: u64,
    pub net_tx_total: u64,
    pub ifaces: Vec<NetIface>,

    pub processes: u64,
    pub top_procs: Vec<ProcInfo>,

    pub uptime: u64,
    pub boot_time: String,
    /// 远端时钟与本地时钟的偏差（秒），仅远端会话有。
    pub clock_offset: Option<i64>,
    /// 远端本地时间（由远端 date 命令输出），避免时区换算误差。
    pub remote_datetime: Option<String>,
}

impl Snapshot {
    pub fn mem_pct(&self) -> f32 {
        if self.mem_total == 0 {
            0.0
        } else {
            self.mem_used as f32 / self.mem_total as f32 * 100.0
        }
    }

    pub fn disk_pct(&self) -> f32 {
        if self.disk_total == 0 {
            0.0
        } else {
            self.disk_used as f32 / self.disk_total as f32 * 100.0
        }
    }

    pub fn remote_time(&self) -> Option<String> {
        if let Some(t) = self.remote_datetime.as_ref().filter(|t| !t.trim().is_empty()) {
            return Some(t.trim().to_string());
        }
        let offset = self.clock_offset?;
        let t = chrono::Utc::now() + chrono::Duration::seconds(offset);
        Some(t.format("%Y-%m-%d %H:%M:%S").to_string())
    }
}

/// 采集线程句柄：Drop 时通知线程退出。
pub struct StatusHandle {
    snapshot: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
}

impl StatusHandle {
    pub fn latest(&self) -> Snapshot {
        self.snapshot
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
}

impl Drop for StatusHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn new_handle() -> (StatusHandle, Arc<Mutex<Snapshot>>, Arc<AtomicBool>) {
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let stop = Arc::new(AtomicBool::new(false));
    (
        StatusHandle {
            snapshot: snapshot.clone(),
            stop: stop.clone(),
        },
        snapshot,
        stop,
    )
}

fn sleep_until_next(stop: &AtomicBool, total: Duration) {
    let step = Duration::from_millis(150);
    let start = Instant::now();
    while start.elapsed() < total {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        thread::sleep(step);
    }
}

// ===================== 本地采集 =====================

pub fn spawn_local(ctx: egui::Context) -> StatusHandle {
    let (handle, snapshot, stop) = new_handle();
    let _ = thread::Builder::new()
        .name("wren-local-stats".into())
        .spawn(move || {
            let mut system = sysinfo::System::new_all();
            thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
            system.refresh_cpu_usage();

            let mut prev_net: Option<(u64, u64, Instant)> = None;
            let mut prev_ifaces: HashMap<String, (u64, u64, Instant)> = HashMap::new();

            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }

                system.refresh_cpu_usage();
                system.refresh_memory();
                system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

                let now = Instant::now();
                let disks = sysinfo::Disks::new_with_refreshed_list();
                let networks = sysinfo::Networks::new_with_refreshed_list();

                let mem_total = system.total_memory();
                let mem_available = system.available_memory();
                let mem_used = system.used_memory().min(mem_total);

                let mut disk_list: Vec<DiskInfo> = disks
                    .iter()
                    .filter(|d| d.total_space() > 0)
                    .map(|d| {
                        let total = d.total_space();
                        let available = d.available_space().min(total);
                        DiskInfo {
                            name: d.name().to_string_lossy().to_string(),
                            mount: d.mount_point().to_string_lossy().to_string(),
                            total,
                            used: total.saturating_sub(available),
                            available,
                            file_system: d.file_system().to_string_lossy().to_string(),
                            removable: d.is_removable(),
                        }
                    })
                    .collect();
                disk_list.sort_by(|a, b| {
                    a.mount
                        .to_lowercase()
                        .cmp(&b.mount.to_lowercase())
                        .then(b.total.cmp(&a.total))
                });

                let sys_drive = std::env::var("SystemDrive")
                    .unwrap_or_else(|_| "C:".into())
                    .to_uppercase();
                let root_disk = disk_list
                    .iter()
                    .find(|d| d.mount.to_uppercase().starts_with(&sys_drive))
                    .or_else(|| disk_list.iter().max_by_key(|d| d.total));

                let mut rx_total = 0u64;
                let mut tx_total = 0u64;
                let mut ifaces = Vec::new();
                for (name, data) in networks.iter() {
                    let rx = data.total_received();
                    let tx = data.total_transmitted();
                    rx_total = rx_total.saturating_add(rx);
                    tx_total = tx_total.saturating_add(tx);
                    let (rx_rate, tx_rate) = prev_ifaces
                        .get(name)
                        .map(|(prx, ptx, at)| {
                            let secs = at.elapsed().as_secs_f64().max(0.001);
                            (
                                rx.saturating_sub(*prx) as f64 / secs,
                                tx.saturating_sub(*ptx) as f64 / secs,
                            )
                        })
                        .unwrap_or((0.0, 0.0));
                    prev_ifaces.insert(name.clone(), (rx, tx, now));
                    ifaces.push(NetIface {
                        name: name.clone(),
                        addrs: Vec::new(),
                        rx_rate,
                        tx_rate,
                    });
                }
                ifaces.sort_by(|a, b| a.name.cmp(&b.name));

                let (rx_rate, tx_rate) = prev_net
                    .map(|(prx, ptx, at)| {
                        let secs = at.elapsed().as_secs_f64().max(0.001);
                        (
                            rx_total.saturating_sub(prx) as f64 / secs,
                            tx_total.saturating_sub(ptx) as f64 / secs,
                        )
                    })
                    .unwrap_or((0.0, 0.0));
                prev_net = Some((rx_total, tx_total, now));

                let mut top_procs: Vec<ProcInfo> = system
                    .processes()
                    .iter()
                    .map(|(pid, p)| ProcInfo {
                        pid: pid.as_u32(),
                        name: p.name().to_string_lossy().to_string(),
                        cpu: p.cpu_usage(),
                        mem_pct: if mem_total == 0 {
                            0.0
                        } else {
                            p.memory() as f32 / mem_total as f32 * 100.0
                        },
                    })
                    .collect();
                top_procs.sort_by(|a, b| {
                    b.mem_pct
                        .partial_cmp(&a.mem_pct)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                top_procs.truncate(8);

                let uptime = sysinfo::System::uptime();
                let load = sysinfo::System::load_average();
                let load = if load.one > 0.0 || load.five > 0.0 || load.fifteen > 0.0 {
                    Some([load.one, load.five, load.fifteen])
                } else {
                    None
                };
                let boot_time = (chrono::Local::now()
                    - chrono::Duration::seconds(uptime.min(i64::MAX as u64) as i64))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();

                let os = {
                    let name = sysinfo::System::name().unwrap_or_default();
                    let version = sysinfo::System::os_version().unwrap_or_default();
                    if version.is_empty() {
                        name
                    } else {
                        format!("{name} {version}")
                    }
                };

                let (disk_total, disk_used) = root_disk
                    .map(|d| (d.total, d.used))
                    .unwrap_or((0, 0));

                let snap = Snapshot {
                    connected: true,
                    error: None,
                    updated: Some(now),
                    latency: None,
                    host: sysinfo::System::host_name().unwrap_or_else(|| "localhost".into()),
                    os,
                    kernel: sysinfo::System::kernel_version().unwrap_or_default(),
                    arch: sysinfo::System::cpu_arch(),
                    cpu_model: system
                        .cpus()
                        .first()
                        .map(|c| c.brand().trim().to_string())
                        .unwrap_or_default(),
                    cores: system
                        .cpus()
                        .iter()
                        .map(|c| CpuCore {
                            usage: c.cpu_usage(),
                        })
                        .collect(),
                    cpu_usage: system.global_cpu_usage(),
                    load,
                    mem_total,
                    mem_used,
                    mem_available,
                    swap_total: system.total_swap(),
                    swap_used: system.used_swap(),
                    disks: disk_list,
                    disk_total,
                    disk_used,
                    net_rx_rate: rx_rate,
                    net_tx_rate: tx_rate,
                    net_rx_total: rx_total,
                    net_tx_total: tx_total,
                    ifaces,
                    processes: system.processes().len() as u64,
                    top_procs,
                    uptime,
                    boot_time,
                    clock_offset: None,
                    remote_datetime: None,
                };

                if let Ok(mut slot) = snapshot.lock() {
                    *slot = snap;
                }
                ctx.request_repaint();
                sleep_until_next(&stop, Duration::from_millis(1500));
            }
        });

    handle
}

// ===================== 远端采集 =====================

/// 远端状态脚本：只依赖 POSIX sh 与 /proc，输出 `KEY=value` 行。
/// 所有内容都是 ASCII，避免 Windows 命令行与远端 locale 的编码问题。
const REMOTE_SCRIPT: &str = concat!(
    r#"echo "HOST=$(hostname 2>/dev/null)";"#,
    r#"echo "KERNEL=$(uname -sr 2>/dev/null)";"#,
    r#"echo "ARCH=$(uname -m 2>/dev/null)";"#,
    r#"echo "OS=$(. /etc/os-release 2>/dev/null; printf '%s' "$PRETTY_NAME")";"#,
    r#"echo "UPTIME=$(cut -d. -f1 /proc/uptime 2>/dev/null)";"#,
    r#"echo "LOAD=$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null)";"#,
    r#"echo "CORES=$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null)";"#,
    r#"echo "CPUINFO=$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2)";"#,
    r#"echo "CPUSTAT=$(grep '^cpu ' /proc/stat 2>/dev/null)";"#,
    r#"echo "PERCPU=$(grep '^cpu[0-9]' /proc/stat 2>/dev/null | tr '\n' ';')";"#,
    r#"echo "MEM=$(awk '/^MemTotal:/{t=$2} /^MemAvailable:/{a=$2} /^MemFree:/{f=$2} /^Buffers:/{b=$2} /^Cached:/{c=$2} END{printf "%d %d %d %d %d\n", t*1024, a*1024, f*1024, b*1024, c*1024}' /proc/meminfo 2>/dev/null)";"#,
    r#"echo "SWAP=$(awk '/^SwapTotal:/{t=$2} /^SwapFree:/{f=$2} END{printf "%d %d\n", t*1024, (t-f)*1024}' /proc/meminfo 2>/dev/null)";"#,
    r#"echo "DISK=$(df -Pk / 2>/dev/null | awk 'NR==2{print $1, $2, $3, $4}')";"#,
    r#"echo "DF=$(df -Pk 2>/dev/null | awk 'NR>1 && $2+0>0 {printf "%s|%s|%s|%s;", $6, $2, $3, $4}')";"#,
    r#"echo "NET=$(awk -F'[: ]+' 'NR>2{rx+=$3; tx+=$11} END{printf "%d %d\n", rx+0, tx+0}' /proc/net/dev 2>/dev/null)";"#,
    r#"echo "IFACES=$(ip -o -4 addr show 2>/dev/null | awk '{printf "%s=%s;", $2, $4}' || hostname -I 2>/dev/null | tr ' ' ';')";"#,
    r#"echo "PROCS=$(ls /proc 2>/dev/null | grep -c '^[0-9]')";"#,
    r#"echo "TOP=$(ps -eo pid,comm,pcpu,pmem --sort=-pcpu 2>/dev/null | sed 1d | head -n 6 | tr '\n' ';')";"#,
    r#"echo "RTS=$(date +%s 2>/dev/null)";"#,
    r#"echo "RDATE=$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)";"#,
    r#"echo "END=1""#,
);

#[derive(Debug, Clone, Copy)]
struct CpuTimes {
    total: u64,
    idle: u64,
}

impl CpuTimes {
    fn parse(line: &str) -> Option<Self> {
        let mut it = line.split_whitespace();
        let first = it.next()?;
        if !first.starts_with("cpu") {
            return None;
        }
        let values: Vec<u64> = it.filter_map(|v| v.parse().ok()).collect();
        if values.len() < 4 {
            return None;
        }
        let total: u64 = values.iter().sum();
        let idle = values[3] + values.get(4).copied().unwrap_or(0);
        Some(Self { total, idle })
    }

    fn usage(prev: Option<Self>, cur: Self) -> f32 {
        let Some(prev) = prev else { return 0.0 };
        let total = cur.total.saturating_sub(prev.total);
        let idle = cur.idle.saturating_sub(prev.idle);
        if total == 0 {
            0.0
        } else {
            ((total.saturating_sub(idle)) as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
        }
    }
}

#[derive(Default)]
struct RemotePrev {
    cpu: Option<CpuTimes>,
    percpu: Vec<Option<CpuTimes>>,
    net: Option<(u64, u64, Instant)>,
}

fn parse_u64(s: &str) -> Option<u64> {
    s.trim().parse().ok()
}

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse().ok()
}

/// 把 `KEY=value` 输出解析成 map；同名键以后出现的为准。
fn parse_kv(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                map.insert(k.to_string(), v.trim_end_matches('\r').to_string());
            }
        }
    }
    map
}

fn parse_disk(value: &str) -> Option<DiskInfo> {
    let mut it = value.split_whitespace();
    let device = it.next()?;
    let total_kb = parse_u64(it.next()?)?;
    let used_kb = parse_u64(it.next()?)?;
    let avail_kb = parse_u64(it.next()?)?;
    let total = total_kb.saturating_mul(1024);
    let available = avail_kb.saturating_mul(1024);
    Some(DiskInfo {
        name: device.to_string(),
        mount: "/".into(),
        total,
        used: used_kb.saturating_mul(1024).min(total),
        available,
        file_system: String::new(),
        removable: false,
    })
}

fn parse_df(value: &str) -> Vec<DiskInfo> {
    let mut disks: Vec<DiskInfo> = Vec::new();
    for entry in value.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.split('|').collect();
        if parts.len() != 4 {
            continue;
        }
        let mount = parts[0].trim();
        if mount.is_empty()
            || ["/snap", "/sys", "/proc", "/dev", "/run"]
                .iter()
                .any(|p| mount == *p || mount.starts_with(&format!("{p}/")))
        {
            continue;
        }
        let (Some(total_kb), Some(used_kb), Some(avail_kb)) = (
            parse_u64(parts[1]),
            parse_u64(parts[2]),
            parse_u64(parts[3]),
        ) else {
            continue;
        };
        if total_kb == 0 {
            continue;
        }
        if disks.iter().any(|d| d.mount == mount) {
            continue;
        }
        disks.push(DiskInfo {
            name: String::new(),
            mount: mount.to_string(),
            total: total_kb.saturating_mul(1024),
            used: used_kb.saturating_mul(1024),
            available: avail_kb.saturating_mul(1024),
            file_system: String::new(),
            removable: false,
        });
    }
    disks.sort_by(|a, b| b.total.cmp(&a.total));
    disks.truncate(8);
    disks
}

fn parse_ifaces(value: &str) -> Vec<NetIface> {
    let mut map: Vec<NetIface> = Vec::new();
    for entry in value.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (name, addr) = match entry.split_once('=') {
            Some((n, a)) => (
                n.trim().to_string(),
                a.split('/').next().unwrap_or("").trim().to_string(),
            ),
            None => ("inet".to_string(), entry.to_string()),
        };
        if name.is_empty() || addr.is_empty() {
            continue;
        }
        if let Some(iface) = map.iter_mut().find(|i| i.name == name) {
            if !iface.addrs.iter().any(|a| a == &addr) {
                iface.addrs.push(addr.to_string());
            }
        } else {
            map.push(NetIface {
                name: name.clone(),
                addrs: vec![addr.to_string()],
                rx_rate: 0.0,
                tx_rate: 0.0,
            });
        }
    }
    map.sort_by(|a, b| a.name.cmp(&b.name));
    map
}

fn parse_top(value: &str) -> Vec<ProcInfo> {
    let mut out = Vec::new();
    for entry in value.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let pid = parse_u64(parts[0]).unwrap_or(0) as u32;
        let cpu = parse_f64(parts[parts.len() - 2]).unwrap_or(0.0) as f32;
        let mem = parse_f64(parts[parts.len() - 1]).unwrap_or(0.0) as f32;
        let name = parts[1..parts.len() - 2].join(" ");
        out.push(ProcInfo {
            pid,
            name,
            cpu,
            mem_pct: mem,
        });
    }
    out.truncate(8);
    out
}

/// 解析一次远端输出，并结合上一次采样计算 CPU / 网络速率。
fn build_remote_snapshot(
    text: &str,
    latency: Duration,
    prev: &mut RemotePrev,
    fallback_host: &str,
) -> Snapshot {
    let map = parse_kv(text);

    let cpu_times = map.get("CPUSTAT").and_then(|s| CpuTimes::parse(s));
    let cpu_usage = CpuTimes::usage(prev.cpu, cpu_times.unwrap_or(CpuTimes { total: 0, idle: 0 }));
    if let Some(cur) = cpu_times {
        prev.cpu = Some(cur);
    }

    let mut per_cpu_now = Vec::new();
    if let Some(raw) = map.get("PERCPU") {
        for part in raw.split(';') {
            if let Some(t) = CpuTimes::parse(part) {
                per_cpu_now.push(t);
            }
        }
    }
    let mut cores: Vec<CpuCore> = Vec::new();
    for (i, cur) in per_cpu_now.iter().enumerate() {
        let before = prev.percpu.get(i).copied().flatten();
        cores.push(CpuCore {
            usage: CpuTimes::usage(before, *cur),
        });
    }
    if !per_cpu_now.is_empty() {
        prev.percpu = per_cpu_now.iter().map(|t| Some(*t)).collect();
    }
    if cores.is_empty() {
        let count = map
            .get("CORES")
            .and_then(|s| parse_u64(s))
            .unwrap_or(1)
            .clamp(1, 256) as usize;
        cores = vec![CpuCore { usage: cpu_usage }; count];
    }
    if let Some(total_cores) = map.get("CORES").and_then(|s| parse_u64(s)) {
        if total_cores > 0 && cores.len() < total_cores as usize {
            cores.resize(total_cores.min(256) as usize, CpuCore { usage: cpu_usage });
        }
    }

    let mem = map.get("MEM").map(|s| s.as_str()).unwrap_or("");
    let mem_vals: Vec<u64> = mem.split_whitespace().filter_map(parse_u64).collect();
    let mem_total = mem_vals.first().copied().unwrap_or(0);
    let mem_available = mem_vals.get(1).copied().unwrap_or(0);
    let mem_free = mem_vals.get(2).copied().unwrap_or(0);
    let buffers = mem_vals.get(3).copied().unwrap_or(0);
    let cached = mem_vals.get(4).copied().unwrap_or(0);
    let mem_available = if mem_available > 0 {
        mem_available.min(mem_total)
    } else {
        (mem_free + buffers + cached).min(mem_total)
    };
    let mem_used = mem_total.saturating_sub(mem_available);

    let swap_vals: Vec<u64> = map
        .get("SWAP")
        .map(|s| s.split_whitespace().filter_map(parse_u64).collect())
        .unwrap_or_default();
    let swap_total = swap_vals.first().copied().unwrap_or(0);
    let swap_used = swap_vals.get(1).copied().unwrap_or(0).min(swap_total);

    let mut disks = map.get("DF").map(|s| parse_df(s)).unwrap_or_default();
    let root = map.get("DISK").and_then(|s| parse_disk(s));
    if let Some(root) = &root {
        if !disks.iter().any(|d| d.mount == "/") {
            disks.insert(0, root.clone());
        }
    }

    let net_vals: Vec<u64> = map
        .get("NET")
        .map(|s| s.split_whitespace().filter_map(parse_u64).collect())
        .unwrap_or_default();
    let net_rx = net_vals.first().copied().unwrap_or(0);
    let net_tx = net_vals.get(1).copied().unwrap_or(0);
    let now = Instant::now();
    let (rx_rate, tx_rate) = prev
        .net
        .map(|(prx, ptx, at)| {
            let secs = at.elapsed().as_secs_f64().max(0.001);
            (
                net_rx.saturating_sub(prx) as f64 / secs,
                net_tx.saturating_sub(ptx) as f64 / secs,
            )
        })
        .unwrap_or((0.0, 0.0));
    prev.net = Some((net_rx, net_tx, now));

    let load = map.get("LOAD").and_then(|s| {
        let v: Vec<f64> = s.split_whitespace().filter_map(parse_f64).collect();
        if v.len() >= 3 {
            Some([v[0], v[1], v[2]])
        } else {
            None
        }
    });

    let uptime = map.get("UPTIME").and_then(|s| parse_u64(s)).unwrap_or(0);
    let boot_time = (chrono::Local::now()
        - chrono::Duration::seconds(uptime.min(i64::MAX as u64) as i64))
    .format("%Y-%m-%d %H:%M:%S")
    .to_string();

    let clock_offset = map.get("RTS").and_then(|s| s.trim().parse::<i64>().ok()).map(|ts| {
        let local = chrono::Utc::now().timestamp();
        ts - local
    });
    let remote_datetime = map
        .get("RDATE")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let os = map
        .get("OS")
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Linux".into());
    let host = map
        .get("HOST")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_host.to_string());

    Snapshot {
        connected: true,
        error: None,
        updated: Some(now),
        latency: Some(latency),
        host,
        os,
        kernel: map.get("KERNEL").cloned().unwrap_or_default(),
        arch: map.get("ARCH").cloned().unwrap_or_default(),
        cpu_model: map
            .get("CPUINFO")
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        cores,
        cpu_usage,
        load,
        mem_total,
        mem_used,
        mem_available,
        swap_total,
        swap_used,
        disk_total: root.as_ref().map(|d| d.total).unwrap_or(0),
        disk_used: root.as_ref().map(|d| d.used).unwrap_or(0),
        disks,
        net_rx_rate: rx_rate,
        net_tx_rate: tx_rate,
        net_rx_total: net_rx,
        net_tx_total: net_tx,
        ifaces: map.get("IFACES").map(|s| parse_ifaces(s)).unwrap_or_default(),
        processes: map.get("PROCS").and_then(|s| parse_u64(s)).unwrap_or(0),
        top_procs: map.get("TOP").map(|s| parse_top(s)).unwrap_or_default(),
        uptime,
        boot_time,
        clock_offset,
        remote_datetime,
    }
}

pub fn spawn_remote(ctx: egui::Context, target: RemoteTarget, interval: Duration) -> StatusHandle {
    let (handle, snapshot, stop) = new_handle();
    let label = target.label();
    let endpoint = target.clone();
    let _ = thread::Builder::new()
        .name("wren-remote-stats".into())
        .spawn(move || {
            let mut prev = RemotePrev::default();
            let mut last_good: Option<Snapshot> = None;

            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let result = poll_remote(&endpoint);
                let snap = match result {
                    Ok((text, latency)) => {
                        let s = build_remote_snapshot(&text, latency, &mut prev, &label);
                        last_good = Some(s.clone());
                        s
                    }
                    Err(err) => {
                        let mut s = last_good.clone().unwrap_or_else(|| Snapshot {
                            host: label.clone(),
                            os: if endpoint.is_ssh() {
                                "SSH".into()
                            } else {
                                "WSL".into()
                            },
                            ..Default::default()
                        });
                        s.connected = false;
                        s.error = Some(err);
                        s
                    }
                };
                if let Ok(mut slot) = snapshot.lock() {
                    *slot = snap;
                }
                ctx.request_repaint();
                sleep_until_next(&stop, interval);
            }
        });
    handle
}

fn poll_remote(target: &RemoteTarget) -> Result<(String, Duration), String> {
    let mut cmd = match target {
        RemoteTarget::Wsl { distro } => {
            let mut c = Command::new("wsl.exe");
            if let Some(d) = distro.as_ref().filter(|d| !d.trim().is_empty()) {
                c.arg("-d").arg(d);
            }
            c.arg("-e").arg("sh").arg("-c").arg(REMOTE_SCRIPT);
            c
        }
        RemoteTarget::Ssh { profile } => {
            let mut c = Command::new("ssh.exe");
            c.arg("-T")
                .arg("-o")
                .arg("BatchMode=yes")
                .arg("-o")
                .arg("ConnectTimeout=6")
                .arg("-o")
                .arg("ConnectionAttempts=1")
                .arg("-o")
                .arg("ServerAliveInterval=10")
                .arg("-o")
                .arg("ServerAliveCountMax=1")
                .arg("-o")
                .arg("LogLevel=ERROR");
            if profile.port != 0 && profile.port != 22 {
                c.arg("-p").arg(profile.port.to_string());
            }
            if !profile.identity_file.trim().is_empty() {
                c.arg("-i").arg(profile.identity_file.trim());
            }
            c.arg(profile.target()).arg(REMOTE_SCRIPT);
            c
        }
    };
    hide_window(&mut cmd);
    let timeout = if target.is_ssh() {
        Duration::from_secs(12)
    } else {
        Duration::from_secs(10)
    };
    let (bytes, latency) = run_command(cmd, timeout, true)?;
    Ok((String::from_utf8_lossy(&bytes).to_string(), latency))
}

/// 运行命令并带超时；`require_end` 用于确认远端脚本完整执行。
fn run_command(
    mut cmd: Command,
    timeout: Duration,
    require_end: bool,
) -> Result<(Vec<u8>, Duration), String> {
    let start = Instant::now();
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("无法启动命令: {e}"))?;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("读取命令输出失败: {e}"))?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let msg = stderr.trim();
                    return Err(if msg.is_empty() {
                        format!("命令退出码 {}", output.status.code().unwrap_or(-1))
                    } else {
                        msg.to_string()
                    });
                }
                if require_end && !String::from_utf8_lossy(&output.stdout).contains("END=1") {
                    return Err("远端状态输出不完整".into());
                }
                return Ok((output.stdout, start.elapsed()));
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("状态查询超时".into());
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(e) => return Err(format!("等待命令失败: {e}")),
        }
    }
}

#[cfg(windows)]
fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_: &mut Command) {}

// ===================== WSL 发行版探测 =====================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslDistro {
    pub name: String,
    pub state: String,
    pub version: String,
    pub is_default: bool,
}

/// 解码 wsl.exe 的输出：`wsl -l -v` 是 UTF-16LE，普通命令是 UTF-8。
fn decode_wsl(bytes: &[u8]) -> String {
    if bytes.iter().filter(|b| **b == 0).count() > bytes.len() / 4 {
        let mut units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if units.first() == Some(&0xFEFF) {
            units.remove(0);
        }
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).to_string()
    }
}

pub fn detect_wsl() -> Result<Vec<WslDistro>, String> {
    let mut cmd = Command::new("wsl.exe");
    cmd.arg("-l").arg("-v");
    hide_window(&mut cmd);
    let (bytes, _) = run_command(cmd, Duration::from_secs(10), false)?;
    let text = decode_wsl(&bytes);
    let mut distros = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.contains("NAME") || line.contains("名称") {
            continue;
        }
        let is_default = line.starts_with('*');
        let line = line.trim_start_matches('*').trim();
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        distros.push(WslDistro {
            name: parts[0].to_string(),
            state: parts[1].to_string(),
            version: parts[parts.len() - 1].to_string(),
            is_default,
        });
    }
    Ok(distros)
}

/// 在后台线程里探测 WSL，避免打开连接窗口时界面卡住。
pub fn detect_wsl_async() -> mpsc::Receiver<Result<Vec<WslDistro>, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(detect_wsl());
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        [
            "HOST=web-01",
            "KERNEL=Linux 6.1.0-18-amd64",
            "ARCH=x86_64",
            "OS=Debian GNU/Linux 12 (bookworm)",
            "UPTIME=3725",
            "LOAD=0.15 0.20 0.10",
            "CORES=2",
            "CPUINFO=Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz",
            "CPUSTAT=cpu  100 0 100 800 0 0 0 0 0 0",
            "PERCPU=cpu0 50 0 50 400 0 0 0 0 0 0;cpu1 50 0 50 400 0 0 0 0 0 0;",
            "MEM=8589934592 6442450944 1073741824 268435456 805306368",
            "SWAP=2147483648 104857600",
            "DISK=/dev/vda1 41943040 12582912 27262976",
            "DF=/|41943040|12582912|27262976;/home|10485760|1048576|9437184;",
            "NET=1000000 2000000",
            "IFACES=eth0=172.17.0.2/16;eth0=10.0.0.8/24;",
            "PROCS=142",
            "TOP=1 systemd 0.5 1.2;222 nginx 0.2 0.8;",
            "RTS=1700000000",
            "RDATE=2023-11-14 22:13:20",
            "END=1",
        ]
        .join("\n")
    }

    #[test]
    fn parses_remote_snapshot() {
        let mut prev = RemotePrev::default();
        let snap = build_remote_snapshot(&sample(), Duration::from_millis(42), &mut prev, "x");
        assert_eq!(snap.host, "web-01");
        assert_eq!(snap.kernel, "Linux 6.1.0-18-amd64");
        assert_eq!(snap.os, "Debian GNU/Linux 12 (bookworm)");
        assert_eq!(snap.cores.len(), 2);
        assert_eq!(snap.mem_total, 8_589_934_592);
        assert_eq!(snap.mem_used, 2_147_483_648);
        assert_eq!(snap.mem_available, 6_442_450_944);
        assert_eq!(snap.swap_total, 2_147_483_648);
        assert_eq!(snap.swap_used, 104_857_600);
        assert_eq!(snap.disk_total, 42_949_672_960);
        assert_eq!(snap.disk_used, 12_884_901_888);
        assert_eq!(snap.disks.len(), 2);
        assert_eq!(snap.net_rx_total, 1_000_000);
        assert_eq!(snap.net_tx_total, 2_000_000);
        assert_eq!(snap.processes, 142);
        assert_eq!(snap.top_procs.len(), 2);
        assert_eq!(snap.top_procs[1].name, "nginx");
        assert_eq!(snap.ifaces[0].addrs.len(), 2);
        assert_eq!(snap.uptime, 3725);
        assert_eq!(snap.remote_time().as_deref(), Some("2023-11-14 22:13:20"));
        assert!(snap.latency.is_some());
    }

    #[test]
    fn cpu_usage_needs_two_samples() {
        let a = CpuTimes::parse("cpu  100 0 100 800 0 0 0 0").unwrap();
        let b = CpuTimes::parse("cpu  200 0 100 900 0 0 0 0").unwrap();
        assert_eq!(CpuTimes::usage(None, b), 0.0);
        let usage = CpuTimes::usage(Some(a), b);
        assert!((usage - 50.0).abs() < 0.01, "{usage}");
    }

    #[test]
    fn skips_pseudo_mounts() {
        let disks = parse_df("/|100|50|50;/proc|100|1|99;/run/user/0|10|1|9;");
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].mount, "/");
    }

    #[test]
    fn decodes_utf16_wsl_output() {
        let s = "  NAME  STATE  VERSION\r\n* Ubuntu Running 2\r\n";
        let mut bytes = Vec::new();
        for u in s.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_wsl(&bytes), s);
    }
}
