//! Native Tilo IPC protocol definitions and message framing.

use std::path::{Path, PathBuf};

/// Default socket filename for Tilo IPC.
pub const DEFAULT_SOCKET_NAME: &str = "tilo-ipc.sock";

/// Returns the primary socket path for Tilo IPC.
///
/// Checks `$TILO_SOCK`, then `$XDG_RUNTIME_DIR/tilo-ipc.sock`,
/// falling back to `/tmp/tilo-ipc-<uid>.sock`.
pub fn get_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("TILO_SOCK") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime_dir.is_empty() {
            return Path::new(&runtime_dir).join(DEFAULT_SOCKET_NAME);
        }
    }

    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/tilo-ipc-{}.sock", uid))
}

/// Request sent from IPC clients to the Window Manager.
#[derive(Debug, Clone, PartialEq)]
pub enum IpcRequest {
    /// Execute one or more commands (e.g. `workspace 2`, `split v; layout tabbed`).
    Command(String),
    /// Retrieve the container tree JSON.
    GetTree,
    /// Retrieve the workspace list JSON.
    GetWorkspaces,
    /// Retrieve the output monitor list JSON.
    GetOutputs,
    /// Retrieve the mark list JSON.
    GetMarks,
    /// Retrieve version JSON.
    GetVersion,
    /// Subscribe connection to event stream.
    Subscribe(Vec<String>),
}

/// Parses an incoming text line from an IPC client.
pub fn parse_request_line(line: &str) -> IpcRequest {
    let trimmed = line.trim();
    if trimmed.eq_ignore_ascii_case("get_tree") {
        IpcRequest::GetTree
    } else if trimmed.eq_ignore_ascii_case("get_workspaces") {
        IpcRequest::GetWorkspaces
    } else if trimmed.eq_ignore_ascii_case("get_outputs") {
        IpcRequest::GetOutputs
    } else if trimmed.eq_ignore_ascii_case("get_marks") {
        IpcRequest::GetMarks
    } else if trimmed.eq_ignore_ascii_case("get_version") {
        IpcRequest::GetVersion
    } else if trimmed.starts_with("subscribe") {
        let events_str = trimmed["subscribe".len()..].trim();
        let events = events_str
            .split([',', ' '])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        IpcRequest::Subscribe(events)
    } else {
        IpcRequest::Command(trimmed.to_string())
    }
}
