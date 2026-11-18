use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::channel;
use std::thread;
use tilo::ipc::{IpcRequest, IpcServer};

#[test]
fn test_ipc_server_lifecycle_and_queries() {
    let (wm_tx, wm_rx) = channel();

    // Create a dummy wake pipe for testing
    let mut pipe_fds = [0i32; 2];
    assert_eq!(
        unsafe { libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) },
        0
    );
    let wake_read = pipe_fds[0];
    let wake_write = pipe_fds[1];

    let server = IpcServer::start(wm_tx, wake_write).expect("Failed to start IPC server");
    let sock_path = server.socket_path.clone();
    assert!(sock_path.exists());

    // Worker thread to simulate WM responding to IPC requests
    let _wm_handle = thread::spawn(move || {
        let mut wake_buf = [0u8; 16];
        while let Ok(req) = wm_rx.recv() {
            // Drain wake pipe
            unsafe {
                libc::read(
                    wake_read,
                    wake_buf.as_mut_ptr() as *mut libc::c_void,
                    wake_buf.len(),
                );
            }

            match req.request {
                IpcRequest::GetVersion => {
                    let _ = req.responder.send("{\"name\":\"tilo\",\"version\":\"0.2.7\"}".to_string());
                }
                IpcRequest::GetWorkspaces => {
                    let _ = req.responder.send("[{\"id\":1,\"name\":\"1\",\"focused\":true}]".to_string());
                }
                IpcRequest::GetMarks => {
                    let _ = req.responder.send("[\"browser\",\"editor\"]".to_string());
                }
                IpcRequest::Command(cmd) => {
                    if cmd == "fail_cmd" {
                        let _ = req.responder.send("{\"success\":false,\"error\":\"command failed\"}".to_string());
                    } else {
                        let _ = req.responder.send("{\"success\":true}".to_string());
                    }
                }
                _ => {
                    let _ = req.responder.send("{\"success\":true}".to_string());
                }
            }
        }
        unsafe {
            libc::close(wake_read);
            libc::close(wake_write);
        }
    });

    // Client connection 1
    let mut stream = UnixStream::connect(&sock_path).expect("Client connect failed");
    let mut reader = BufReader::new(stream.try_clone().unwrap());

    // Query version
    stream.write_all(b"get_version\n").unwrap();
    stream.flush().unwrap();
    let mut resp = String::new();
    reader.read_line(&mut resp).unwrap();
    assert!(resp.contains("\"name\":\"tilo\""));
    assert!(resp.contains("\"version\":\"0.2.7\""));

    // Query workspaces
    resp.clear();
    stream.write_all(b"get_workspaces\n").unwrap();
    stream.flush().unwrap();
    reader.read_line(&mut resp).unwrap();
    assert!(resp.contains("\"name\":\"1\""));

    // Query marks
    resp.clear();
    stream.write_all(b"get_marks\n").unwrap();
    stream.flush().unwrap();
    reader.read_line(&mut resp).unwrap();
    assert!(resp.contains("\"browser\""));
    assert!(resp.contains("\"editor\""));

    // Send successful command
    resp.clear();
    stream.write_all(b"workspace 2\n").unwrap();
    stream.flush().unwrap();
    reader.read_line(&mut resp).unwrap();
    assert_eq!(resp.trim(), "{\"success\":true}");

    // Send failing command
    resp.clear();
    stream.write_all(b"fail_cmd\n").unwrap();
    stream.flush().unwrap();
    reader.read_line(&mut resp).unwrap();
    assert!(resp.contains("\"success\":false"));
    assert!(resp.contains("command failed"));

    // Test subscriber
    let sub_stream = UnixStream::connect(&sock_path).expect("Subscriber connect failed");
    let mut sub_reader = BufReader::new(sub_stream.try_clone().unwrap());
    let mut sub_writer = sub_stream;
    sub_writer.write_all(b"subscribe workspace,window\n").unwrap();
    sub_writer.flush().unwrap();

    let mut sub_resp = String::new();
    sub_reader.read_line(&mut sub_resp).unwrap();
    assert_eq!(sub_resp.trim(), "{\"success\":true}");

    // Broadcast an event
    server.broadcast_event(
        "workspace",
        "{\"event\":\"workspace\",\"change\":\"focus\",\"current\":\"2\"}",
    );

    let mut event_line = String::new();
    sub_reader.read_line(&mut event_line).unwrap();
    assert!(event_line.contains("\"event\":\"workspace\""));
    assert!(event_line.contains("\"current\":\"2\""));

    // Drop client streams
    drop(stream);
    drop(sub_writer);

    // Test built-in tilo msg CLI runner
    let code = tilo::ipc::run_cli(&["msg".to_string(), "workspace".to_string(), "3".to_string()]);
    assert_eq!(code, 0);

    // Shutdown server
    server.shutdown();
    assert!(!sock_path.exists());
    drop(server);
}
