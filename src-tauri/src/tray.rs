use std::sync::atomic::Ordering;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::AppState;
use crate::windows::settings;

const SETTINGS: &str = "settings";
const PAUSE: &str = "pause";
const QUIT: &str = "quit";
#[cfg(debug_assertions)]
const TEST_POPUP: &str = "test_popup";

/// Event emitted to all windows with the new paused state (`bool`).
pub const PAUSED_CHANGED: &str = "paused-changed";

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let settings_item = MenuItem::with_id(app, SETTINGS, "Settings", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(app, PAUSE, "Pause", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&settings_item, &pause_item, &separator, &quit_item])?;

    #[cfg(debug_assertions)]
    {
        let test_item =
            MenuItem::with_id(app, TEST_POPUP, "Test popup (in 3 s)", true, None::<&str>)?;
        menu.insert(&test_item, 2)?;
    }

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Screen Buddy")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            SETTINGS => {
                if let Err(err) = settings::open(app) {
                    eprintln!("could not open settings: {err}");
                }
            }
            PAUSE => toggle_pause(app, &pause_item),
            QUIT => app.exit(0),
            #[cfg(debug_assertions)]
            TEST_POPUP => debug::test_popup(app),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn toggle_pause(app: &AppHandle, item: &MenuItem<Wry>) {
    let state = app.state::<AppState>();
    // fetch_xor flips the flag and returns the old value.
    let paused = !state.paused.fetch_xor(true, Ordering::SeqCst);

    if let Err(err) = item.set_text(if paused { "Resume" } else { "Pause" }) {
        eprintln!("could not update tray label: {err}");
    }
    if let Err(err) = app.emit(PAUSED_CHANGED, paused) {
        eprintln!("could not emit {PAUSED_CHANGED}: {err}");
    }
}

/// Manual check for the "popup never takes focus" rule, until the real flow exists (Task 6).
#[cfg(debug_assertions)]
mod debug {
    use std::thread;
    use std::time::Duration;

    use tauri::{AppHandle, Manager, PhysicalPosition};

    use crate::windows::{POPUP, popup};

    /// After 3 s, shows the popup next to the cursor without activating it; hides it 5 s later.
    pub fn test_popup(app: &AppHandle) {
        let app = app.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(3));
            run_on_popup(&app, |window| {
                let cursor = window.cursor_position()?;
                let pos = PhysicalPosition::new(cursor.x as i32 + 16, cursor.y as i32 + 16);
                popup::show_at(window, pos)
            });
            thread::sleep(Duration::from_secs(5));
            run_on_popup(&app, popup::hide);
        });
    }

    fn run_on_popup(
        app: &AppHandle,
        f: impl FnOnce(&tauri::WebviewWindow) -> popup::Result<()> + Send + 'static,
    ) {
        let handle = app.clone();
        let result = app.run_on_main_thread(move || {
            if let Some(window) = handle.get_webview_window(POPUP)
                && let Err(err) = f(&window)
            {
                eprintln!("test popup failed: {err}");
            }
        });
        if let Err(err) = result {
            eprintln!("test popup failed: {err}");
        }
    }
}
