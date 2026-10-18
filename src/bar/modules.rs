use std::io::Read;
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

/// Identifies a configurable right-aligned statusbar module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BarModuleType {
    /// Local IP address module.
    Ip,
    /// RAM usage percentage module.
    Ram,
    /// CPU usage percentage module.
    Cpu,
    /// Battery capacity and charging status module.
    Battery,
    /// Output audio volume level and mute state module.
    Volume,
    /// System clock module formatted as DAY:DATE:TIME.
    Clock,
}

impl BarModuleType {
    /// Returns the canonical identifier string for this module.
    pub fn as_str(&self) -> &'static str {
        match self {
            BarModuleType::Ip => "ip",
            BarModuleType::Ram => "ram",
            BarModuleType::Cpu => "cpu",
            BarModuleType::Battery => "battery",
            BarModuleType::Volume => "volume",
            BarModuleType::Clock => "clock",
        }
    }

    /// Parses a module name or synonym loosely (case-insensitive, trims commas/spaces).
    pub fn from_str_loose(s: &str) -> Result<Self, String> {
        let clean = s.trim().trim_matches(',').to_lowercase();
        match clean.as_str() {
            "ip" | "ip_address" | "ip-address" | "ipaddress" | "network" | "net" => Ok(BarModuleType::Ip),
            "ram" | "ram_usage" | "ram-usage" | "ramusage" | "memory" | "mem" => Ok(BarModuleType::Ram),
            "cpu" | "cpu_usage" | "cpu-usage" | "cpuusage" => Ok(BarModuleType::Cpu),
            "battery" | "battery_percentage" | "battery-percentage" | "batterypercentage" | "bat" => Ok(BarModuleType::Battery),
            "volume" | "vol" | "audio" | "sound" => Ok(BarModuleType::Volume),
            "clock" | "time" | "date" | "datetime" | "date-time" | "day:date:time" | "day_date_time" | "day-date-time" => Ok(BarModuleType::Clock),
            _ => Err(format!(
                "Unknown statusbar module '{}'. Available modules: ip, ram, cpu, battery, volume, clock",
                s
            )),
        }
    }

    /// Instantiates a new boxed status module instance for this module type.
    pub fn instantiate(&self) -> Box<dyn StatusModule> {
        match self {
            BarModuleType::Ip => Box::new(IpModule::new()),
            BarModuleType::Ram => Box::new(RamModule::new()),
            BarModuleType::Cpu => Box::new(CpuModule::new()),
            BarModuleType::Battery => Box::new(BatteryModule::new()),
            BarModuleType::Volume => Box::new(VolumeModule::new()),
            BarModuleType::Clock => Box::new(ClockModule::new()),
        }
    }
}

impl std::fmt::Display for BarModuleType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for BarModuleType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s)
    }
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

    /// Returns a reference to Any for downcasting if supported.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
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
    pub cached_ip: Option<String>,
}

impl IpModule {
    pub fn new() -> Self {
        Self::with_ip(None)
    }

    pub fn with_ip(ip: Option<String>) -> Self {
        let (cached_text, cached_ip) = match ip.as_deref() {
            Some("") => ("IP Offline".to_string(), Some("".to_string())),
            Some(s) => (format!("IP {}", s), Some(s.to_string())),
            None => ("IP ...".to_string(), None),
        };
        Self {
            cached_text,
            last_updated: None,
            custom_ip: ip,
            cached_ip,
        }
    }

    pub fn set_ip(&mut self, ip: Option<String>) {
        self.custom_ip = ip;
    }

    fn query_local_ip_addr() -> Option<std::net::IpAddr> {
        let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("1.1.1.1:80").ok()?;
        let addr = socket.local_addr().ok()?;
        let ip = addr.ip();
        if ip.is_unspecified() || ip.is_loopback() {
            None
        } else {
            Some(ip)
        }
    }

