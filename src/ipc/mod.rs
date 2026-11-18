//! Native Tilo IPC (Inter-Process Communication) system.
//!
//! Provides a zero-dependency, Unix Domain Socket based control interface
//! for scripting, container management, queries, and event subscriptions.

pub mod client;
pub mod command;
pub mod json;
pub mod protocol;
pub mod server;

pub use client::run_cli;
pub use command::{CommandResult, ParsedCommand, parse_command, split_command_chain};
pub use json::JsonValue;
pub use protocol::{IpcRequest, get_socket_path, parse_request_line};
pub use server::{IpcServer, IpcWmRequest};
