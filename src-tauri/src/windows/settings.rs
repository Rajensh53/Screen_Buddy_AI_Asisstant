use tauri::{AppHandle, Manager, WindowEvent};

/// Closing the settings window hides it, so the app keeps running in the tray.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(super::SETTINGS) else {
        return Ok(());
    };

    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(err) = handle.hide() {
                eprintln!("could not hide settings window: {err}");
            }
        }
    });
    Ok(())
}

pub fn open(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(super::SETTINGS) else {
        return Ok(());
    };
    window.show()?;
    window.unminimize()?;
    window.set_focus()
}
