//! Audio subsystem integration and volume controller for tilo.
//!
//! Provides a universal, low-overhead interface to query and adjust system volume
//! across Linux audio backends:
//! - WirePlumber / PipeWire (`wpctl`)
//! - PulseAudio (`pactl`)
//! - ALSA (`amixer`)
//!
//! Designed for near-zero CPU usage:
//! - Probes and caches available backends lazily without shell overhead (`/bin/sh`).
//! - Executes commands directly with argument vectors and minimal buffers.
//! - Caches volume state to avoid redundant parsing and statusbar redraws.

use std::sync::OnceLock;

/// Active system audio backend detected on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioBackend {
    /// WirePlumber / PipeWire (`wpctl`)
    Wpctl,
    /// PulseAudio (`pactl`)
    Pactl,
    /// ALSA mixer (`amixer`)
    Amixer,
    /// No supported audio CLI tool found in PATH
    None,
}

/// Represents the current audio volume and mute state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeState {
    /// Volume percentage from 0 to 100 (or up to 150 if boosted).
    pub volume: u8,
    /// Whether audio output is currently muted.
    pub muted: bool,
}

impl VolumeState {
    pub fn new(volume: u8, muted: bool) -> Self {
        Self { volume, muted }
    }
}

static DETECTED_BACKEND: OnceLock<AudioBackend> = OnceLock::new();

/// Returns whether a binary name is executable in `PATH`.
pub fn is_command_available(cmd: &str) -> bool {
    if let Some(paths) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&paths) {
            let candidate = p.join(cmd);
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

/// Probes the system for available audio tools and returns the preferred backend.
///
/// Priority order:
/// 1. `wpctl` (modern PipeWire standard)
/// 2. `pactl` (PulseAudio standard)
/// 3. `amixer` (ALSA base fallback)
pub fn detect_backend() -> AudioBackend {
    if is_command_available("wpctl") {
        AudioBackend::Wpctl
    } else if is_command_available("pactl") {
        AudioBackend::Pactl
    } else if is_command_available("amixer") {
        AudioBackend::Amixer
    } else {
        AudioBackend::None
    }
}

/// Returns the cached audio backend, detecting it on first call.
pub fn get_backend() -> AudioBackend {
    *DETECTED_BACKEND.get_or_init(detect_backend)
}

// ---------------------------------------------------------------------------
fn run_command_stdout(cmd: &str, args: &[&str]) -> Option<String> {
    use std::io::Read;
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let mut output = Vec::new();
    let _ = stdout.read_to_end(&mut output);
    let _ = child.wait(); // Safely ignore ECHILD when WM has set SIGCHLD to SIG_IGN

    if output.is_empty() {
        return None;
    }
    String::from_utf8(output).ok()
}

fn run_command_sync(cmd: &str, args: &[&str]) {
    if let Ok(mut child) = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        let _ = child.wait();
    }
}

// ---------------------------------------------------------------------------
// Parsers for backend outputs
// ---------------------------------------------------------------------------

/// Parses output from `wpctl get-volume @DEFAULT_AUDIO_SINK@`.
/// Examples:
/// - "Volume: 0.65"
/// - "Volume: 0.65 [MUTED]"
/// - "Volume: 0.00 [MUTED]"
/// - "Volume: 1.00"
pub fn parse_wpctl_output(output: &str) -> Option<VolumeState> {
    let trimmed = output.trim();
    let rest = trimmed.strip_prefix("Volume:")?.trim();
    let muted = rest.contains("[MUTED]");
    let num_part = rest.split_whitespace().next()?;
    let vol_f32: f32 = num_part.parse().ok()?;
    let volume = (vol_f32 * 100.0).round().clamp(0.0, 150.0) as u8;
    Some(VolumeState { volume, muted })
}

/// Parses volume percentage from `pactl get-sink-volume @DEFAULT_SINK@`.
/// Example line:
/// "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB"
pub fn parse_pactl_volume_output(output: &str) -> Option<u8> {
    for part in output.split('/') {
        let trimmed = part.trim();
        if let Some(pct) = trimmed.strip_suffix('%')
            && let Ok(val) = pct.trim().parse::<u8>()
        {
            return Some(val);
        }
    }
    None
}

/// Parses mute status from `pactl get-sink-mute @DEFAULT_SINK@`.
/// Example: "Mute: yes" or "Mute: no"
pub fn parse_pactl_mute_output(output: &str) -> Option<bool> {
    let trimmed = output.trim();
    if let Some(status) = trimmed.strip_prefix("Mute:") {
        let status = status.trim().to_lowercase();
        if status == "yes" {
            return Some(true);
        } else if status == "no" {
            return Some(false);
        }
    }
    None
}

/// Parses output from `amixer get Master`.
/// Example lines:
/// "  Front Left: Playback 32768 [50%] [-18.00dB] [on]"
/// "  Front Left: Playback 0 [0%] [-inf dB] [off]"
pub fn parse_amixer_output(output: &str) -> Option<VolumeState> {
    let mut volume = None;
    let mut muted = false;

    for line in output.lines() {
        if let Some(start) = line.find('[')
            && let Some(pct_idx) = line[start..].find("%]")
        {
            let pct_str = &line[start + 1..start + pct_idx];
            if let Ok(v) = pct_str.parse::<u8>() {
                volume = Some(v);
            }
        }
        if line.contains("[off]") {
            muted = true;
        }
    }

    volume.map(|v| VolumeState { volume: v, muted })
}

