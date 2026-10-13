use std::sync::atomic::Ordering;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::AppState;
use crate::flow::Flow;
use crate::windows::settings;

const SETTINGS: &str = "settings";
const PAUSE: &str = "pause";
const QUIT: &str = "quit";
#[cfg(debug_assertions)]
const TEST_OLLAMA: &str = "test_ollama";

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
        let ollama_item = MenuItem::with_id(app, TEST_OLLAMA, "Test Ollama", true, None::<&str>)?;
        menu.insert(&ollama_item, 2)?;
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
            TEST_OLLAMA => debug::test_ollama(),
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

    if paused {
        app.state::<Flow>().hide();
    }
    if let Err(err) = item.set_text(if paused { "Resume" } else { "Pause" }) {
        eprintln!("could not update tray label: {err}");
    }
    if let Err(err) = app.emit(PAUSED_CHANGED, paused) {
        eprintln!("could not emit {PAUSED_CHANGED}: {err}");
    }
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
                    eprintln!("[ollama] could not create client: {err}");
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
