//! User settings, saved in `settings.json` in the app data folder (tauri-plugin-store), and the
//! commands the settings window uses. Changes apply at once: the model, the block list (read by
//! the selection worker on every gesture), Pause and the clipboard fallback.

use std::sync::{PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use ::windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameDisplay};
use ::windows::core::PWSTR;
use tauri_plugin_store::StoreExt;

use crate::AppState;
use crate::flow::Flow;
use crate::privacy::BlockList;
use crate::provider::ollama::OllamaProvider;
use crate::provider::{DEFAULT_MODEL, Provider};
use crate::selection::clipboard::{self, ClipboardPolicy};
use crate::tray;
use crate::windows::mascot::{self, Corner};
use crate::windows::SETTINGS;

/// Event emitted to all windows with the new `Settings`.
pub const SETTINGS_CHANGED: &str = "settings-changed";

const STORE_FILE: &str = "settings.json";
const STORE_KEY: &str = "settings";
const MAX_MODEL_CHARS: usize = 100;
const MAX_NAME_CHARS: usize = 40;

/// Everything the user can change. Missing or unknown fields in a saved file fall back to the
/// defaults, so files from older versions keep working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub model: String,
    /// What the mascot calls the user in its greeting. Empty: the first name of the Windows account.
    /// Sent only to the local model.
    pub user_name: String,
    /// Exe names whose text is never read.
    pub block_list: Vec<String>,
    pub paused: bool,
    /// When UI Automation can't give the selection (or gives wrong text, as in the Edge/Chrome PDF
    /// viewer), copy it with Ctrl+C and restore the clipboard (see `selection::clipboard`). On by
    /// default; the user can turn it off.
    pub clipboard_fallback: bool,
    /// Apps (exe names) whose UI Automation text can be wrong: their selection is copied first,
    /// and UI Automation only used if that fails. Needs `clipboard_fallback`. The Edge/Chrome PDF
    /// viewer is handled automatically and needn't be listed.
    pub clipboard_first_apps: Vec<String>,
    /// The mascot flies next to the popup while explaining, and back home when it closes.
    pub fly_to_selection: bool,
    /// A time-of-day greeting next to the mascot when the app starts.
    pub greeting: bool,
    /// Every minute or so of quiet the mascot waves, turns around, patrols or stretches.
    pub idle_play: bool,
    /// The corner the mascot flew to at the last start, so the next start picks another.
    /// Not shown in the settings window. (Older versions saved `mascotPosition`; it is ignored.)
    pub start_corner: Option<Corner>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_owned(),
            user_name: String::new(),
            block_list: BlockList::defaults().entries(),
            paused: false,
            clipboard_fallback: true,
            clipboard_first_apps: Vec::new(),
            fly_to_selection: true,
            greeting: true,
            idle_play: true,
            start_corner: None,
        }
    }
}

impl Settings {
    /// Trims the model name (empty: the default model, at most `MAX_MODEL_CHARS` characters)
    /// and normalizes both app lists like `BlockList` does (lowercase `.exe` names, sorted, unique).
    pub fn normalized(mut self) -> Self {
        let model = self.model.trim();
        self.model = if model.is_empty() { DEFAULT_MODEL.to_owned() } else { model.chars().take(MAX_MODEL_CHARS).collect() };
        self.user_name = self.user_name.trim().chars().take(MAX_NAME_CHARS).collect();
        self.block_list = BlockList::new(&self.block_list).entries();
        self.clipboard_first_apps = BlockList::new(&self.clipboard_first_apps).entries();
        self
    }

    /// Whether the selection in `process_name` (exe, any case) is copied before UI Automation is
    /// asked. Only with the clipboard fallback on: that switch controls all clipboard use.
    pub fn reads_clipboard_first(&self, process_name: &str) -> bool {
        self.clipboard_fallback && BlockList::new(&self.clipboard_first_apps).contains(process_name)
    }
}

/// The name the mascot greets the user with: the one from the settings, or else the first name of
/// the Windows account ("Rajnesh Charan" -> "Rajnesh"). Empty if neither is known.
pub fn greeting_name(settings: &Settings) -> String {
    if settings.user_name.is_empty() { windows_first_name().unwrap_or_default() } else { settings.user_name.clone() }
}

/// The first name of the signed-in Windows user, from the account's display name.
///
/// GetUserNameExW: https://learn.microsoft.com/en-us/windows/win32/api/secext/nf-secext-getusernameexw
fn windows_first_name() -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;
    // SAFETY: `buffer` holds `size` UTF-16 units; the call writes at most that and updates `size`.
    let ok = unsafe { GetUserNameExW(NameDisplay, Some(PWSTR(buffer.as_mut_ptr())), &mut size) };
    if !ok {
        return None;
    }
    first_name(&String::from_utf16_lossy(&buffer[..size as usize]))
}

