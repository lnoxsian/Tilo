use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, WindowClass};
use x11rb::wrapper::ConnectionExt as _;

struct XephyrServer {
    child: Child,
    display: String,
}

impl XephyrServer {
    fn start(display_num: u32) -> Option<Self> {
        let display = format!(":{}", display_num);
        let child = Command::new("Xephyr")
            .arg(&display)
            .arg("-screen")
            .arg("1000x800")
            .arg("-ac")
            .arg("-br")
            .arg("-noreset")
            .spawn()
            .ok()?;

        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if X11Context::connect_to(Some(&display)).is_ok() {
                return Some(Self { child, display });
            }
        }

        let mut s = Self { child, display };
        s.kill();
        None
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for XephyrServer {
    fn drop(&mut self) {
        self.kill();
    }
}

fn send_ipc_cmd(
    stream: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    wm: &mut WindowManager,
    cmd: &str,
) -> String {
    let payload = format!("{}\n", cmd);
    stream.write_all(payload.as_bytes()).unwrap();
    stream.flush().unwrap();
    let _ = wm.process_pending_ipc_requests();
    let _ = wm.process_pending_events();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    line.trim().to_string()
}

#[test]
fn test_native_ipc_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(94);
    if let Some(server) = xephyr {
        let wm_ctx =
            X11Context::connect_to(Some(&server.display)).expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");
        let sock_path = wm.ipc_server.as_ref().unwrap().socket_path.clone();

        // Connect client to create windows
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let create_win = |title: &str| -> x11rb::protocol::xproto::Window {
            let win = client_conn.generate_id().expect("Failed XID");
            client_conn
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    win,
                    root,
                    0,
                    0,
                    200,
                    200,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    x11rb::COPY_FROM_PARENT,
                    &CreateWindowAux::new(),
                )
                .expect("Create window failed");

            client_conn
                .change_property8(
                    x11rb::protocol::xproto::PropMode::REPLACE,
                    win,
                    AtomEnum::WM_NAME,
                    AtomEnum::STRING,
                    title.as_bytes(),
                )
                .expect("Set title failed");

            client_conn.map_window(win).expect("Map window failed");
            let _ = client_conn.flush();
            win
        };

        // Create initial window
        let win1 = create_win("Editor Window");
        let _ = wm.process_pending_events();

        // Connect IPC client over Unix domain socket
        let mut stream = UnixStream::connect(&sock_path).expect("IPC connect failed");
        let mut reader = BufReader::new(stream.try_clone().unwrap());

        // 1. Get version
        let ver = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "get_version");
        assert!(ver.contains("\"name\":\"tilo\""));

        // 2. Get workspaces
        let ws_list = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "get_workspaces");
        assert!(ws_list.contains("\"name\":\"1\""));

        // 3. Switch to named workspace
        let res = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "workspace \"2:code\"");
        assert_eq!(res, "{\"success\":true}");
        assert_eq!(
            wm.current_workspace,
            wm.workspace_manager.find_by_name("2:code").unwrap().id
        );

        // 4. Switch back and forth
        let res = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "workspace back_and_forth");
        assert_eq!(res, "{\"success\":true}");
        assert_eq!(wm.current_workspace, 1);

        // 5. Mark window
        let res = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "mark editor_mark");
        assert_eq!(res, "{\"success\":true}");
        let marks = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "get_marks");
        assert!(marks.contains("\"editor_mark\""));

        // 6. Criteria targeting: float window with mark
        let res = send_ipc_cmd(
            &mut stream,
            &mut reader,
            &mut wm,
            "[con_mark=\"editor_mark\"] floating enable",
        );
        assert_eq!(res, "{\"success\":true}");
        assert!(wm.client_state.get_client(win1).unwrap().is_floating());

        // 7. Chained container commands
        let res = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "split v; layout tabbed");
        assert!(res.contains("\"success\":true"));

        // 8. Create second window and test swap
        let _win2 = create_win("Terminal Window");
        let _ = wm.process_pending_events();
        send_ipc_cmd(&mut stream, &mut reader, &mut wm, "mark term_mark");

        let res = send_ipc_cmd(
            &mut stream,
            &mut reader,
            &mut wm,
            "swap container with mark editor_mark",
        );
        assert_eq!(res, "{\"success\":true}");

        // 9. Get tree query
        let tree = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "get_tree");
        assert!(tree.contains("\"type\":\"root\""));
        assert!(tree.contains("\"nodes\":"));

        // 10. Close window
        let res = send_ipc_cmd(&mut stream, &mut reader, &mut wm, "close");
        assert_eq!(res, "{\"success\":true}");
        let _ = wm.process_pending_events();

        // 11. Built-in tilo msg CLI test
        let code = tilo::ipc::run_cli(&["msg".to_string(), "get_workspaces".to_string()]);
        assert_eq!(code, 0);

        drop(stream);
        let _ = wm.perform_graceful_shutdown();
        assert!(!sock_path.exists());
    }
}
