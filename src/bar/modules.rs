//! System monitor modules for the status bar (RAM, CPU, IP, Battery, Clock).

use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::Instant;
use super::StatusbarError;

/// Represents the data returned by a module after an update tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleUpdate {
    /// The module's text has changed; UI needs to be re-rendered.
    Changed(String),
    /// No change in module text since last tick.
    Unchanged,
    /// The module encountered an error (e.g., failed to read `/proc`).
    Error(String),
}

/// Trait defining the behavior of an independent statusbar module.
pub trait StatusModule: std::fmt::Debug + Send {
    /// Returns the name of the module for debugging and identification.
    fn name(&self) -> &'static str;

    /// The interval in milliseconds at which this module should be polled.
    fn update_interval(&self) -> u64;

    /// Called periodically to fetch new data.
    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError>;

    /// Returns the current cached text to be drawn.
    fn current_text(&self) -> &str;

    /// Returns the timestamp when this module was last updated, if available.
    fn last_updated(&self) -> Option<Instant> {
        None
    }

    /// Returns a mutable reference to Any for downcasting if supported.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }
}

/// IP status module displaying the active local IP address.
#[derive(Debug, Clone)]
pub struct IpModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
    pub custom_ip: Option<String>,
}

impl IpModule {
    pub fn new() -> Self {
        Self::with_ip(None)
    }

    pub fn with_ip(ip: Option<String>) -> Self {
        let mut s = Self {
            cached_text: "IP ...".to_string(),
            last_updated: None,
            custom_ip: ip,
        };
        let _ = s.update();
        s
    }

    pub fn set_ip(&mut self, ip: Option<String>) {
        self.custom_ip = ip;
    }

    fn query_local_ip() -> Option<String> {
        let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("1.1.1.1:80").ok()?;
        let addr = socket.local_addr().ok()?;
        let ip = addr.ip();
        if ip.is_unspecified() || ip.is_loopback() {
            None
        } else {
            Some(ip.to_string())
        }
    }
}

impl Default for IpModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for IpModule {
    fn name(&self) -> &'static str {
        "ip"
    }

    fn update_interval(&self) -> u64 {
        3000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());

        let ip_opt = if let Some(ref custom) = self.custom_ip {
            Some(custom.clone())
        } else {
            Self::query_local_ip()
        };

        let new_text = match ip_opt {
            Some(ip) if !ip.is_empty() => format!("IP {}", ip),
            _ => "IP Offline".to_string(),
        };

        if new_text != self.cached_text {
            self.cached_text = new_text.clone();
            Ok(ModuleUpdate::Changed(new_text))
        } else {
            Ok(ModuleUpdate::Unchanged)
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// RAM status module reading `/proc/meminfo` directly without external child processes.
#[derive(Debug, Clone)]
pub struct RamModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
    pub meminfo_path: PathBuf,
}

impl RamModule {
    pub fn new() -> Self {
        Self::with_path(PathBuf::from("/proc/meminfo"))
    }

    pub fn with_path(path: PathBuf) -> Self {
        let mut s = Self {
            cached_text: "RAM --%".to_string(),
            last_updated: None,
            meminfo_path: path,
        };
        let _ = s.update();
        s
    }

    pub fn parse_meminfo(content: &str) -> Option<u32> {
        let mut mem_total: Option<u64> = None;
        let mut mem_available: Option<u64> = None;

        for line in content.lines() {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            let val = parts.next()?.parse::<u64>().ok()?;

            if key == "MemTotal:" {
                mem_total = Some(val);
            } else if key == "MemAvailable:" {
                mem_available = Some(val);
            }

            if mem_total.is_some() && mem_available.is_some() {
                break;
            }
        }

        let total = mem_total?;
        let available = mem_available?;
        if total == 0 {
            return None;
        }

        let used = total.saturating_sub(available);
        let percent = ((used as f64 / total as f64) * 100.0).round() as u32;
        Some(percent.min(100))
    }
}

