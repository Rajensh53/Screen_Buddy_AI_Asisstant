//! Connects a selection to an answer: shows the popup next to the selection, streams the
//! explanation into it, and tells the mascot what is happening.
//!
//! Flow: selection worker -> `on_selection` -> `buddy` events to the popup and mascot,
//! answer streamed by a Tauri async task. A new selection or a click outside the popup
//! cancels the running answer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::async_runtime::{self, JoinHandle};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::explainer::{self, Explained, Explainer};
use crate::provider::DEFAULT_MODEL;
use crate::provider::ollama::OllamaProvider;
use crate::selection::{Point, Rect, SelectionEvent};
use crate::windows::placement::{self, HeightLimits, Side, Size};
use crate::windows::{POPUP, popup};

/// Event name for `BuddyEvent`, listened to by the popup and the mascot.
pub const BUDDY_EVENT: &str = "buddy";

/// Popup size in logical (CSS) pixels; converted with the monitor's scale factor.
const POPUP_WIDTH: f64 = 360.0;
/// Big enough for a typical word explanation, so most answers never resize the popup.
const POPUP_START_HEIGHT: f64 = 200.0;
const POPUP_MIN_HEIGHT: f64 = 80.0;
const POPUP_MAX_HEIGHT: f64 = 320.0;
const POPUP_HEIGHT_STEP: f64 = 48.0;
/// The selected text shown as the popup's title is cut to this many characters.
const TITLE_CHARS: usize = 80;
/// The popup is shown once it has rendered the new selection and reported its size, so it
/// never flashes the previous answer or jumps in size. This is the fallback if no report comes.
const SHOW_FALLBACK: Duration = Duration::from_millis(250);

/// What the windows are told. Serialized as `{ "kind": "start", ... }`; mirrored in
/// `src/shared/events.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuddyEvent {
    Start { id: u64, title: String },
    Chunk { id: u64, text: String },
    Done { id: u64, cached: bool },
    Error { id: u64, message: String },
    CantRead,
    Hide,
}

/// The popup for the current selection, kept so it can grow next to the same selection and so
/// clicks outside it can be detected.
#[derive(Debug, Clone, Copy)]
struct PopupState {
    /// The selection this popup belongs to (`BuddyEvent` id).
    id: u64,
    /// False until the popup has rendered the new selection (or the fallback fired).
    visible: bool,
    anchor: Rect,
    work_area: Rect,
    scale: f64,
    side: Side,
    bounds: Rect,
}

pub struct Flow {
    app: AppHandle,
    explainer: Arc<Explainer<OllamaProvider>>,
    /// The answer being streamed, if any.
    current: Mutex<Option<JoinHandle<()>>>,
    next_id: AtomicU64,
    popup: Mutex<Option<PopupState>>,
}

impl Flow {
    pub fn new(app: AppHandle) -> reqwest::Result<Self> {
        let explainer = Explainer::new(OllamaProvider::new()?, DEFAULT_MODEL);
        Ok(Self {
            app,
            explainer: Arc::new(explainer),
            current: Mutex::new(None),
            next_id: AtomicU64::new(0),
            popup: Mutex::new(None),
        })
    }

    /// Logs at startup whether Ollama and the model are ready. The app works either way;
    /// the popup shows the same message when a selection is explained.
    pub fn check_ollama(&self) {
        let explainer = Arc::clone(&self.explainer);
        async_runtime::spawn(async move {
            match explainer.health().await {
                Ok(()) => println!("[ollama] ready"),
                Err(err) => println!("[ollama] {err}"),
            }
        });
    }

