//! Detects text selections in other apps and reads them with UI Automation.
//!
//! Flow: mouse hook thread -> channel -> selection worker thread
//! (trigger rules -> wait `READ_DELAY` -> UIA reader -> privacy gate -> `SelectionEvent`)
//! -> `Flow`, which shows the popup and streams the explanation.

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
use crate::flow::Flow;
use crate::privacy::{AppIdentity, BlockList, Decision, PrivacyGate, SkipReason};
pub use trigger::Point;
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
    let flow = app.state::<Flow>();
    let gate = PrivacyGate::new(std::process::id(), block_list());
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
        let points = match gesture {
            Gesture::Drag { down, up } => [down, up],
            Gesture::DoubleClick { pt } => [pt, pt],
        };
        // Where the gesture ended: the selection should be under the mouse there.
        let at = points[1];

        // Privacy check before any UI Automation call: our own windows and blocked apps are never read.
        let owners: Vec<(u32, String)> = points
            .iter()
            .filter_map(|pt| process_info::process_at(*pt))
            .map(|pid| (pid, process_info::process_name(pid).unwrap_or_default()))
            .collect();
        let apps: Vec<AppIdentity> =
            owners.iter().map(|(pid, name)| AppIdentity { process_id: *pid, process_name: name }).collect();
        if let Decision::Skip(reason) = gate.check_apps(&apps) {
            log_skip(&reason);
            continue;
        }

        thread::sleep(READ_DELAY);
        // Apps like the new Outlook show content through a helper process (msedgewebview2.exe).
        // Name the app after the window under the mouse (olk.exe), so logs and settings show the real app.
        let app_of = |element_pid: u32| process_info::lookup(process_info::process_at(at).unwrap_or(element_pid));
        // The process that owns the text, as the privacy gate sees it.
        let check_element = |process_id: u32, is_password: bool| {
            let name = process_info::process_name(process_id).unwrap_or_default();
            gate.check_element(AppIdentity { process_id, process_name: &name }, is_password)
        };

        let (text, process_id, rect, source) = match reader.read(at) {
            ReadOutcome::Selection { text, process_id, rect, source } => {
                if let Decision::Skip(reason) = check_element(process_id, false) {
                    log_skip(&reason);
                    continue;
                }
                (text, process_id, rect, source)
            }
            ReadOutcome::EmptySelection { .. } => {
                debug_skip("the app reports nothing selected");
                continue;
            }
            ReadOutcome::Password { process_id } => {
                // The reader stopped before reading any text; the gate records why.
                if let Decision::Skip(reason) = check_element(process_id, true) {
                    log_skip(&reason);
                }
                continue;
            }
            ReadOutcome::NoTextPattern { process_id } => {
                let info = app_of(process_id);
                println!("[selection] can't read this app: {} ({})", info.app_name, info.process_name);
                flow.on_cant_read();
                continue;
            }
            ReadOutcome::NoSelection => {
                debug_skip("no text area under the mouse");
                continue;
            }
            ReadOutcome::NoFocus => {
                debug_skip("nothing readable under the mouse");
                continue;
            }
        };

        if !trigger::gesture_near_selection(gesture, rect) {
            debug_skip("the gesture was not on the selected text");
            continue;
        }
        let Some(text) = trigger::prepare_text(&text) else {
            debug_skip("selection too short");
            continue;
        };
        // The same text again is only ignored while its popup is still open. After the popup
        // was closed, selecting the same word again should show it again (from the cache).
        if !repeats.accept(&text, Instant::now()) && flow.popup_visible() {
            debug_skip("same text, popup still open");
            continue;
        }
        let info = app_of(process_id);
        let event = SelectionEvent { text, app_name: info.app_name, process_name: info.process_name, rect };
        log_event(&event, source);
        flow.on_selection(event, at);
    }
}


/// The block list. Defaults until settings exist (Task 7).
fn block_list() -> BlockList {
    #[allow(unused_mut)]
    let mut list = BlockList::defaults();
    // Debug builds only: extra entries for manual testing, for example
    // `$env:SCREEN_BUDDY_EXTRA_BLOCK = "notepad.exe"` before `npm run tauri dev`.
    #[cfg(debug_assertions)]
    if let Ok(extra) = std::env::var("SCREEN_BUDDY_EXTRA_BLOCK") {
        list.extend(extra.split(','));
    }
    list
}

/// Logs why a selection was skipped. Never logs any text.
fn log_skip(reason: &SkipReason) {
    match reason {
        SkipReason::OwnWindow => {}
        SkipReason::PasswordField => println!("[privacy] skipped: password field"),
        SkipReason::BlockedApp(name) => println!("[privacy] skipped: blocked app {name}"),
    }
}

/// Debug builds only: why a gesture did not show a popup, to diagnose "nothing happened".
/// Never logs any text.
fn debug_skip(why: &str) {
    if cfg!(debug_assertions) {
        println!("[selection] skipped: {why}");
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