/// The first word of a display name, at most `MAX_NAME_CHARS` characters.
fn first_name(display_name: &str) -> Option<String> {
    let first = display_name.split_whitespace().next()?;
    Some(first.chars().take(MAX_NAME_CHARS).collect())
}

/// The current settings, shared by the tray, the selection worker and the settings window.
pub struct SettingsState {
    current: RwLock<Settings>,
}

impl SettingsState {
    /// Loads the saved settings, or the defaults if there are none or they can't be read.
    pub fn load(app: &AppHandle) -> Self {
        let saved = app.store(STORE_FILE).ok().and_then(|store| store.get(STORE_KEY));
        let settings = match saved.map(serde_json::from_value::<Settings>) {
            Some(Ok(settings)) => settings.normalized(),
            Some(Err(err)) => {
                log::warn!("[settings] could not read saved settings, using defaults: {err}");
                Settings::default()
            }
            None => Settings::default(),
        };
        Self { current: RwLock::new(settings) }
    }

    pub fn get(&self) -> Settings {
        self.current.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Changes the settings, then saves them, applies what changed and tells all windows.
    pub fn update(&self, app: &AppHandle, change: impl FnOnce(&mut Settings)) -> Settings {
        let (old, new) = {
            let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
            let old = current.clone();
            let mut new = old.clone();
            change(&mut new);
            let new = new.normalized();
            *current = new.clone();
            (old, new)
        };
        if old != new {
            save(app, &new);
            apply(app, &old, &new);
            if let Err(err) = app.emit(SETTINGS_CHANGED, &new) {
                log::warn!("[settings] could not emit {SETTINGS_CHANGED}: {err}");
            }
        }
        new
    }
}

fn save(app: &AppHandle, settings: &Settings) {
    let result = app.store(STORE_FILE).map_err(|e| e.to_string()).and_then(|store| {
        let value = serde_json::to_value(settings).map_err(|e| e.to_string())?;
        store.set(STORE_KEY, value);
        store.save().map_err(|e| e.to_string())
    });
    if let Err(err) = result {
        log::warn!("[settings] could not save settings: {err}");
    }
}

/// Applies the changes that need more than a new value in `SettingsState`. The block list and
/// the clipboard switch are read on every gesture, so they need nothing here.
fn apply(app: &AppHandle, old: &Settings, new: &Settings) {
    if old.model != new.model
        && let Some(flow) = app.try_state::<Flow>()
    {
        flow.set_model(&new.model);
    }
    if old.paused != new.paused {
        app.state::<AppState>().paused.store(new.paused, std::sync::atomic::Ordering::SeqCst);
        tray::show_paused(app, new.paused);
        if new.paused
            && let Some(flow) = app.try_state::<Flow>()
        {
            flow.hide();
        }
    }
}

// --- Commands for the settings window ---

/// The settings window is the only one allowed to read or change settings.
fn only_settings_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == SETTINGS { Ok(()) } else { Err("not allowed from this window".to_owned()) }
}

#[tauri::command]
pub fn get_settings(window: WebviewWindow, state: State<'_, SettingsState>) -> Result<Settings, String> {
    only_settings_window(&window)?;
    Ok(state.get())
}

/// Saves the settings from the window and returns them normalized. The mascot's start corner is
/// the app's own, kept as it is.
#[tauri::command]
pub fn save_settings(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, SettingsState>,
    settings: Settings,
) -> Result<Settings, String> {
    only_settings_window(&window)?;
    Ok(state.update(&app, |current| {
        let corner = current.start_corner;
        *current = settings;
        current.start_corner = corner;
    }))
}

/// What the mascot window needs: whether it is paused, and whether it may play when idle.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MascotSettings {
    paused: bool,
    idle_play: bool,
}

#[tauri::command]
pub fn mascot_settings(window: WebviewWindow, state: State<'_, SettingsState>) -> Result<MascotSettings, String> {
    if window.label() != crate::windows::MASCOT {
        return Err("not allowed from this window".to_owned());
    }
    let s = state.get();
    Ok(MascotSettings { paused: s.paused, idle_play: s.idle_play })
}

/// The first name from Windows, shown as the hint in the settings' name field.
#[tauri::command]
pub fn windows_user_name(window: WebviewWindow) -> Result<Option<String>, String> {
    only_settings_window(&window)?;
    Ok(windows_first_name())
}

/// The built-in block list, for "Restore defaults".
#[tauri::command]
pub fn default_block_list(window: WebviewWindow) -> Result<Vec<String>, String> {
    only_settings_window(&window)?;
    Ok(Settings::default().block_list)
}

/// Models installed in Ollama, for the model field's suggestions.
#[tauri::command]
pub async fn list_models(window: WebviewWindow) -> Result<Vec<String>, String> {
    only_settings_window(&window)?;
    let provider = OllamaProvider::new().map_err(|e| e.to_string())?;
    provider.list_models().await.map_err(|e| e.to_string())
}

