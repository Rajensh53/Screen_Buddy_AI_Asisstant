//! Detects text selections in other apps and reads them with UI Automation.
//!
//! Flow: mouse hook thread -> channel -> selection worker thread
//! (trigger rules -> wait `READ_DELAY` -> UIA reader -> `SelectionEvent`).
//! For now events are only logged to the console; the explainer is wired in Task 6.

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
}

#[derive(Debug, Clone)]
pub struct SelectionEvent {
    /// Trimmed, at least 2 and at most 2000 characters.
    pub text: String,
    pub app_name: String,
    pub process_name: String,
    /// Where the selected text is on screen, if the app reports it.
    pub rect: Option<Rect>,
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
    let reader = match UiaReader::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[selection] UI Automation unavailable, selection reading is off: {e}");
            return;
        }
    };
    let own_pid = std::process::id();
    let mut detector = GestureDetector::new(double_click_settings());
    let mut repeats = RepeatFilter::default();

    for input in rx {
        let Some(gesture) = detector.feed(input) else { continue };

        if app.state::<AppState>().paused.load(Ordering::SeqCst) {
            continue;
        }
        // Dragging the mascot or selecting inside our own windows is not a selection to explain.
        let points = match gesture {
            Gesture::Drag { down, up } => [down, up],
            Gesture::DoubleClick { pt } => [pt, pt],
        };
        if points.iter().any(|pt| process_info::process_at(*pt) == Some(own_pid)) {
            continue;
        }

        // Where the gesture ended: the selection should be under the mouse there.
        let at = match gesture {
            Gesture::Drag { up, .. } => up,
            Gesture::DoubleClick { pt } => pt,
        };

        thread::sleep(READ_DELAY);
        // Apps like the new Outlook show content through a helper process (msedgewebview2.exe).
        // Name the app after the window under the mouse (olk.exe), so the block list sees the real app.
        let app_of = |element_pid: u32| process_info::lookup(process_info::process_at(at).unwrap_or(element_pid));
        match reader.read(at) {
            ReadOutcome::Selection { text, process_id, rect, source } => {
                if process_id == own_pid || !trigger::gesture_near_selection(gesture, rect) {
                    continue;
                }
                let Some(text) = trigger::prepare_text(&text) else { continue };
                if !repeats.accept(&text, Instant::now()) {
                    continue;
                }
                let info = app_of(process_id);
                let event =
                    SelectionEvent { text, app_name: info.app_name, process_name: info.process_name, rect };
                log_event(&event, source);
            }
            ReadOutcome::Password { process_id } => {
                println!("[selection] skipped: password field in {}", app_of(process_id).process_name);
            }
            ReadOutcome::NoTextPattern { process_id } => {
                let info = app_of(process_id);
                println!("[selection] can't read this app: {} ({})", info.app_name, info.process_name);
            }
            ReadOutcome::NoSelection | ReadOutcome::NoFocus => {}
        }
    }
}

/// Logs to the console (visible in `npm run tauri dev`; release builds have no console).
fn log_event(event: &SelectionEvent, source: Source) {
    const PREVIEW_CHARS: usize = 120;
    let chars = event.text.chars().count();
    let preview: String = event.text.chars().take(PREVIEW_CHARS).collect();
    let more = if chars > PREVIEW_CHARS { "…" } else { "" };
    // Which fallback found the text, to learn what real apps need.
    let via = if cfg!(debug_assertions) { format!(" via={source:?}") } else { String::new() };
    println!(
        "[selection] {} ({}) rect={:?} chars={} text={:?}{}{}",
        event.app_name, event.process_name, event.rect, chars, preview, more, via
    );
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
