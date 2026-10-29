//! Process spawning and sanitization utilities for Tilo.
//!
//! Provides i3-parity process isolation for child processes (terminals, application launchers,
//! and autostart tasks). Ensures child processes are cleanly detached into their own Unix session
//! (`setsid`) and do not inherit Tilo's X11 server connection, event loops, or internal file descriptors.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

/// Linux constant `CLOSE_RANGE_CLOEXEC` = (1 << 2) = 4.
#[cfg(target_os = "linux")]
const CLOSE_RANGE_CLOEXEC: libc::c_uint = 4;

/// Configures a `Command` with standard window manager process isolation:
/// 1. Attaches `stdin`, `stdout`, and `stderr` to `/dev/null` so child output does not pollute
///    the window manager's console or block on open pipes.
/// 2. Sets a `pre_exec` hook that calls `libc::setsid()` to establish a new session and process group.
/// 3. Resets `SIGCHLD` to `SIG_DFL` so child processes can manage their own subprocesses cleanly.
/// 4. Sets `FD_CLOEXEC` on all inherited file descriptors $\ge 3$ via `SYS_close_range` with `CLOSE_RANGE_CLOEXEC`
///    (or a fallback `fcntl` sweep). This ensures Tilo's X11 sockets, statusbar timers, and pipes are
///    closed atomically upon `exec`, while preserving Rust's internal error pipe during process startup.
pub fn configure_detached_command(cmd: &mut Command) {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            // 1. Detach from parent session and process group (i3 parity)
            libc::setsid();

            // 2. Reset SIGCHLD so child processes handle their own children normally
            libc::signal(libc::SIGCHLD, libc::SIG_DFL);

            // 3. Mark all file descriptors >= 3 with FD_CLOEXEC so they are closed on exec.
            #[cfg(target_os = "linux")]
            {
                if libc::syscall(libc::SYS_close_range, 3, libc::c_uint::MAX, CLOSE_RANGE_CLOEXEC) != 0 {
                    set_cloexec_fallback();
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                set_cloexec_fallback();
            }

            Ok(())
        });
    }
}

/// Fallback helper to set `FD_CLOEXEC` on file descriptors >= 3 when `SYS_close_range` is unavailable.
#[cfg(unix)]
unsafe fn set_cloexec_fallback() {
    let max_fd = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) }.clamp(256, 4096) as libc::c_int;
    for fd in 3..max_fd {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags >= 0 {
            unsafe {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

/// Spawns a shell command string asynchronously via `sh -c` with full process and FD isolation.
///
/// Returns the Process ID (PID) of the spawned child process on success.
pub fn spawn_detached_shell(cmd_str: &str) -> std::io::Result<u32> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(cmd_str);
    configure_detached_command(&mut cmd);
    let child = cmd.spawn()?;
    Ok(child.id())
}

/// Spawns an executable directly with arguments with full process and FD isolation.
///
/// Returns the Process ID (PID) of the spawned child process on success.
pub fn spawn_detached_app(binary: &str, args: &[String]) -> std::io::Result<u32> {
    let mut cmd = Command::new(binary);
    cmd.args(args);
    configure_detached_command(&mut cmd);
    let child = cmd.spawn()?;
    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_detached_shell_echo() {
        let res = spawn_detached_shell("echo tilo_test");
        assert!(res.is_ok(), "spawn_detached_shell should succeed for echo");
    }

    #[test]
    fn test_spawn_detached_app_true() {
        let res = spawn_detached_app("true", &[]);
        assert!(res.is_ok(), "spawn_detached_app should succeed for 'true'");
    }

    #[test]
    fn test_spawn_detached_app_nonexistent_fails() {
        let res = spawn_detached_app("__non_existent_binary_xyz_123__", &[]);
        assert!(res.is_err(), "nonexistent binary should fail and return error");
    }
}
