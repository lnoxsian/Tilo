//! Startup and autostart command execution.
//!
//! Implements Phase 22 of Plan.md:
//! - Optional startup commands parsed from the configuration file (`autostart "<command>"`).
//! - Direct, non-blocking process execution avoiding a shell where possible.
//! - Non-crashing error handling (spawning failures are logged as warnings).
//! - Automatic process reaping (`SIGCHLD` ignore) to prevent zombie processes.

use std::process::Command;
use thiserror::Error;

/// Represents a single autostart command configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutostartCmd {
    /// The binary name or path to execute.
    pub program: String,
    /// The arguments to pass to the program.
    pub args: Vec<String>,
}

impl AutostartCmd {
    /// Creates a new `AutostartCmd` with a program and arguments.
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// Parses a command line string into an `AutostartCmd`, respecting quotes.
    ///
    /// # Examples
    /// ```
    /// use tilo::autostart::AutostartCmd;
    /// let cmd = AutostartCmd::from_command_line("nm-applet").unwrap();
    /// assert_eq!(cmd.program, "nm-applet");
    /// assert!(cmd.args.is_empty());
    ///
    /// let cmd_args = AutostartCmd::from_command_line("feh --bg-fill /wallpaper.jpg").unwrap();
    /// assert_eq!(cmd_args.program, "feh");
    /// assert_eq!(cmd_args.args, vec!["--bg-fill", "/wallpaper.jpg"]);
    /// ```
    pub fn from_command_line(cmd: &str) -> Option<Self> {
        let parts = crate::launcher::parse_command_line(cmd);
        if parts.is_empty() {
            None
        } else {
            Some(Self {
                program: parts[0].clone(),
                args: parts[1..].to_vec(),
            })
        }
    }
}

/// Errors that can occur during the autostart phase.
#[derive(Debug, Error)]
pub enum AutostartError {
    /// Failed to spawn the process.
    #[error("Failed to spawn '{0}': {1}")]
    SpawnFailed(String, std::io::Error),
}

/// Spawns an individual autostart command as a detached background process.
///
/// Direct process execution is used to avoid unnecessary `/bin/sh` wrapping.
///
/// # Arguments
/// * `cmd` - The autostart command to spawn.
///
/// # Returns
/// The Process ID (PID) of the spawned child process, or an [`AutostartError`].
pub fn spawn_command(cmd: &AutostartCmd) -> Result<u32, AutostartError> {
    if cmd.program.is_empty() {
        return Err(AutostartError::SpawnFailed(
            String::new(),
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Empty program name"),
        ));
    }

    let mut command = Command::new(&cmd.program);
    command.args(&cmd.args);
    crate::process::configure_detached_command(&mut command);

    let child = command
        .spawn()
        .map_err(|e| AutostartError::SpawnFailed(cmd.program.clone(), e))?;

    Ok(child.id())
}

/// Executes all configured autostart commands.
///
/// Iterates over the provided list of commands and spawns each one in the background.
/// Note that this function does not block waiting for the commands to finish.
///
/// Spawn errors for individual binaries are logged as warnings and do not abort
/// remaining commands or crash the window manager.
///
/// # Arguments
/// * `cmds` - A slice of autostart commands to execute.
///
/// # Returns
/// A `Result` which is `Ok` if all spawn attempts were processed.
pub fn run_autostart_commands(cmds: &[AutostartCmd]) -> Result<(), AutostartError> {
    for cmd in cmds {
        if cmd.program.is_empty() {
            continue;
        }

        match spawn_command(cmd) {
            Ok(pid) => {
                println!(
                    "[INFO  tilo::autostart] Spawned '{}' (PID {})",
                    cmd.program, pid
                );
            }
            Err(e) => {
                eprintln!(
                    "[WARN  tilo::autostart] {}", e
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autostart_cmd_from_command_line() {
        let cmd1 = AutostartCmd::from_command_line("nm-applet").unwrap();
        assert_eq!(cmd1.program, "nm-applet");
        assert!(cmd1.args.is_empty());

        let cmd2 = AutostartCmd::from_command_line("feh --bg-fill '/path/to my/wallpaper.jpg'").unwrap();
        assert_eq!(cmd2.program, "feh");
        assert_eq!(cmd2.args, vec!["--bg-fill", "/path/to my/wallpaper.jpg"]);

        assert_eq!(AutostartCmd::from_command_line("   "), None);
    }

    #[test]
    fn test_spawn_command_success() {
        let cmd = AutostartCmd::new("true", vec![]);
        let res = spawn_command(&cmd);
        assert!(res.is_ok());
    }

    #[test]
    fn test_spawn_command_nonexistent() {
        let cmd = AutostartCmd::new("__non_existent_binary_12345__", vec![]);
        let res = spawn_command(&cmd);
        assert!(res.is_err());
        match res.unwrap_err() {
            AutostartError::SpawnFailed(prog, _) => {
                assert_eq!(prog, "__non_existent_binary_12345__");
            }
        }
    }

    #[test]
    fn test_run_autostart_commands_continues_on_failure() {
        let cmds = vec![
            AutostartCmd::new("__non_existent_binary_12345__", vec![]),
            AutostartCmd::new("true", vec![]),
        ];
        let res = run_autostart_commands(&cmds);
        assert!(res.is_ok());
    }
}