    /// A new selection: cancel the old answer, show the popup and stream the new answer.
    /// `pointer` is where the mouse was released.
    pub fn on_selection(&self, event: SelectionEvent, pointer: Point) {
        if explainer::clean_input(&event.text).is_none() {
            return;
        }
        self.cancel_answer();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.prepare_popup(id, event.rect, pointer);
        emit(&self.app, BuddyEvent::Start { id, title: title_for(&event.text) });
        let app = self.app.clone();
        thread::spawn(move || {
            thread::sleep(SHOW_FALLBACK);
            app.state::<Flow>().update_popup(id, None, false);
        });

        let app = self.app.clone();
        let explainer = Arc::clone(&self.explainer);
        let text = event.text;
        let task = async_runtime::spawn(async move {
            let result = explainer
                .explain(&text, |piece| emit(&app, BuddyEvent::Chunk { id, text: piece.to_owned() }))
                .await;
            let end = match result {
                Ok(explained) => BuddyEvent::Done { id, cached: explained == Explained::Cached },
                Err(err) => BuddyEvent::Error { id, message: err.to_string() },
            };
            emit(&app, end);
        });
        *lock(&self.current) = Some(task);
    }

    /// Every left-button press: a press outside the visible popup hides it.
    pub fn on_mouse_down(&self, pt: Point) {
        let outside = lock(&self.popup).is_some_and(|popup| !popup.bounds.contains(pt));
        if outside {
            self.hide();
        }
    }

    /// Whether a popup is showing (or about to show) an answer.
    pub fn popup_visible(&self) -> bool {
        lock(&self.popup).is_some()
    }

    /// An app whose text can't be read: the mascot shows it briefly, no popup.
    pub fn on_cant_read(&self) {
        emit(&self.app, BuddyEvent::CantRead);
    }

    /// Hides the popup and stops the running answer.
    pub fn hide(&self) {
        self.cancel_answer();
        let was_shown = lock(&self.popup).take().is_some();
        if was_shown {
            emit(&self.app, BuddyEvent::Hide);
            run_on_popup(&self.app, popup::hide);
        }
    }

    /// Resizes the popup of selection `id` for content that needs `css_height` (CSS pixels;
    /// `complete` once the answer is finished, see `placement::popup_height`) and shows it if
    /// it isn't visible yet. `None` only shows it.
    fn update_popup(&self, id: u64, css_height: Option<f64>, complete: bool) {
        let (bounds, show) = {
            let mut guard = lock(&self.popup);
            let Some(state) = guard.as_mut().filter(|s| s.id == id) else { return };
            let mut resized = false;
            if let Some(css_height) = css_height.filter(|h| h.is_finite()) {
                let physical = |css: f64| (css * state.scale).round() as i32;
                let limits = HeightLimits {
                    min: physical(POPUP_MIN_HEIGHT),
                    max: physical(POPUP_MAX_HEIGHT),
                    step: physical(POPUP_HEIGHT_STEP),
                };
                let height =
                    placement::popup_height(state.bounds.height(), physical(css_height), complete, limits);
                if height != state.bounds.height() {
                    let size = Size { width: state.bounds.width(), height };
                    (state.bounds, state.side) = placement::place(state.anchor, size, state.work_area, state.side);
                    resized = true;
                }
            }
            let show = !state.visible;
            if !show && !resized {
                return;
            }
            state.visible = true;
            (state.bounds, show)
        };
        run_on_popup(&self.app, move |window| {
            if show { popup::show_at(window, bounds) } else { popup::set_bounds(window, bounds) }
        });
    }

    /// Places the popup for selection `id`. It is shown by `update_popup`.
    fn prepare_popup(&self, id: u64, selection: Option<Rect>, pointer: Point) {
        let monitor = self
            .app
            .monitor_from_point(f64::from(pointer.x), f64::from(pointer.y))
            .ok()
            .flatten()
            .or_else(|| self.app.primary_monitor().ok().flatten());
        let Some(monitor) = monitor else {
            eprintln!("[popup] no monitor found");
            return;
        };
        let area = monitor.work_area();
        let work_area = Rect {
            left: area.position.x,
            top: area.position.y,
            right: area.position.x + area.size.width as i32,
            bottom: area.position.y + area.size.height as i32,
        };
        let scale = monitor.scale_factor();
        let size = Size {
            width: (POPUP_WIDTH * scale).round() as i32,
            height: (POPUP_START_HEIGHT * scale).round() as i32,
        };
        let anchor = placement::anchor(selection, pointer, work_area);
        let (bounds, side) = placement::place(anchor, size, work_area, Side::Below);

        *lock(&self.popup) = Some(PopupState { id, visible: false, anchor, work_area, scale, side, bounds });
    }