impl Default for RamModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for RamModule {
    fn name(&self) -> &'static str {
        "ram"
    }

    fn update_interval(&self) -> u64 {
        2000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());

        let content = match std::fs::read_to_string(&self.meminfo_path) {
            Ok(c) => c,
            Err(e) => {
                self.cached_text = "RAM: ERR".to_string();
                return Ok(ModuleUpdate::Error(format!(
                    "Failed to read {}: {}",
                    self.meminfo_path.display(),
                    e
                )));
            }
        };

        if let Some(percent) = Self::parse_meminfo(&content) {
            let new_text = format!("RAM {}%", percent);
            if new_text != self.cached_text {
                self.cached_text = new_text.clone();
                Ok(ModuleUpdate::Changed(new_text))
            } else {
                Ok(ModuleUpdate::Unchanged)
            }
        } else {
            self.cached_text = "RAM: ERR".to_string();
            Ok(ModuleUpdate::Error("Malformed /proc/meminfo content".to_string()))
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }
}

/// CPU status module reading `/proc/stat` delta metrics.
#[derive(Debug, Clone)]
pub struct CpuModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
    pub stat_path: PathBuf,
    pub prev_total: u64,
    pub prev_idle: u64,
}

impl CpuModule {
    pub fn new() -> Self {
        Self::with_path(PathBuf::from("/proc/stat"))
    }

    pub fn with_path(path: PathBuf) -> Self {
        let mut s = Self {
            cached_text: "CPU --%".to_string(),
            last_updated: None,
            stat_path: path,
            prev_total: 0,
            prev_idle: 0,
        };
        let _ = s.update();
        s
    }

    pub fn parse_cpu_times(content: &str) -> Option<(u64, u64)> {
        for line in content.lines() {
            if line.starts_with("cpu ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
                    let user = parts[1].parse::<u64>().ok()?;
                    let nice = parts[2].parse::<u64>().ok()?;
                    let system = parts[3].parse::<u64>().ok()?;
                    let idle = parts[4].parse::<u64>().ok()?;
                    let iowait = parts.get(5).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                    let irq = parts.get(6).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                    let softirq = parts.get(7).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                    let steal = parts.get(8).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

                    let idle_time = idle + iowait;
                    let total_time = user + nice + system + idle + iowait + irq + softirq + steal;
                    return Some((total_time, idle_time));
                }
            }
        }
        None
    }
}

impl Default for CpuModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for CpuModule {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn update_interval(&self) -> u64 {
        1000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());

        let content = match std::fs::read_to_string(&self.stat_path) {
            Ok(c) => c,
            Err(e) => {
                self.cached_text = "CPU: ERR".to_string();
                return Ok(ModuleUpdate::Error(format!(
                    "Failed to read {}: {}",
                    self.stat_path.display(),
                    e
                )));
            }
        };

        if let Some((total, idle)) = Self::parse_cpu_times(&content) {
            if self.prev_total == 0 {
                self.prev_total = total;
                self.prev_idle = idle;
                self.cached_text = "CPU 0%".to_string();
                return Ok(ModuleUpdate::Changed(self.cached_text.clone()));
            }

            let total_delta = total.saturating_sub(self.prev_total);
            let idle_delta = idle.saturating_sub(self.prev_idle);

            self.prev_total = total;
            self.prev_idle = idle;

            if total_delta > 0 {
                let usage = 100.0 * (total_delta.saturating_sub(idle_delta)) as f64 / total_delta as f64;
                let percent = (usage.round() as u32).min(100);
                let new_text = format!("CPU {}%", percent);
                if new_text != self.cached_text {
                    self.cached_text = new_text.clone();
                    Ok(ModuleUpdate::Changed(new_text))
                } else {
                    Ok(ModuleUpdate::Unchanged)
                }
            } else {
                Ok(ModuleUpdate::Unchanged)
            }
        } else {
            self.cached_text = "CPU: ERR".to_string();
            Ok(ModuleUpdate::Error("Malformed /proc/stat content".to_string()))
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }
}

/// Battery status module reading `/sys/class/power_supply/BAT*` capacity and charging status.
#[derive(Debug, Clone)]
pub struct BatteryModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
    pub sysfs_path: Option<PathBuf>,
}

impl BatteryModule {
    pub fn new() -> Self {
        Self::with_path(Self::find_battery_path())
    }

