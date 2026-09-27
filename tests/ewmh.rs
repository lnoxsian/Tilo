use std::process::{Child, Command};
use tilo::ewmh::{
    get_net_active_window, get_net_client_list, get_net_current_desktop, get_net_wm_state,
    get_supporting_wm_check,
};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageData, ClientMessageEvent, ConnectionExt, CreateWindowAux, PropMode,
    WindowClass,
};
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
fn test_ewmh_support_in_xephyr() {
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
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM context connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("Failed to initialize WM");
        let root = wm.context.root_window;
        let ewmh = wm.ewmh_atoms;

        // Client connection to inspect and send messages
        let (client_conn, screen_num) =
            x11rb::connect(Some(&server.display)).expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        assert_eq!(screen.root, root);

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

        // 1. Test _NET_SUPPORTING_WM_CHECK
        let check_root = get_supporting_wm_check(&client_conn, root, &ewmh).unwrap();
        assert!(check_root.is_some(), "_NET_SUPPORTING_WM_CHECK must exist on root");
        let supporting_win = check_root.unwrap();
        assert_eq!(supporting_win, wm.supporting_window);

        let check_child = get_supporting_wm_check(&client_conn, supporting_win, &ewmh).unwrap();
        assert_eq!(check_child, Some(supporting_win), "Child window must point to itself");

        // Verify _NET_WM_NAME on supporting window is "tilo"
        let name_reply = client_conn
            .get_property(false, supporting_win, ewmh.net_wm_name, ewmh.utf8_string, 0, 32)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(name_reply.value, b"tilo");

        // 2. Test _NET_CURRENT_DESKTOP and _NET_NUMBER_OF_DESKTOPS
        let current_desktop = get_net_current_desktop(&client_conn, root, &ewmh).unwrap();
        assert_eq!(current_desktop, Some(0)); // 0-indexed for workspace 1

        let num_desktops_reply = client_conn
            .get_property(false, root, ewmh.net_number_of_desktops, AtomEnum::CARDINAL, 0, 1)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(num_desktops_reply.value32().unwrap().next(), Some(9));

        // 3. Test _NET_CLIENT_LIST updates on manage and unmanage
        let initial_clients = get_net_client_list(&client_conn, root, &ewmh).unwrap();
        assert!(initial_clients.is_empty());

        let win1 = create_win();
        assert!(wm.manage_window(win1).unwrap());

        let clients_after_win1 = get_net_client_list(&client_conn, root, &ewmh).unwrap();
        assert_eq!(clients_after_win1, vec![win1]);

        let win2 = create_win();
        assert!(wm.manage_window(win2).unwrap());

        let clients_after_win2 = get_net_client_list(&client_conn, root, &ewmh).unwrap();
        assert_eq!(clients_after_win2, vec![win1, win2]);

        // 4. Test _NET_ACTIVE_WINDOW
        let active = get_net_active_window(&client_conn, root, &ewmh).unwrap();
        assert_eq!(active, Some(win2)); // win2 was the most recently managed/focused window

        wm.focus_client(win1).unwrap();
        let active2 = get_net_active_window(&client_conn, root, &ewmh).unwrap();
        assert_eq!(active2, Some(win1));

        // 5. Test ClientMessage _NET_CURRENT_DESKTOP
        let data = [3u32, 0, 0, 0, 0]; // Switch to desktop 3 (workspace 4)
        let event = ClientMessageEvent::new(
            32,
            root,
            ewmh.net_current_desktop,
            ClientMessageData::from(data),
        );
        wm.handle_client_message(&event).unwrap();
        assert_eq!(wm.current_workspace, 4);

        let desktop_after = get_net_current_desktop(&client_conn, root, &ewmh).unwrap();
        assert_eq!(desktop_after, Some(3));

        // Switch back to desktop 0 (workspace 1)
        let event_back = ClientMessageEvent::new(
            32,
            root,
            ewmh.net_current_desktop,
            ClientMessageData::from([0u32, 0, 0, 0, 0]),
        );
        wm.handle_client_message(&event_back).unwrap();
        assert_eq!(wm.current_workspace, 1);

        // 6. Test ClientMessage _NET_ACTIVE_WINDOW
        let event_focus = ClientMessageEvent::new(
            32,
            win2,
            ewmh.net_active_window,
            ClientMessageData::from([1u32, 0, 0, 0, 0]),
        );
        wm.handle_client_message(&event_focus).unwrap();
        let active3 = get_net_active_window(&client_conn, root, &ewmh).unwrap();
        assert_eq!(active3, Some(win2));

        // 7. Test ClientMessage _NET_WM_STATE for Floating (_NET_WM_STATE_ABOVE)
        assert!(!wm.client_state.get_client(win2).unwrap().floating);
        let event_float = ClientMessageEvent::new(
            32,
            win2,
            ewmh.net_wm_state,
            ClientMessageData::from([1u32, ewmh.net_wm_state_above, 0, 0, 0]), // Add above
        );
        wm.handle_client_message(&event_float).unwrap();
        assert!(wm.client_state.get_client(win2).unwrap().floating);

        let states = get_net_wm_state(&client_conn, win2, &ewmh).unwrap();
        assert!(states.contains(&ewmh.net_wm_state_above));

        // 8. Test ClientMessage _NET_CLOSE_WINDOW
        client_conn
            .change_property32(
                PropMode::REPLACE,
                win1,
                wm.atoms.wm_protocols,
                AtomEnum::ATOM,
                &[wm.atoms.wm_delete_window],
            )
            .unwrap();
        client_conn.flush().unwrap();
        client_conn.sync().unwrap();

        let event_close = ClientMessageEvent::new(
            32,
            win1,
            ewmh.net_close_window,
            ClientMessageData::from([0u32, 0, 0, 0, 0]),
        );
        wm.handle_client_message(&event_close).unwrap();
        // win1 was sent WM_DELETE_WINDOW / unmanaged
        wm.unmanage_window(win1);
        let clients_final = get_net_client_list(&client_conn, root, &ewmh).unwrap();
        assert_eq!(clients_final, vec![win2]);
    }
}