    fn query_local_ip() -> Option<String> {
        Self::query_local_ip_addr().map(|ip| ip.to_string())
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

        let current_ip = if let Some(ref custom) = self.custom_ip {
            Some(custom.clone())
        } else {
            Self::query_local_ip()
        };

        if self.cached_ip == current_ip {
            return Ok(ModuleUpdate::Unchanged);
        }

        self.cached_ip = current_ip;
        use std::fmt::Write;
        self.cached_text.clear();
        match self.cached_ip.as_deref() {
            Some(ip) if !ip.is_empty() => {
                let _ = write!(&mut self.cached_text, "IP {}", ip);
            }
            _ => {
                self.cached_text.push_str("IP Offline");
            }
        }

        Ok(ModuleUpdate::Changed(self.cached_text.clone()))
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
    pub cached_percent: Option<u32>,
}

impl RamModule {
    pub fn new() -> Self {
        Self::with_path(PathBuf::from("/proc/meminfo"))
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self {
            cached_text: "RAM --%".to_string(),
            last_updated: None,
            meminfo_path: path,
            cached_percent: None,
        }
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

        let mut file = match std::fs::File::open(&self.meminfo_path) {
            Ok(f) => f,
            Err(e) => {
                self.cached_text = "RAM: ERR".to_string();
                return Ok(ModuleUpdate::Error(format!(
                    "Failed to read {}: {}",
                    self.meminfo_path.display(),
                    e
                )));
            }
        };

        let mut buf = [0u8; 1024];
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                self.cached_text = "RAM: ERR".to_string();
                return Ok(ModuleUpdate::Error(format!(
                    "Failed to read {}: {}",
                    self.meminfo_path.display(),
                    e
                )));
            }
        };

        let content = std::str::from_utf8(&buf[..n]).unwrap_or("");

        if let Some(percent) = Self::parse_meminfo(content) {
            if self.cached_percent == Some(percent) {
                return Ok(ModuleUpdate::Unchanged);
            }

            self.cached_percent = Some(percent);
            use std::fmt::Write;
            self.cached_text.clear();
            let _ = write!(&mut self.cached_text, "RAM {}%", percent);
            Ok(ModuleUpdate::Changed(self.cached_text.clone()))
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
    pub cached_percent: Option<u32>,
}

impl CpuModule {
    pub fn new() -> Self {
        Self::with_path(PathBuf::from("/proc/stat"))
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self {
            cached_text: "CPU --%".to_string(),
            last_updated: None,
            stat_path: path,
            prev_total: 0,
            prev_idle: 0,
            cached_percent: None,
        }
    }