/// Whether Ollama runs and has `model`, with the same clear message the popup shows.
#[tauri::command]
pub async fn check_model(window: WebviewWindow, model: String) -> Result<(), String> {
    only_settings_window(&window)?;
    let provider = OllamaProvider::new().map_err(|e| e.to_string())?;
    provider.health(model.trim()).await.map_err(|e| e.to_string())
}

/// Whether Windows clipboard history or cloud sync is on, for the clipboard switch.
#[tauri::command]
pub fn clipboard_status(window: WebviewWindow) -> Result<ClipboardPolicy, String> {
    only_settings_window(&window)?;
    Ok(clipboard::policy())
}

/// Puts the mascot in the bottom-right corner for this session.
#[tauri::command]
pub fn reset_mascot_position(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    only_settings_window(&window)?;
    mascot::move_to_corner(&app).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert_eq!(s.model, DEFAULT_MODEL);
        assert!(s.block_list.contains(&"bitwarden.exe".to_owned()));
        assert!(!s.paused);
        assert!(s.clipboard_fallback);
        assert!(s.fly_to_selection && s.greeting && s.idle_play);
        assert_eq!(s.start_corner, None);
    }

    #[test]
    fn model_is_trimmed_and_never_empty() {
        let s = Settings { model: "  llama3.2:1b \n".into(), ..Settings::default() }.normalized();
        assert_eq!(s.model, "llama3.2:1b");
        let s = Settings { model: "   ".into(), ..Settings::default() }.normalized();
        assert_eq!(s.model, DEFAULT_MODEL);
    }

    #[test]
    fn user_name_is_trimmed_and_cut() {
        let s = Settings { user_name: "  Rajnesh  ".into(), ..Settings::default() }.normalized();
        assert_eq!(s.user_name, "Rajnesh");
        let s = Settings { user_name: "x".repeat(100), ..Settings::default() }.normalized();
        assert_eq!(s.user_name.chars().count(), MAX_NAME_CHARS);
        assert_eq!(Settings::default().user_name, "");
    }

    #[test]
    fn first_name_from_the_windows_display_name() {
        assert_eq!(first_name("Rajnesh Charan"), Some("Rajnesh".into()));
        assert_eq!(first_name("  Rajnesh  "), Some("Rajnesh".into()));
        assert_eq!(first_name("   "), None);
    }

    #[test]
    fn very_long_model_name_is_cut() {
        let s = Settings { model: "m".repeat(500), ..Settings::default() }.normalized();
        assert_eq!(s.model.chars().count(), MAX_MODEL_CHARS);
    }

    #[test]
    fn block_list_is_normalized() {
        let block_list = vec!["Notepad".into(), " notepad.exe".into(), "".into(), "KeePass.EXE".into()];
        let s = Settings { block_list, ..Settings::default() }.normalized();
        assert_eq!(s.block_list, ["keepass.exe", "notepad.exe"]);
    }

    #[test]
    fn clipboard_first_list_is_normalized() {
        let apps = vec!["Antigravity IDE".into(), " antigravity ide.exe".into(), "OLK.EXE".into()];
        let s = Settings { clipboard_first_apps: apps, ..Settings::default() }.normalized();
        assert_eq!(s.clipboard_first_apps, ["antigravity ide.exe", "olk.exe"]);
    }

    #[test]
    fn clipboard_first_matches_listed_apps_only_with_the_fallback_on() {
        let s = Settings { clipboard_first_apps: vec!["olk.exe".into()], ..Settings::default() };
        assert!(s.reads_clipboard_first("OLK.exe"));
        assert!(!s.reads_clipboard_first("notepad.exe"));
        let off = Settings { clipboard_fallback: false, ..s };
        assert!(!off.reads_clipboard_first("olk.exe"));
    }

    #[test]
    fn saved_file_with_missing_and_unknown_fields_loads() {
        // `mascotPosition` and `historyEnabled` were saved by older versions.
        let saved = json!({ "model": "llama3.2:1b", "paused": true, "mascotPosition": { "x": 1, "y": 2 }, "historyEnabled": true });
        let s: Settings = serde_json::from_value(saved).unwrap();
        assert_eq!(s.model, "llama3.2:1b");
        assert!(s.paused);
        assert_eq!(s.block_list, Settings::default().block_list);
        assert!(s.clipboard_fallback);
    }

    #[test]
    fn json_uses_the_names_the_frontend_expects() {
        let s = Settings { start_corner: Some(Corner::TopLeft), ..Settings::default() };
        let value = serde_json::to_value(&s).unwrap();
        for key in [
            "model",
            "userName",
            "blockList",
            "paused",
            "clipboardFallback",
            "clipboardFirstApps",
            "flyToSelection",
            "greeting",
            "idlePlay",
            "startCorner",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["startCorner"], json!("topLeft"));
    }
}
