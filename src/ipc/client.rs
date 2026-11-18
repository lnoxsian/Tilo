//! Built-in IPC client implementation for `tilo msg`.

use super::protocol::get_socket_path;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

/// Runs the CLI client for `tilo msg <command...>`.
///
/// Returns the exit code (0 for success, 1 for error).
pub fn run_cli(args: &[String]) -> i32 {
    let socket_path = get_socket_path();
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!(
                "[ERROR tilo::ipc] Could not connect to Tilo IPC socket at '{}': {}. Is Tilo running?",
                socket_path.display(),
                err
            );
            return 1;
        }
    };

    let command_to_send = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        // Read single line from stdin if no arguments provided
        let mut input = String::new();
        if let Err(err) = std::io::stdin().read_line(&mut input) {
            eprintln!("[ERROR tilo::ipc] Failed to read from stdin: {}", err);
            return 1;
        }
        input.trim().to_string()
    };

    if command_to_send.is_empty() {
        eprintln!("[WARN  tilo::ipc] No command provided. Usage: tilo msg <command>");
        return 1;
    }

    let payload = format!("{}\n", command_to_send);
    if let Err(err) = stream.write_all(payload.as_bytes()) {
        eprintln!("[ERROR tilo::ipc] Failed to write to socket: {}", err);
        return 1;
    }
    let _ = stream.flush();

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    match reader.read_line(&mut response) {
        Ok(0) => {
            eprintln!("[ERROR tilo::ipc] Connection closed unexpectedly by Window Manager.");
            1
        }
        Ok(_) => {
            print!("{}", response);
            if response.contains("\"success\":false") { 1 } else { 0 }
        }
        Err(err) => {
            eprintln!("[ERROR tilo::ipc] Failed to read response: {}", err);
            1
        }
    }
}