    pub fn parse_cpu_times(content: &str) -> Option<(u64, u64)> {
        for line in content.lines() {
            if line.starts_with("cpu ") {
                let mut parts = line.split_whitespace();
                let _cpu = parts.next()?;
                let user = parts.next()?.parse::<u64>().ok()?;
                let nice = parts.next()?.parse::<u64>().ok()?;
                let system = parts.next()?.parse::<u64>().ok()?;
                let idle = parts.next()?.parse::<u64>().ok()?;
                let iowait = parts.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                let irq = parts.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                let softirq = parts.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                let steal = parts.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

                let idle_time = idle + iowait;
                let total_time = user + nice + system + idle + iowait + irq + softirq + steal;
                return Some((total_time, idle_time));
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

        let mut file = match std::fs::File::open(&self.stat_path) {
            Ok(f) => f,
            Err(e) => {
                self.cached_text = "CPU: ERR".to_string();
                return Ok(ModuleUpdate::Error(format!(
                    "Failed to read {}: {}",
                    self.stat_path.display(),
                    e
                )));
            }
        };

        let mut buf = [0u8; 256];
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                self.cached_text = "CPU: ERR".to_string();
                return Ok(ModuleUpdate::Error(format!(
                    "Failed to read {}: {}",
                    self.stat_path.display(),
                    e
                )));
            }
        };

        let content = std::str::from_utf8(&buf[..n]).unwrap_or("");

        if let Some((total, idle)) = Self::parse_cpu_times(content) {
            if self.prev_total == 0 {
                self.prev_total = total;
                self.prev_idle = idle;
                self.cached_percent = Some(0);
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
                if self.cached_percent == Some(percent) {
                    return Ok(ModuleUpdate::Unchanged);
                }
                self.cached_percent = Some(percent);
                use std::fmt::Write;
                self.cached_text.clear();
                let _ = write!(&mut self.cached_text, "CPU {}%", percent);
                Ok(ModuleUpdate::Changed(self.cached_text.clone()))
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
    pub capacity_path: Option<PathBuf>,
    pub status_path: Option<PathBuf>,
    pub cached_state: Option<(u32, bool)>,
}

impl BatteryModule {
    pub fn new() -> Self {
        Self::with_path(Self::find_battery_path())
    }

    pub fn with_path(path: Option<PathBuf>) -> Self {
        let capacity_path = path.as_ref().map(|p| p.join("capacity"));
        let status_path = path.as_ref().map(|p| p.join("status"));
        let cached_text = if path.is_none() {
            "BAT N/A".to_string()
        } else {
            "BAT --%".to_string()
        };
        let mut s = Self {
            cached_text,
            last_updated: None,
            sysfs_path: path,
            capacity_path,
            status_path,
            cached_state: None,
        };
        if s.sysfs_path.is_some() {
            let _ = s.update();
        }
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

    pub fn parse_battery_from_files(
        capacity_path: &std::path::Path,
        status_path: Option<&std::path::Path>,
    ) -> Result<(u32, bool), String> {
        let mut cap_file = std::fs::File::open(capacity_path)
            .map_err(|e| format!("Failed to read capacity: {}", e))?;
        let mut cap_buf = [0u8; 16];
        let n = cap_file.read(&mut cap_buf)
            .map_err(|e| format!("Failed to read capacity: {}", e))?;
        let cap_content = std::str::from_utf8(&cap_buf[..n])
            .map_err(|e| format!("Failed to parse capacity utf8: {}", e))?;
        let cap: u32 = cap_content
            .trim()
            .parse::<u32>()
            .map_err(|e| format!("Failed to parse capacity: {}", e))?;

        let is_charging = if let Some(stat_path) = status_path {
            if let Ok(mut status_file) = std::fs::File::open(stat_path) {
                let mut stat_buf = [0u8; 32];
                if let Ok(sn) = status_file.read(&mut stat_buf) {
                    let status_content = std::str::from_utf8(&stat_buf[..sn]).unwrap_or("");
                    status_content.trim().eq_ignore_ascii_case("charging")
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        Ok((cap.min(100), is_charging))
    }

    pub fn parse_battery_info(bat_dir: &std::path::Path) -> Result<(u32, bool), String> {
        let capacity_path = bat_dir.join("capacity");
        let status_path = bat_dir.join("status");
        Self::parse_battery_from_files(&capacity_path, Some(&status_path))
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

        let cap_path = match self.capacity_path.as_deref() {
            Some(p) => p,
            None => {
                let na = "BAT N/A";
                if self.cached_text != na {
                    self.cached_text = na.to_string();
                    return Ok(ModuleUpdate::Changed(self.cached_text.clone()));
                }
                return Ok(ModuleUpdate::Unchanged);
            }
        };

        match Self::parse_battery_from_files(cap_path, self.status_path.as_deref()) {
            Ok((cap, is_charging)) => {
                if self.cached_state == Some((cap, is_charging)) {
                    return Ok(ModuleUpdate::Unchanged);
                }

                self.cached_state = Some((cap, is_charging));
                use std::fmt::Write;
                self.cached_text.clear();
                if is_charging {
                    let _ = write!(&mut self.cached_text, "BAT {}% (+)", cap);
                } else {
                    let _ = write!(&mut self.cached_text, "BAT {}%", cap);
                }

                Ok(ModuleUpdate::Changed(self.cached_text.clone()))
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
    pub cached_time: Option<(i32, i32, i32)>,
}

impl ClockModule {
    pub fn new() -> Self {
        let mut s = Self {
            cached_text: String::new(),
            last_updated: None,
            cached_time: None,
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
        let mut now = 0 as libc::time_t;
        unsafe {
            libc::time(&mut now);
            let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
            if !libc::localtime_r(&now, tm.as_mut_ptr()).is_null() {
                let tm = tm.assume_init();
                let time_key = (tm.tm_yday, tm.tm_hour, tm.tm_min);
                if self.cached_time == Some(time_key) && !self.cached_text.is_empty() {
                    return Ok(ModuleUpdate::Unchanged);
                }
                self.cached_time = Some(time_key);
                const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
                let day_name = DAYS.get(tm.tm_wday.clamp(0, 6) as usize).unwrap_or(&"---");
                let year = 1900 + tm.tm_year;
                let month = tm.tm_mon + 1;
                let mday = tm.tm_mday;
                let hour = tm.tm_hour;
                let min = tm.tm_min;

                use std::fmt::Write;
                self.cached_text.clear();
                let _ = write!(
                    &mut self.cached_text,
                    "{}:{:04}-{:02}-{:02}:{:02}:{:02}",
                    day_name, year, month, mday, hour, min
                );
                Ok(ModuleUpdate::Changed(self.cached_text.clone()))
            } else {
                if self.cached_text == "---:0000-00-00:00:00" {
                    return Ok(ModuleUpdate::Unchanged);
                }
                self.cached_text = "---:0000-00-00:00:00".to_string();
                self.cached_time = None;
                Ok(ModuleUpdate::Changed(self.cached_text.clone()))
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

/// Volume status module displaying the current output audio level (e.g. `VOL 65%` or `VOL MUTE`).
#[derive(Debug, Clone)]
pub struct VolumeModule {
    pub cached_text: String,
    pub cached_state: Option<crate::audio::VolumeState>,
    pub last_updated: Option<Instant>,
    pub update_interval_ms: u64,
}

impl VolumeModule {
    pub fn new() -> Self {
        Self::with_interval(2000)
    }

    pub fn with_interval(update_interval_ms: u64) -> Self {
        let mut s = Self {
            cached_text: String::new(),
            cached_state: None,
            last_updated: None,
            update_interval_ms,
        };
        let _ = s.update();
        s
    }

    /// Returns the currently cached volume state if known.
    pub fn state(&self) -> Option<crate::audio::VolumeState> {
        self.cached_state
    }

    /// Explicitly updates the volume state and updates cached text.
    pub fn set_state(&mut self, state: Option<crate::audio::VolumeState>) -> ModuleUpdate {
        self.last_updated = Some(Instant::now());
        if self.cached_state == state {
            return ModuleUpdate::Unchanged;
        }
        self.cached_state = state;
        Self::write_state(&mut self.cached_text, state);
        ModuleUpdate::Changed(self.cached_text.clone())
    }

    pub fn write_state(buf: &mut String, state: Option<crate::audio::VolumeState>) {
        buf.clear();
        use std::fmt::Write;
        match state {
            Some(s) if s.muted => {
                if s.volume > 0 {
                    let _ = write!(buf, "VOL {}% [MUTE]", s.volume);
                } else {
                    buf.push_str("VOL MUTE");
                }
            }
            Some(s) => {
                let _ = write!(buf, "VOL {}%", s.volume);
            }
            None => {
                buf.push_str("VOL N/A");
            }
        }
    }

    pub fn format_state(state: Option<crate::audio::VolumeState>) -> String {
        let mut s = String::new();
        Self::write_state(&mut s, state);
        s
    }
}

impl Default for VolumeModule {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusModule for VolumeModule {
    fn name(&self) -> &'static str {
        "volume"
    }

    fn update_interval(&self) -> u64 {
        self.update_interval_ms
    }

    fn update(&mut self) -> Result<ModuleUpdate, StatusbarError> {
        self.last_updated = Some(Instant::now());
        let current_state = crate::audio::get_volume();
        if self.cached_state == current_state {
            return Ok(ModuleUpdate::Unchanged);
        }
        self.cached_state = current_state;
        Self::write_state(&mut self.cached_text, current_state);
        Ok(ModuleUpdate::Changed(self.cached_text.clone()))
    }

    fn current_text(&self) -> &str {
        &self.cached_text
    }

    fn last_updated(&self) -> Option<Instant> {
        self.last_updated
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

