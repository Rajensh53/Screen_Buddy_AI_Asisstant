mod tray;
mod windows;

use std::sync::atomic::AtomicBool;

/// State shared between the tray and (later) the selection pipeline.
#[derive(Default)]
pub(crate) struct AppState {
    pub paused: AtomicBool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle();
            crate::windows::mascot::setup(handle)?;
            crate::windows::settings::setup(handle)?;
            tray::setup(handle)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Screen Buddy");
}