// ---------------------------------------------------------------------------
// High-level public Audio Controller API
// ---------------------------------------------------------------------------

/// Queries the current audio volume and mute state, trying backends in fallback order.
pub fn get_volume() -> Option<VolumeState> {
    // 1. Try wpctl if available
    if is_command_available("wpctl") {
        if let Some(out) = run_command_stdout("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
            && let Some(state) = parse_wpctl_output(&out)
        {
            return Some(state);
        }
        if let Some(out) = run_command_stdout("wpctl", &["get-volume", "@DEFAULT_SINK@"])
            && let Some(state) = parse_wpctl_output(&out)
        {
            return Some(state);
        }
    }

    // 2. Try pactl if available
    if is_command_available("pactl")
        && let Some(vol_out) = run_command_stdout("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
        && let Some(vol) = parse_pactl_volume_output(&vol_out)
    {
        let mute_out = run_command_stdout("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]);
        let muted = mute_out.as_deref().and_then(parse_pactl_mute_output).unwrap_or(false);
        return Some(VolumeState { volume: vol, muted });
    }

    // 3. Try amixer if available
    if is_command_available("amixer")
        && let Some(out) = run_command_stdout("amixer", &["get", "Master"])
        && let Some(state) = parse_amixer_output(&out)
    {
        return Some(state);
    }

    None
}

/// Changes the current volume by `delta` percentage points (positive or negative).
pub fn adjust_volume(delta: i8) {
    let abs_val = delta.unsigned_abs();
    match get_backend() {
        AudioBackend::Wpctl => {
            let arg = if delta >= 0 {
                format!("{}%+", abs_val)
            } else {
                format!("{}%-", abs_val)
            };
            run_command_sync(
                "wpctl",
                &["set-volume", "-l", "1.5", "@DEFAULT_AUDIO_SINK@", &arg],
            );
        }
        AudioBackend::Pactl => {
            let arg = if delta >= 0 {
                format!("+{}%", abs_val)
            } else {
                format!("-{}%", abs_val)
            };
            run_command_sync(
                "pactl",
                &["set-sink-volume", "@DEFAULT_SINK@", &arg],
            );
        }
        AudioBackend::Amixer => {
            let arg = if delta >= 0 {
                format!("{}%+", abs_val)
            } else {
                format!("{}%-", abs_val)
            };
            run_command_sync("amixer", &["-q", "sset", "Master", &arg]);
        }
        AudioBackend::None => {}
    }
}

/// Toggles audio output mute state.
pub fn toggle_mute() {
    match get_backend() {
        AudioBackend::Wpctl => {
            run_command_sync("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
        }
        AudioBackend::Pactl => {
            run_command_sync("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "toggle"]);
        }
        AudioBackend::Amixer => {
            run_command_sync("amixer", &["-q", "sset", "Master", "toggle"]);
        }
        AudioBackend::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_wpctl() {
        assert_eq!(
            parse_wpctl_output("Volume: 0.65"),
            Some(VolumeState { volume: 65, muted: false })
        );
        assert_eq!(
            parse_wpctl_output("Volume: 0.00 [MUTED]"),
            Some(VolumeState { volume: 0, muted: true })
        );
        assert_eq!(
            parse_wpctl_output("Volume: 1.00"),
            Some(VolumeState { volume: 100, muted: false })
        );
        assert_eq!(
            parse_wpctl_output("Volume: 0.50 [MUTED]\n"),
            Some(VolumeState { volume: 50, muted: true })
        );
        assert_eq!(parse_wpctl_output("Invalid output"), None);
    }

    #[test]
    fn test_parse_pactl() {
        let pactl_vol = "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB\n        balance 0.00\n";
        assert_eq!(parse_pactl_volume_output(pactl_vol), Some(50));
        assert_eq!(parse_pactl_mute_output("Mute: yes\n"), Some(true));
        assert_eq!(parse_pactl_mute_output("Mute: no\n"), Some(false));
    }

    #[test]
    fn test_parse_amixer() {
        let amixer_out = "\
Simple mixer control 'Master',0
  Capabilities: pvolume pswitch pswitch-joined
  Playback channels: Front Left - Front Right
  Limits: Playback 0 - 65536
  Mono:
  Front Left: Playback 32768 [50%] [-18.00dB] [on]
  Front Right: Playback 32768 [50%] [-18.00dB] [on]
";
        assert_eq!(
            parse_amixer_output(amixer_out),
            Some(VolumeState { volume: 50, muted: false })
        );

        let amixer_muted = "\
Simple mixer control 'Master',0
  Capabilities: pvolume pswitch pswitch-joined
  Playback channels: Front Left - Front Right
  Front Left: Playback 0 [0%] [off]
  Front Right: Playback 0 [0%] [off]
";
        assert_eq!(
            parse_amixer_output(amixer_muted),
            Some(VolumeState { volume: 0, muted: true })
        );
    }

    #[test]
    fn test_is_command_available_common() {
        // "sh" is standard on all Linux / POSIX systems
        assert!(is_command_available("sh"));
        assert!(!is_command_available("nonexistent_command_xyz_12345"));
    }

    #[test]
    fn test_live_backend_detection_and_query() {
        let backend = get_backend();
        println!("Detected audio backend on host: {:?}", backend);
        // On this host, wpctl is present, so backend should be Wpctl
        assert_eq!(backend, AudioBackend::Wpctl);
        let vol = get_volume();
        println!("Current live volume state: {:?}", vol);
        assert!(vol.is_some());
    }
}
