mod explainer;
mod flow;
mod notice;
mod privacy;
mod provider;
mod selection;
mod settings;
mod tray;
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};

use log::LevelFilter;
use tauri::{Manager, RunEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

use flow::Flow;
use notice::NoticeKind;
use settings::SettingsState;

/// State shared between the tray and the selection pipeline.
#[derive(Default)]
pub(crate) struct AppState {
    pub paused: AtomicBool,
}

/// Log file in the app's log folder (`%LOCALAPPDATA%\com.screenbuddy.desktop\logs`), at most
/// 1 MB plus one older file. Debug builds also log to the console. Selected text is never
/// logged in release builds (see `selection::log_event`).
fn log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let our_level = if cfg!(debug_assertions) { LevelFilter::Debug } else { LevelFilter::Info };
    let mut builder = tauri_plugin_log::Builder::new()
        .clear_targets()
        .target(Target::new(TargetKind::LogDir { file_name: Some("screen-buddy".into()) }))
        .max_file_size(1_000_000)
        .rotation_strategy(RotationStrategy::KeepOne)
        // Libraries (HTTP client, web view) only log problems.
        .level(LevelFilter::Warn)
        .level_for("screen_buddy_lib", our_level);
    if cfg!(debug_assertions) {
        builder = builder.target(Target::new(TargetKind::Stdout));
    }
    builder.build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // First, as the plugin requires: a second launch opens Settings in this one instead of
        // starting a second app with a second mouse hook.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Err(err) = crate::windows::settings::open(app) {
                log::warn!("could not open settings: {err}");
            }
        }))
        .plugin(log_plugin())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(AppState::default())
        .on_menu_event(tray::handle_menu_event)
        .invoke_handler(tauri::generate_handler![
            settings::mascot_settings,
            flow::popup_resize,
            flow::popup_close,
            flow::popup_retry,
            flow::popup_use_clipboard,
            flow::popup_copy,
            flow::popup_open_settings,
            tray::show_mascot_menu,
            crate::windows::mascot::mascot_grabbed,
            settings::get_settings,
            settings::save_settings,
            settings::default_block_list,
            settings::windows_user_name,
            settings::list_models,
            settings::check_model,
            settings::clipboard_status,
            settings::reset_mascot_position,
        ])
        .setup(|app| {
            log::info!("Screen Buddy {} starting", app.package_info().version);
            let handle = app.handle();
            // Settings first: everything below starts from them.
            let saved = SettingsState::load(handle);
            let settings = saved.get();
            app.manage(saved);
            app.state::<AppState>().paused.store(settings.paused, Ordering::SeqCst);

            app.manage(crate::windows::flight::Flight::new(handle.clone()));
            crate::windows::mascot::setup(handle, settings.mascot_position)?;
            crate::windows::settings::setup(handle)?;
            // Before the tray and the selection worker, which both use it.
            app.manage(Flow::new(handle.clone(), &settings.model)?);
            // The greeting checks Ollama itself once its bubble is gone.
            if settings.greeting {
                app.state::<Flow>().greet();
            } else {
                app.state::<Flow>().check_ollama();
            }
            tray::setup(handle, settings.paused)?;
            // The app still runs without selection reading; the user is told why.
            if let Err(err) = selection::start(handle.clone()) {
                app.state::<Flow>().notify_startup(NoticeKind::HookFailed, err);
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
