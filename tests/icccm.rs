use std::process::{Child, Command};
use tilo::icccm::{get_wm_hints, get_wm_normal_hints, WmState};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass,
};
use x11rb::protocol::Event;
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

#[test]
fn test_icccm_protocols_and_wm_state_in_xephyr() {
    if !Command::new("which")
        .arg("Xephyr")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return;
    }

    let xephyr = XephyrServer::start(95);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");

        // Client connection to create windows
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        let create_win = || {
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
                    &CreateWindowAux::default(),
                )
                .expect("Failed to create client window");
            client_conn.flush().unwrap();
            client_conn.sync().unwrap();
            win
        };

        // 1. Test WM_STATE on manage, workspace switch, and unmanage
        let win1 = create_win();
        assert!(wm.manage_window(win1).unwrap());

        // Managed & mapped window has WM_STATE == Normal
        let state = wm.get_wm_state(win1).unwrap();
        assert_eq!(state, Some(WmState::Normal));

        // Switch to workspace 2: win1 becomes Iconic
        wm.switch_workspace(2).unwrap();
        let state_inactive = wm.get_wm_state(win1).unwrap();
        assert_eq!(state_inactive, Some(WmState::Iconic));

        // Switch back to workspace 1: win1 becomes Normal again
        wm.switch_workspace(1).unwrap();
        let state_active = wm.get_wm_state(win1).unwrap();
        assert_eq!(state_active, Some(WmState::Normal));

        // 2. Test WM_TAKE_FOCUS and WM_DELETE_WINDOW support via WM_PROTOCOLS
        let wm_delete = wm.atoms.wm_delete_window;
        let wm_take_focus = wm.atoms.wm_take_focus;
        let wm_protocols = wm.atoms.wm_protocols;
        client_conn
            .change_property32(
                PropMode::REPLACE,
                win1,
                wm_protocols,
                AtomEnum::ATOM,
                &[wm_delete, wm_take_focus],
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        // WM focuses window and should send WM_TAKE_FOCUS
        wm.focus_client(win1).expect("focus_client failed");
        client_conn.sync().unwrap();

        let mut received_take_focus = false;
        while let Some(event) = client_conn.poll_for_event().unwrap() {
            if let Event::ClientMessage(msg) = event {
                if msg.window == win1 && msg.type_ == wm_protocols {
                    let data = msg.data.as_data32();
                    if data[0] == wm_take_focus {
                        received_take_focus = true;
                        break;
                    }
                }
            }
        }
        assert!(received_take_focus, "Client should receive WM_TAKE_FOCUS ClientMessage");

        // WM requests window close
        wm.kill_client(win1).expect("kill_client failed");
        client_conn.sync().unwrap();

        // Client checks for ClientMessage event
        let mut received_delete = false;
        while let Some(event) = client_conn.poll_for_event().unwrap() {
            if let Event::ClientMessage(msg) = event {
                if msg.window == win1 && msg.type_ == wm_protocols {
                    let data = msg.data.as_data32();
                    if data[0] == wm_delete {
                        received_delete = true;
                        break;
                    }
                }
            }
        }
        assert!(received_delete, "Client should receive WM_DELETE_WINDOW ClientMessage");

        // 3. Test KillClient fallback for client without WM_DELETE_WINDOW
        let (doomed_conn, _) =
            x11rb::connect(Some(&server.display)).expect("Doomed client connect failed");
        let win2 = doomed_conn.generate_id().unwrap();
        doomed_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win2,
                root,
                0,
                0,
                200,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .unwrap();
        doomed_conn.flush().unwrap();
        doomed_conn.sync().unwrap();

        assert!(wm.manage_window(win2).unwrap());
        // win2 does NOT have WM_DELETE_WINDOW in WM_PROTOCOLS
        wm.kill_client(win2).expect("kill_client win2 failed");
        wm.context.flush().unwrap();

        // doomed_conn should now be disconnected by the X server due to KillClient
        assert!(
            doomed_conn.sync().is_err(),
            "doomed_conn should be disconnected by KillClient"
        );

        // 4. Test WM_NORMAL_HINTS size hints parsing
        let win3 = create_win();
        // flags: PMinSize (1<<4) | PMaxSize (1<<5)
        let flags = (1 << 4) | (1 << 5);
        let mut normal_hints_data = vec![0u32; 18];
        normal_hints_data[0] = flags;
        normal_hints_data[5] = 300; // min_width
        normal_hints_data[6] = 200; // min_height
        normal_hints_data[7] = 800; // max_width
        normal_hints_data[8] = 600; // max_height

        client_conn
            .change_property32(
                PropMode::REPLACE,
                win3,
                AtomEnum::WM_NORMAL_HINTS,
                AtomEnum::WM_SIZE_HINTS,
                &normal_hints_data,
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        let parsed_hints = get_wm_normal_hints(&wm.context.connection, win3)
            .unwrap()
            .expect("Should parse WM_NORMAL_HINTS");
        assert_eq!(parsed_hints.min_width, Some(300));
        assert_eq!(parsed_hints.min_height, Some(200));
        assert_eq!(parsed_hints.max_width, Some(800));
        assert_eq!(parsed_hints.max_height, Some(600));

        // 5. Test WM_HINTS urgency hint
        let mut hints_data = vec![0u32; 3];
        hints_data[0] = 1 << 8; // URGENCY_HINT flag
        client_conn
            .change_property32(
                PropMode::REPLACE,
                win3,
                AtomEnum::WM_HINTS,
                AtomEnum::WM_HINTS,
                &hints_data,
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        let parsed_wm_hints = get_wm_hints(&wm.context.connection, win3)
            .unwrap()
            .expect("Should parse WM_HINTS");
        assert!(parsed_wm_hints.is_urgent, "Urgency hint should be set");

        // 6. Test unmanage sets WM_STATE to Withdrawn
        wm.unmanage_window(win1);
        let state_unmanaged = wm.get_wm_state(win1).unwrap();
        assert_eq!(state_unmanaged, Some(WmState::Withdrawn));
    }
}
