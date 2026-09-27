use std::process::{Child, Command};
use tilo::layout::{calculate_master_stack, LayoutConfig, Rect};
use tilo::wm::WindowManager;
use tilo::x11::X11Context;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateWindowAux, WindowClass,
};

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
fn test_master_stack_layout_math() {
    let screen = Rect::new(0, 0, 1920, 1080);
    let config = LayoutConfig {
        master_ratio: 0.55,
        inner_gap: 10,
        outer_gap: 15,
        border_width: 2,
    };

    // 0 windows
    assert!(calculate_master_stack(&[], &screen, &config).is_empty());

    // 1 window
    let r1 = calculate_master_stack(&[1], &screen, &config);
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0].x, 15);
    assert_eq!(r1[0].y, 15);
    assert_eq!(r1[0].width, 1920 - 30 - 4);
    assert_eq!(r1[0].height, 1080 - 30 - 4);

    // 2 windows
    let r2 = calculate_master_stack(&[1, 2], &screen, &config);
    assert_eq!(r2.len(), 2);
    assert!(!r2[0].overlaps(&r2[1]));

    // 4 windows
    let r4 = calculate_master_stack(&[1, 2, 3, 4], &screen, &config);
    assert_eq!(r4.len(), 4);
    for i in 0..r4.len() {
        for j in (i + 1)..r4.len() {
            assert!(!r4[i].overlaps(&r4[j]), "Rect {} and {} overlap", i, j);
        }
    }
}

#[test]
fn test_tiling_live_in_xephyr() {
    if !Command::new("which").arg("Xephyr").output().map(|o| o.status.success()).unwrap_or(false) {
        return;
    }

    let xephyr = XephyrServer::start(90);
    if let Some(server) = xephyr {
        let wm_ctx = X11Context::connect_to(Some(&server.display))
            .expect("WM connect failed");
        assert!(wm_ctx.become_wm().is_ok());

        let mut wm = WindowManager::new(wm_ctx).expect("WM init failed");
        wm.layout_config = LayoutConfig {
            master_ratio: 0.50,
            inner_gap: 10,
            outer_gap: 10,
            border_width: 2,
        };

        let (client_conn, screen_num) = x11rb::connect(Some(&server.display))
            .expect("Client connect failed");
        let screen = &client_conn.setup().roots[screen_num];
        let root = screen.root;

        // Helper to spawn a client window
        let create_win = |conn: &x11rb::rust_connection::RustConnection| -> u32 {
            let win = conn.generate_id().unwrap();
            conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                0,
                0,
                100,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::default(),
            )
            .unwrap();
            conn.flush().unwrap();
            win
        };

        // Window 1: Becomes single full tiled window
        let win1 = create_win(&client_conn);
        assert!(wm.manage_window(win1).unwrap());

        let g1 = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        // Screen is 1000x800. Outer gap 10, border 2.
        // Expected width: 1000 - 2*10 - 2*2 = 976
        // Expected height: 800 - 2*10 - 2*2 = 776
        assert_eq!(g1.x, 10);
        assert_eq!(g1.y, 10);
        assert_eq!(g1.width, 976);
        assert_eq!(g1.height, 776);

        // Window 2: Window 1 becomes Master, Window 2 becomes Stack
        let win2 = create_win(&client_conn);
        assert!(wm.manage_window(win2).unwrap());

        let g1_two = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        let g2_two = client_conn.get_geometry(win2).unwrap().reply().unwrap();

        // Win1 (master) should be on left
        assert_eq!(g1_two.x, 10);
        // Win2 (stack) should be on right
        assert!(g2_two.x > g1_two.x + g1_two.width as i16);
        // Neither overlaps
        assert!(g1_two.x + g1_two.width as i16 <= g2_two.x);

        // Window 3: Window 1 stays Master, Window 2 and 3 share Stack vertically
        let win3 = create_win(&client_conn);
        assert!(wm.manage_window(win3).unwrap());

        let g2_three = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        let g3_three = client_conn.get_geometry(win3).unwrap().reply().unwrap();

        // Win2 and Win3 are in stack column (same x)
        assert_eq!(g2_three.x, g3_three.x);
        // Win3 is below Win2 (g3.y > g2.y)
        assert!(g3_three.y > g2_three.y);
        assert!(g2_three.y + g2_three.height as i16 <= g3_three.y);

        // Close Window 3: Win2 re-expands to full stack height
        wm.unmanage_window(win3);
        let g2_after_unmanage = client_conn.get_geometry(win2).unwrap().reply().unwrap();
        assert!(g2_after_unmanage.height > g2_three.height);

        // Close Window 2: Win1 re-expands to single window full screen
        wm.unmanage_window(win2);
        let g1_after_unmanage = client_conn.get_geometry(win1).unwrap().reply().unwrap();
        assert_eq!(g1_after_unmanage.width, 976);
        assert_eq!(g1_after_unmanage.height, 776);

        wm.stop();
    }
}
