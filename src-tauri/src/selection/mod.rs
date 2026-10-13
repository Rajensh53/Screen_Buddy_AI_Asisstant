//! Detects text selections in other apps and reads them with UI Automation.
//!
//! Flow: mouse hook thread -> channel -> selection worker thread
//! (trigger rules -> wait `READ_DELAY` -> UIA reader -> privacy gate -> `SelectionEvent`)
//! -> `Flow`, which shows the popup and streams the explanation.
//! When UI Automation can't give the selection (or gives the shifted text of the Edge/Chrome PDF
//! viewer), the clipboard fallback copies it instead (see `clipboard`).

pub mod clipboard;
mod mouse_hook;
mod process_info;
mod trigger;
mod uia_reader;

use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant};

use ::windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use ::windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::flow::{Flow, PopupAnchor};
use crate::notice::{Context, NoticeKind};
use crate::privacy::{AppIdentity, BlockList, Decision, PrivacyGate, SkipReason};
use crate::settings::SettingsState;
pub use trigger::{Point, prepare_text};
use trigger::{DoubleClickSettings, Gesture, GestureDetector, MouseInput, RepeatFilter};
use uia_reader::{ReadOutcome, Source, UiaReader};

/// Pause after the trigger so the app has finished updating its selection.
const READ_DELAY: Duration = Duration::from_millis(80);

/// Screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn union(self, other: Rect) -> Rect {
        Rect {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.bottom - self.top
    }

    /// Left and top edges are inside, right and bottom edges are outside (as in Win32 RECT).
    pub fn contains(self, pt: Point) -> bool {
        pt.x >= self.left && pt.x < self.right && pt.y >= self.top && pt.y < self.bottom
    }

    pub fn intersects(self, other: Rect) -> bool {
        self.left < other.right && other.left < self.right && self.top < other.bottom && other.top < self.bottom
    }
}

#[derive(Debug, Clone)]
pub struct SelectionEvent {
    /// Trimmed, at least 2 and at most 2000 characters.
    pub text: String,
    pub app_name: String,
    pub process_name: String,
    /// Where the selected text is on screen, if the app reports it.
    pub rect: Option<Rect>,
    /// The process of the top-level window the selection was made in, for copying it again.
    pub window_pid: Option<u32>,
    /// Read through the clipboard rather than UI Automation.
    pub via_clipboard: bool,
}

/// Installs the mouse hook and starts the selection worker.
pub fn start(app: AppHandle) -> Result<(), String> {
    let (tx, rx) = channel();
    mouse_hook::start(tx)?;
    thread::Builder::new()
        .name("selection".into())
        .spawn(move || worker(app, rx))
        .map_err(|e| format!("could not start selection thread: {e}"))?;
    Ok(())
}

/// Removes the mouse hook. Call when the app exits.
pub fn stop() {
    mouse_hook::stop();
}