    fn cancel_answer(&self) {
        if let Some(task) = lock(&self.current).take() {
            // Dropping the future also drops the HTTP request to Ollama.
            task.abort();
        }
    }
}

/// Called by the popup after rendering selection `id`, with the height its content needs
/// (CSS pixels) and whether the answer is complete.
#[tauri::command]
pub fn popup_resize(window: WebviewWindow, flow: State<'_, Flow>, id: u64, height: f64, complete: bool) {
    if window.label() == POPUP {
        flow.update_popup(id, Some(height), complete);
    }
}

/// The selection as a one-line title: whitespace collapsed, cut on a character boundary.
fn title_for(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one_line.char_indices().nth(TITLE_CHARS) {
        Some((cut, _)) => format!("{}…", one_line[..cut].trim_end()),
        None => one_line,
    }
}

fn emit(app: &AppHandle, event: BuddyEvent) {
    if let Err(err) = app.emit(BUDDY_EVENT, &event) {
        eprintln!("[flow] could not emit {BUDDY_EVENT}: {err}");
    }
}

/// Window calls go through the main thread, where Tauri owns the popup window.
fn run_on_popup(app: &AppHandle, f: impl FnOnce(&WebviewWindow) -> popup::Result<()> + Send + 'static) {
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(POPUP)
            && let Err(err) = f(&window)
        {
            eprintln!("[popup] {err}");
        }
    });
    if let Err(err) = result {
        eprintln!("[popup] {err}");
    }
}

/// A panic while holding these locks cannot leave the data half-updated, so keep using it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn events_have_the_shape_the_frontend_expects() {
        let cases = [
            (BuddyEvent::Start { id: 1, title: "word".into() }, json!({"kind": "start", "id": 1, "title": "word"})),
            (BuddyEvent::Chunk { id: 1, text: "Hi".into() }, json!({"kind": "chunk", "id": 1, "text": "Hi"})),
            (BuddyEvent::Done { id: 1, cached: true }, json!({"kind": "done", "id": 1, "cached": true})),
            (BuddyEvent::Error { id: 2, message: "m".into() }, json!({"kind": "error", "id": 2, "message": "m"})),
            (BuddyEvent::CantRead, json!({"kind": "cant_read"})),
            (BuddyEvent::Hide, json!({"kind": "hide"})),
        ];
        for (event, expected) in cases {
            assert_eq!(serde_json::to_value(&event).unwrap(), expected);
        }
    }

    #[test]
    fn title_is_one_line() {
        assert_eq!(title_for("  two\nlines\r\n\tand  tabs "), "two lines and tabs");
    }

    #[test]
    fn long_title_is_cut_on_a_character_boundary() {
        let urdu = "سلام ".repeat(40);
        let title = title_for(&urdu);
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= TITLE_CHARS + 1);
        assert_eq!(title_for(&"a".repeat(TITLE_CHARS)), "a".repeat(TITLE_CHARS));
    }

    #[test]
    fn outside_click_test_uses_the_popup_rect() {
        let bounds = Rect { left: 100, top: 100, right: 460, bottom: 300 };
        assert!(bounds.contains(Point::new(100, 100)));
        assert!(bounds.contains(Point::new(459, 299)));
        assert!(!bounds.contains(Point::new(460, 200)));
        assert!(!bounds.contains(Point::new(50, 200)));
    }
}