    pub fn with_path(path: Option<PathBuf>) -> Self {
        let mut s = Self {
            cached_text: "BAT --%".to_string(),
            last_updated: None,
            sysfs_path: path,
        };
        let _ = s.update();
        s
    }

    pub fn find_battery_path() -> Option<PathBuf> {
        let base = std::path::Path::new("/sys/class/power_supply");
        if let Ok(entries) = std::fs::read_dir(base) {
            let mut bat_paths = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("BAT") || name_str.starts_with("bat") {
                    bat_paths.push(entry.path());
                }
            }
            bat_paths.sort();
            if let Some(first) = bat_paths.into_iter().next() {
                return Some(first);
            }
        }
        None
    }

    pub fn parse_battery_info(bat_dir: &std::path::Path) -> Result<(u32, bool), String> {
        let capacity_path = bat_dir.join("capacity");
        let status_path = bat_dir.join("status");

        let cap_content = std::fs::read_to_string(&capacity_path)
            .map_err(|e| format!("Failed to read capacity: {}", e))?;
        let cap: u32 = cap_content
            .trim()
            .parse::<u32>()
            .map_err(|e| format!("Failed to parse capacity: {}", e))?;

        let is_charging = if let Ok(status_content) = std::fs::read_to_string(&status_path) {
            status_content.trim().eq_ignore_ascii_case("charging")
        } else {
            false
        };

        Ok((cap.min(100), is_charging))
    }
}

impl Default for BatteryModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for BatteryModule {
    fn name(&self) -> &'static str {
        "battery"
    }

    fn update_interval(&self) -> u64 {
        5000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());

        let path = match self.sysfs_path {
            Some(ref p) => p,
            None => {
                let na = "BAT N/A".to_string();
                if self.cached_text != na {
                    self.cached_text = na.clone();
                    return Ok(ModuleUpdate::Changed(na));
                }
                return Ok(ModuleUpdate::Unchanged);
            }
        };

        match Self::parse_battery_info(path) {
            Ok((cap, is_charging)) => {
                let new_text = if is_charging {
                    format!("BAT {}% (+)", cap)
                } else {
                    format!("BAT {}%", cap)
                };

                if new_text != self.cached_text {
                    self.cached_text = new_text.clone();
                    Ok(ModuleUpdate::Changed(new_text))
                } else {
                    Ok(ModuleUpdate::Unchanged)
                }
            }
            Err(e) => {
                self.cached_text = "BAT: ERR".to_string();
                Ok(ModuleUpdate::Error(e))
            }
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }
}

/// Clock status module displaying the system time in `DAY:DATE:TIME` format (e.g. `Tue:2026-09-29:04:18`).
#[derive(Debug, Clone)]
pub struct ClockModule {
    pub cached_text: String,
    pub last_updated: Option<Instant>,
}

impl ClockModule {
    pub fn new() -> Self {
        let mut s = Self {
            cached_text: String::new(),
            last_updated: None,
        };
        let _ = s.update();
        s
    }

    pub fn format_now() -> String {
        let mut now = 0 as libc::time_t;
        unsafe {
            libc::time(&mut now);
            let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
            if !libc::localtime_r(&now, tm.as_mut_ptr()).is_null() {
                let tm = tm.assume_init();
                const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
                let day_name = DAYS.get(tm.tm_wday.clamp(0, 6) as usize).unwrap_or(&"---");
                let year = 1900 + tm.tm_year;
                let month = tm.tm_mon + 1;
                let mday = tm.tm_mday;
                let hour = tm.tm_hour;
                let min = tm.tm_min;
                format!("{}:{:04}-{:02}-{:02}:{:02}:{:02}", day_name, year, month, mday, hour, min)
            } else {
                "---:0000-00-00:00:00".to_string()
            }
        }
    }
}

impl Default for ClockModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for ClockModule {
    fn name(&self) -> &'static str {
        "clock"
    }

    fn update_interval(&self) -> u64 {
        1000
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());
        let new_text = Self::format_now();
        if new_text != self.cached_text {
            self.cached_text = new_text.clone();
            Ok(ModuleUpdate::Changed(new_text))
        } else {
            Ok(ModuleUpdate::Unchanged)
        }
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }
}
