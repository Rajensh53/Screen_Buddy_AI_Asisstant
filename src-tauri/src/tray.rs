//! The tray icon and the mascot's right-click menu. Both menus use the same item ids, and all
//! clicks go to `handle_menu_event` (registered once with `app.on_menu_event`).

use std::thread;
use std::time::Duration;

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, Wry};

use crate::settings::SettingsState;
use crate::windows::{MASCOT, POPUP, popup, settings};

const SETTINGS: &str = "settings";
const PAUSE: &str = "pause";
const QUIT: &str = "quit";
#[cfg(debug_assertions)]
const TEST_OLLAMA: &str = "test_ollama";
#[cfg(debug_assertions)]
const PREVIEW_MASCOT: &str = "preview_mascot";

/// Sent to the mascot on Quit: it glitches away into a beam of light (Mascot.tsx).
const MASCOT_EXIT: &str = "mascot-exit";
/// How long that takes before the app closes.
const EXIT_ANIMATION: Duration = Duration::from_millis(650);

/// Event emitted to all windows with the new paused state (`bool`).
pub const PAUSED_CHANGED: &str = "paused-changed";

/// The tray's Pause/Resume item, kept so its label follows the setting.
struct PauseItem(MenuItem<Wry>);

/// `paused`: the saved setting, so the label is right from the start.
pub fn setup(app: &AppHandle, paused: bool) -> tauri::Result<()> {
    let menu = build_menu(app, paused)?;
    #[cfg(debug_assertions)]
    {
        let ollama_item = MenuItem::with_id(app, TEST_OLLAMA, "Test Ollama", true, None::<&str>)?;
        menu.insert(&ollama_item, 2)?;
        let preview_item = MenuItem::with_id(app, PREVIEW_MASCOT, "Preview mascot states", true, None::<&str>)?;
        menu.insert(&preview_item, 3)?;
    }

    let mut builder = TrayIconBuilder::with_id("main").tooltip("Screen Buddy").menu(&menu);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Settings, Pause/Resume, a separator and Quit. The Pause item of the first menu (the tray's)
/// is kept so its label can follow the setting.
fn build_menu(app: &AppHandle, paused: bool) -> tauri::Result<Menu<Wry>> {
    let settings_item = MenuItem::with_id(app, SETTINGS, "Settings", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(app, PAUSE, pause_label(paused), true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&settings_item, &pause_item, &separator, &quit_item])?;
    if app.try_state::<PauseItem>().is_none() {
        app.manage(PauseItem(pause_item));
    }
    Ok(menu)
}

/// Every menu click, from the tray or the mascot's menu.
pub fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        SETTINGS => {
            if let Err(err) = settings::open(app) {
                log::warn!("could not open settings: {err}");
            }
        }
        // Saved like any other setting; `show_paused` then updates the label and the windows.
        PAUSE => {
            app.state::<SettingsState>().update(app, |s| s.paused = !s.paused);
        }
        QUIT => quit(app),
        #[cfg(debug_assertions)]
        TEST_OLLAMA => debug::test_ollama(),
        // The mascot shows each of its looks for 2 s (Mascot.tsx, debug builds only).
        #[cfg(debug_assertions)]
        PREVIEW_MASCOT => {
            if let Err(err) = app.emit("mascot-preview", ()) {
                log::warn!("could not emit mascot-preview: {err}");
            }
        }
        _ => {}
    }
}

/// Quit: the popup goes, the mascot plays its exit glitch, then the app closes.
fn quit(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(POPUP) {
        let _ = popup::hide(&window);
    }
    if let Err(err) = app.emit_to(MASCOT, MASCOT_EXIT, ()) {
        log::warn!("could not emit {MASCOT_EXIT}: {err}");
    }
    let handle = app.clone();
    let spawned = thread::Builder::new().name("quit".into()).spawn(move || {
        thread::sleep(EXIT_ANIMATION);
        handle.exit(0);
    });
    if spawned.is_err() {
        app.exit(0);
    }
}

/// Right-click on the mascot: the same menu as the tray, next to the mouse.
#[tauri::command]
pub fn show_mascot_menu(window: WebviewWindow, app: AppHandle, settings: State<'_, SettingsState>) -> Result<(), String> {
    if window.label() != MASCOT {
        return Err("not allowed from this window".to_owned());
    }
    let menu = build_menu(&app, settings.get().paused).map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

/// Shows the new paused state in the tray label and tells the windows (the mascot dims).
pub fn show_paused(app: &AppHandle, paused: bool) {
    if let Some(item) = app.try_state::<PauseItem>()
        && let Err(err) = item.0.set_text(pause_label(paused))
    {
        log::warn!("could not update tray label: {err}");
    }
    if let Err(err) = app.emit(PAUSED_CHANGED, paused) {
        log::warn!("could not emit {PAUSED_CHANGED}: {err}");
    }
}

fn pause_label(paused: bool) -> &'static str {
    if paused { "Resume" } else { "Pause" }
}
/// Manual check of the Ollama connection and the prompt, without selecting anything.
#[cfg(debug_assertions)]
mod debug {
    use std::io::Write;
    use std::sync::OnceLock;
    use std::time::Instant;

    use crate::explainer::{Explained, Explainer};
    use crate::provider::DEFAULT_MODEL;
    use crate::provider::ollama::OllamaProvider;

    /// Kept for the whole run, so a second click shows a cached answer.
    /// `$env:SCREEN_BUDDY_TEST_MODEL = "no-such-model"` tests the "model missing" message.
    fn explainer() -> Option<&'static Explainer<OllamaProvider>> {
        static EXPLAINER: OnceLock<Option<Explainer<OllamaProvider>>> = OnceLock::new();
        EXPLAINER
            .get_or_init(|| match OllamaProvider::new() {
                Ok(provider) => {
                    let model =
                        std::env::var("SCREEN_BUDDY_TEST_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
                    Some(Explainer::new(provider, model))
                }
                Err(err) => {
                    log::warn!("[ollama] could not create client: {err}");
                    None
                }
            })
            .as_ref()
    }

    /// Checks Ollama, then explains a fixed test text with the real prompt and streams the
    /// answer to the console. Never sends any selected text.
    /// `$env:SCREEN_BUDDY_TEST_TEXT = "..."` sets another test text.
    pub fn test_ollama() {
        tauri::async_runtime::spawn(async {
            let Some(explainer) = explainer() else { return };
            if let Err(err) = explainer.health().await {
                return println!("[ollama] {err}");
            }
            let text = std::env::var("SCREEN_BUDDY_TEST_TEXT").unwrap_or_else(|_| "serendipity".to_owned());
            println!("[ollama] ok. Explaining {text:?}...");

            let start = Instant::now();
            let mut first_piece = None;
            let result = explainer
                .explain(&text, |piece| {
                    first_piece.get_or_insert_with(|| start.elapsed());
                    print!("{piece}");
                    let _ = std::io::stdout().flush();
                })
                .await;
            println!();
            match result {
                Ok(Explained::Fresh) => println!(
                    "[ollama] first piece after {:.1?}, done after {:.1?}",
                    first_piece.unwrap_or_default(),
                    start.elapsed()
                ),
                Ok(Explained::Cached) => println!("[ollama] (from cache) in {:.1?}", start.elapsed()),
                Ok(Explained::NothingToExplain) => println!("[ollama] nothing to explain"),
                Err(err) => println!("[ollama] {err}"),
            }
        });
    }
}