fn worker(app: AppHandle, rx: Receiver<MouseInput>) {
    let flow = app.state::<Flow>();
    let reader = match UiaReader::new() {
        Ok(r) => r,
        Err(e) => {
            flow.notify_startup(NoticeKind::UiaUnavailable, format!("UI Automation is unavailable: {e}"));
            return;
        }
    };
    let settings_state = app.state::<SettingsState>();
    let own_pid = std::process::id();
    let mut detector = GestureDetector::new(double_click_settings());
    let mut repeats = RepeatFilter::default();

    for input in rx {
        // Any press outside the popup hides it, before it can start a new selection.
        if let MouseInput::LeftDown { pt, .. } = input {
            flow.on_mouse_down(pt);
        }
        let Some(gesture) = detector.feed(input) else { continue };

        if app.state::<AppState>().paused.load(Ordering::SeqCst) {
            continue;
        }
        // Read on every gesture, so changes in the settings window apply at once.
        let settings = settings_state.get();
        let gate = PrivacyGate::new(own_pid, block_list(&settings.block_list));
        let points = match gesture {
            Gesture::Drag { down, up } => [down, up],
            Gesture::DoubleClick { pt } => [pt, pt],
        };
        // Where the gesture ended: the selection should be under the mouse there.
        let at = points[1];
        // Moving or resizing a window (title bar, border, scroll bar, ...) is not a selection.
        // Checked first, so it never shows a notice either.
        if points.iter().any(|pt| process_info::on_window_frame(*pt)) {
            log::debug!("[selection] skipped: gesture on a window frame");
            continue;
        }
        // Notices about this gesture go next to the mouse.
        let notify = |kind: NoticeKind, app_name: &str, reason: Option<&str>| {
            let cx = Context { app: Some(app_name), reason, ..Context::default() };
            flow.notify(kind, cx, PopupAnchor::Selection { rect: None, pointer: at });
        };
        let report_skip = |reason: &SkipReason| match reason {
            SkipReason::OwnWindow => {}
            SkipReason::PasswordField => notify(NoticeKind::PasswordField, "", None),
            SkipReason::BlockedApp(name) => notify(NoticeKind::BlockedApp, name, None),
        };

        // Privacy check before any UI Automation call: our own windows and blocked apps are never read.
        let owners: Vec<(u32, String)> = points
            .iter()
            .filter_map(|pt| process_info::process_at(*pt))
            .map(|pid| (pid, process_info::process_name(pid).unwrap_or_default()))
            .collect();
        let apps: Vec<AppIdentity> =
            owners.iter().map(|(pid, name)| AppIdentity { process_id: *pid, process_name: name }).collect();
        if let Decision::Skip(reason) = gate.check_apps(&apps) {
            report_skip(&reason);
            continue;
        }

        thread::sleep(READ_DELAY);
        // Apps like the new Outlook show content through a helper process (msedgewebview2.exe).
        // Name the app after the window under the mouse (olk.exe), so logs and settings show the real app.
        let app_of = |element_pid: u32| process_info::lookup(process_info::process_at(at).unwrap_or(element_pid));
        // The process that owns the text, as the privacy gate sees it.
        let element_allowed = |process_id: u32| {
            let name = process_info::process_name(process_id).unwrap_or_default();
            match gate.check_element(AppIdentity { process_id, process_name: &name }, false) {
                Decision::Allow => true,
                Decision::Skip(reason) => {
                    report_skip(&reason);
                    false
                }
            }
        };
        // UI Automation first. When it can't give the selection, or gives the shifted text of the
        // Edge/Chrome PDF viewer, the clipboard fallback copies it (see `clipboard` for the rules).
        // Clipboard text has no trustworthy rectangle, so its popup goes next to the mouse.
        // `in_text_area`: UI Automation found text here, so a blocked copy is worth a notice.
        let window_pid = process_info::process_at(at);
        let copy_selection = |in_text_area: bool| -> Copied {
            let Some(window_pid) = window_pid else { return Copied::Failed("the app's window wasn't found") };
            if !settings.clipboard_fallback {
                return Copied::Failed("the clipboard fallback is turned off in Settings");
            }
            // Both ends of a drag: a scroll bar dragged and released over the text doesn't count.
            if !points.iter().all(|pt| reader.may_hold_selection_at(*pt)) {
                return Copied::Failed("the mouse was on a button, menu or image, not on text");
            }
            if let Err(clipboard::Blocked::CloudSync) = clipboard::may_copy(true, clipboard::policy()) {
                if !in_text_area {
                    return Copied::Failed("clipboard sync is on");
                }
                notify(NoticeKind::ClipboardSync, &app_of(window_pid).app_name, None);
                return Copied::Noticed;
            }
            // Terminals get their own copy keys: Ctrl+C would stop the running program there.
            let keys = clipboard::copy_keys(&process_info::process_name(window_pid).unwrap_or_default());
            match clipboard::copy_selection(window_pid, keys) {
                Ok(text) => Copied::Text(text),
                Err(why) => {
                    log::debug!("[selection] clipboard fallback: {why}");
                    Copied::Failed(why)
                }
            }
        };

        let (text, process_id, rect, source) = match reader.read(at) {
            ReadOutcome::Selection { text, process_id, rect, source, pdf_viewer } => {
                if !element_allowed(process_id) {
                    continue;
                }
                // The Edge/Chrome PDF viewer, and apps the user listed as "clipboard first" because
                // their UI Automation text was wrong: their own Copy first, UI Automation if it fails.
                let app_exe = window_pid.and_then(process_info::process_name).unwrap_or_default();
                let copy_first = pdf_viewer || settings.reads_clipboard_first(&app_exe);
                match copy_first.then(|| copy_selection(true)) {
                    Some(Copied::Text(copied)) => (copied, process_id, None, Source::Clipboard),
                    _ => (text, process_id, rect, source),
                }
            }
            ReadOutcome::EmptySelection { process_id, .. } => {
                if !element_allowed(process_id) {
                    continue;
                }
                match copy_selection(true) {
                    Copied::Text(copied) => (copied, process_id, None, Source::Clipboard),
                    Copied::Noticed => continue,
                    Copied::Failed(why) => {
                        let info = app_of(process_id);
                        log::info!("[selection] {} ({}) reports nothing selected; {why}", info.app_name, info.process_name);
                        notify(NoticeKind::NothingSelected, &info.app_name, Some(why));
                        continue;
                    }
                }
            }
            ReadOutcome::Password { .. } => {
                // The reader stopped before reading any text. Never copied either.
                notify(NoticeKind::PasswordField, "", None);
                continue;
            }
            ReadOutcome::NoTextPattern { process_id } => {
                if !element_allowed(process_id) {
                    continue;
                }
                match copy_selection(true) {
                    Copied::Text(copied) => (copied, process_id, None, Source::Clipboard),
                    Copied::Noticed => continue,
                    Copied::Failed(why) => {
                        let info = app_of(process_id);
                        log::info!("[selection] can't read {} ({}); {why}", info.app_name, info.process_name);
                        notify(NoticeKind::CantRead, &info.app_name, Some(why));
                        continue;
                    }
                }
            }
            ReadOutcome::NoSelection | ReadOutcome::NoFocus => {
                // UI Automation sees no text here. Many apps (custom-drawn, Java, some Qt) still
                // copy their selection. The apps under the gesture already passed the gate.
                // Silent when nothing comes of it: these are mostly clicks and drags on things
                // that aren't text (a file list, a drawing), not failed selections.
                let (Copied::Text(copied), Some(process_id)) = (copy_selection(false), window_pid) else {
                    log::debug!("[selection] skipped: no readable text under the mouse");
                    continue;
                };
                (copied, process_id, None, Source::Clipboard)
            }
        };

        if !trigger::gesture_near_selection(gesture, rect) {
            log::debug!("[selection] skipped: the gesture was not on the selected text");
            continue;
        }
        let Some(text) = trigger::prepare_text(&text) else {
            log::debug!("[selection] skipped: selection too short");
            continue;
        };
        // The same text again is only ignored while its popup is still open. After the popup
        // was closed, selecting the same word again should show it again (from the cache).
        if !repeats.accept(&text, Instant::now()) && flow.popup_visible() {
            log::debug!("[selection] skipped: same text, popup still open");
            continue;
        }
        let info = app_of(process_id);
        let event = SelectionEvent {
            text,
            app_name: info.app_name,
            process_name: info.process_name,
            rect,
            window_pid,
            via_clipboard: source == Source::Clipboard,
        };
        log_event(&event, source);
        flow.on_selection(event, at);
    }
}

