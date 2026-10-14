use ::windows::Win32::UI::WindowsAndMessaging::{SW_SHOWMINNOACTIVE, ShowWindow};
use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent};

/// The settings window is also the app's button on the taskbar (the mascot and the popup stay
/// off it: they never take focus). It starts minimized there, a click on the button opens it, and
/// closing it minimizes it back, so the app keeps running. Quit is in the tray and mascot menus.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(super::SETTINGS) else {
        return Ok(());
    };

    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(err) = handle.minimize() {
                log::warn!("could not minimize settings window: {err}");
            }
        }
    });
    show_minimized(&window)
}

pub fn open(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(super::SETTINGS) else {
        return Ok(());
    };
    window.show()?;
    window.unminimize()?;
    window.set_focus()
}

/// Puts the window on the taskbar, minimized, without activating it: the app the user is in keeps
/// focus at startup.
///
/// ShowWindow: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow
fn show_minimized(window: &WebviewWindow) -> tauri::Result<()> {
    let hwnd = window.hwnd()?;
    // SAFETY: `hwnd` is the live top-level window owned by this Tauri webview window.
    // The return value is the previous visibility, not an error.
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWMINNOACTIVE) };
    Ok(())
}
