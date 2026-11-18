//! Unix Domain Socket server and event broadcaster for Tilo IPC.

use super::protocol::{IpcRequest, get_socket_path, parse_request_line};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

/// A request forwarded from an IPC client to the Window Manager thread.
pub struct IpcWmRequest {
    pub request: IpcRequest,
    pub responder: Sender<String>,
}

struct Subscriber {
    stream: UnixStream,
    events: Vec<String>,
}

/// The IPC server managing client connections and event broadcasting.
pub struct IpcServer {
    pub socket_path: PathBuf,
    subscribers: Arc<Mutex<Vec<Subscriber>>>,
    running: Arc<AtomicBool>,
}

impl IpcServer {
    /// Starts the IPC server, binding the Unix socket and spawning worker threads.
    ///
    /// # Arguments
    /// * `wm_tx` - Channel sender to forward requests to the WM thread.
    /// * `wake_fd` - Raw file descriptor of the write end of the wake pipe.
    pub fn start(wm_tx: Sender<IpcWmRequest>, wake_fd: i32) -> Result<Self, std::io::Error> {
        let socket_path = get_socket_path();

        // Remove stale socket if it exists
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let listener = UnixListener::bind(&socket_path)?;
        // Ensure socket permissions are 0700 (owner only)
        let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o700));

        // Export environment variables
        let sock_str = socket_path.to_string_lossy().to_string();
        unsafe {
            std::env::set_var("TILO_SOCK", &sock_str);
            std::env::set_var("TILO_SOCKET", &sock_str);
        }

        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));

        let server_running = Arc::clone(&running);
        let server_subs = Arc::clone(&subscribers);

        thread::Builder::new()
            .name("tilo-ipc-server".to_string())
            .spawn(move || {
                for stream in listener.incoming() {
                    if !server_running.load(Ordering::Relaxed) {
                        break;
                    }
                    let stream = match stream {
                        Ok(s) => s,
                        Err(_) => continue,
                    };

                    let wm_tx_clone = wm_tx.clone();
                    let subs_clone = Arc::clone(&server_subs);

                    thread::Builder::new()
                        .name("tilo-ipc-client".to_string())
                        .spawn(move || {
                            handle_client_connection(stream, wm_tx_clone, wake_fd, subs_clone);
                        })
                        .ok();
                }
            })?;

        Ok(Self {
            socket_path,
            subscribers,
            running,
        })
    }

    /// Broadcasts an event payload to all subscribed clients.
    pub fn broadcast_event(&self, event_name: &str, payload: &str) {
        let mut subs = match self.subscribers.lock() {
            Ok(s) => s,
            Err(_) => return,
        };

        let message = format!("{}\n", payload);
        subs.retain_mut(|sub| {
            if sub.events.is_empty()
                || sub.events.iter().any(|e| e == "*" || e.eq_ignore_ascii_case(event_name))
            {
                sub.stream.write_all(message.as_bytes()).is_ok()
            } else {
                true
            }
        });
    }

    /// Shuts down the IPC server and removes the socket file.
    pub fn shutdown(&self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.socket_path);
        unsafe {
            std::env::remove_var("TILO_SOCK");
            std::env::remove_var("TILO_SOCKET");
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn handle_client_connection(
    stream: UnixStream,
    wm_tx: Sender<IpcWmRequest>,
    wake_fd: i32,
    subscribers: Arc<Mutex<Vec<Subscriber>>>,
) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut writer = stream;

    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }

        let req = parse_request_line(trimmed);

        // Check if this is a subscribe request
        if let IpcRequest::Subscribe(ref events) = req {
            let reply = "{\"success\":true}\n";
            let _ = writer.write_all(reply.as_bytes());
            let _ = writer.flush();

            // Register into subscribers list and break out of command loop
            if let Ok(mut subs) = subscribers.lock() {
                subs.push(Subscriber {
                    stream: writer,
                    events: events.clone(),
                });
            }
            return;
        }

        // Send request to WM thread
        let (resp_tx, resp_rx): (Sender<String>, Receiver<String>) = channel();
        let wm_req = IpcWmRequest {
            request: req,
            responder: resp_tx,
        };

        if wm_tx.send(wm_req).is_err() {
            let _ = writer.write_all(b"{\"success\":false,\"error\":\"WM stopped\"}\n");
            return;
        }

        // Wake up poll on WM thread
        unsafe {
            let byte: [u8; 1] = [1];
            libc::write(wake_fd, byte.as_ptr() as *const libc::c_void, 1);
        }

        // Wait for response from WM thread with 5 second timeout
        match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(resp_str) => {
                let mut out = resp_str;
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                if writer.write_all(out.as_bytes()).is_err() || writer.flush().is_err() {
                    return;
                }
            }
            Err(_) => {
                let _ = writer.write_all(b"{\"success\":false,\"error\":\"WM response timeout\"}\n");
                return;
            }
        }

        line.clear();
    }
}