/// What the clipboard fallback did.
enum Copied {
    Text(String),
    /// It was not allowed, and a notice already told the user why.
    Noticed,
    /// It didn't run or nothing was copied, and why (shown to the user).
    Failed(&'static str),
}

/// The block list from the settings.
fn block_list(entries: &[String]) -> BlockList {
    #[allow(unused_mut)]
    let mut list = BlockList::new(entries);
    // Debug builds only: extra entries for manual testing, for example
    // `$env:SCREEN_BUDDY_EXTRA_BLOCK = "notepad.exe"` before `npm run tauri dev`.
    #[cfg(debug_assertions)]
    if let Ok(extra) = std::env::var("SCREEN_BUDDY_EXTRA_BLOCK") {
        list.extend(extra.split(','));
    }
    list
}

/// Logs a selection. The text itself is logged only in debug builds; the log file of a release
/// build never contains what the user selected.
fn log_event(event: &SelectionEvent, source: Source) {
    let chars = event.text.chars().count();
    log::info!("[selection] {} ({}) chars={chars} via={source:?}", event.app_name, event.process_name);
    if cfg!(debug_assertions) {
        const PREVIEW_CHARS: usize = 120;
        let preview: String = event.text.chars().take(PREVIEW_CHARS).collect();
        let more = if chars > PREVIEW_CHARS { "…" } else { "" };
        log::debug!("[selection] rect={:?} text={preview:?}{more}", event.rect);
    }
}

/// The user's double-click speed and distance from Windows settings.
///
/// GetDoubleClickTime: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdoubleclicktime
/// GetSystemMetrics (SM_CXDOUBLECLK): https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetrics
fn double_click_settings() -> DoubleClickSettings {
    // SAFETY: plain Win32 queries without pointers.
    unsafe {
        DoubleClickSettings {
            time_ms: GetDoubleClickTime(),
            width: GetSystemMetrics(SM_CXDOUBLECLK),
            height: GetSystemMetrics(SM_CYDOUBLECLK),
        }
    }
}
