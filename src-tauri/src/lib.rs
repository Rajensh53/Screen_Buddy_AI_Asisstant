mod explainer;
mod flow;
mod privacy;
mod provider;
mod selection;
mod tray;
mod windows;

use std::sync::atomic::AtomicBool;

use tauri::{Manager, RunEvent};

use flow::Flow;

/// State shared between the tray and the selection pipeline.
#[derive(Default)]
pub(crate) struct AppState {
    pub paused: AtomicBool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![flow::popup_resize])
        .setup(|app| {
            let handle = app.handle();
            crate::windows::mascot::setup(handle)?;
            crate::windows::settings::setup(handle)?;
            // Before the tray and the selection worker, which both use it.
            app.manage(Flow::new(handle.clone())?);
            app.state::<Flow>().check_ollama();
            tray::setup(handle)?;
            // The app still runs without selection reading, so log instead of failing.
            if let Err(err) = selection::start(handle.clone()) {
                eprintln!("[selection] not started: {err}");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Screen Buddy");

    app.run(|_app, event| {
        if let RunEvent::Exit = event {
            selection::stop();
        }
    });
}
