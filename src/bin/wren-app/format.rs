//! 状态栏与详情面板使用的格式化工具。

use std::time::{Duration, Instant};

const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

/// 人类可读的容量，例如 `15.6 GB`。
pub fn bytes(value: u64) -> String {
    let mut v = value as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value, UNITS[unit])
    } else if v >= 100.0 {
        format!("{:.0} {}", v, UNITS[unit])
    } else {
        format!("{:.1} {}", v, UNITS[unit])
    }
}

/// 网络速率，例如 `1.2 MB/s`。
pub fn rate(bytes_per_sec: f64) -> String {
    format!("{}/s", bytes(bytes_per_sec.max(0.0) as u64))
}

/// 百分比，保留一位小数；整数时省略小数。
pub fn pct(value: f32) -> String {
    if !value.is_finite() || value <= 0.0 {
        "0%".into()
    } else if value >= 100.0 {
        "100%".into()
    } else if (value - value.round()).abs() < 0.05 {
        format!("{:.0}%", value)
    } else {
        format!("{:.1}%", value)
    }
}

/// 运行时长，例如 `3天4小时`、`12分8秒`。
pub fn duration(secs: u64) -> String {
    if secs < 60 {
        return format!("{}秒", secs);
    }
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    let s = secs % 60;
    if days > 0 {
        format!("{}天{}小时", days, hours)
    } else if hours > 0 {
        format!("{}小时{}分", hours, mins)
    } else {
        format!("{}分{}秒", mins, s)
    }
}

/// 数据更新时间的相对描述。
pub fn age(at: Option<Instant>) -> String {
    match at {
        None => "尚未更新".into(),
        Some(t) => {
            let secs = t.elapsed().as_secs();
            if secs < 2 {
                "刚刚".into()
            } else if secs < 60 {
                format!("{} 秒前", secs)
            } else if secs < 3_600 {
                format!("{} 分钟前", secs / 60)
            } else {
                format!("{} 小时前", secs / 3_600)
            }
        }
    }
}

pub fn latency(d: Option<Duration>) -> String {
    match d {
        Some(d) if d.as_millis() < 1 => "<1ms".into(),
        Some(d) if d.as_millis() < 1_000 => format!("{}ms", d.as_millis()),
        Some(d) => format!("{:.1}s", d.as_secs_f64()),
        None => "—".into(),
    }
}

pub fn clock() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

pub fn datetime() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 截断过长的主机名 / 标题，避免状态栏被撑爆。
pub fn shorten(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1024), "1.0 KB");
        assert_eq!(bytes(16 * 1024 * 1024 * 1024), "16.0 GB");
    }

    #[test]
    fn formats_duration() {
        assert_eq!(duration(5), "5秒");
        assert_eq!(duration(125), "2分5秒");
        assert_eq!(duration(3_725), "1小时2分");
        assert_eq!(duration(90_000), "1天1小时");
    }

    #[test]
    fn formats_latency() {
        assert_eq!(latency(None), "—");
        assert_eq!(latency(Some(Duration::from_millis(12))), "12ms");
    }
}
