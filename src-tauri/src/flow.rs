//! Connects a selection to an answer: shows the popup next to the selection, streams the
//! explanation into it, and tells the mascot what is happening. Problems the user should know
//! about (an app that can't be read, a privacy skip, Ollama not running, ...) are shown in the
//! same popup as short notices (see `notice`).
//!
//! Flow: selection worker -> `on_selection` -> `buddy` events to the popup and mascot,
//! answer streamed by a Tauri async task. A new selection or a click outside the popup
//! cancels the running answer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::async_runtime::{self, JoinHandle};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::explainer::{self, Explained, Explainer};
use crate::notice::{self, Context, Notice, NoticeKind, NoticeThrottle};
use crate::provider::ProviderError;
use crate::provider::ollama::OllamaProvider;
use crate::selection::{Point, Rect, SelectionEvent, clipboard};
use crate::windows::placement::{self, HeightLimits, Side, Size};
use crate::windows::{self as app_windows, POPUP, mascot, popup};

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
/// Startup notices wait until the windows have loaded and listen for events.
const STARTUP_NOTICE_DELAY: Duration = Duration::from_secs(3);

/// What the windows are told. Serialized as `{ "kind": "start", ... }`; mirrored in
/// `src/shared/events.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuddyEvent {
    Start { id: u64, title: String },
    Chunk { id: u64, text: String },
    Done { id: u64, cached: bool },
    Error { id: u64, message: String },
    /// A problem shown in the popup instead of an answer.
    Notice { id: u64, notice: Notice },
    CantRead,
    Hide,
}

/// What a popup is placed next to.
#[derive(Debug, Clone, Copy)]
pub enum PopupAnchor {
    /// The selection (or the pointer, where the mouse was released, if its rectangle is unknown).
    Selection { rect: Option<Rect>, pointer: Point },
    /// The mascot: for problems at startup, which have no selection.
    Mascot,
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
    /// The last selection, in memory only, for Retry.
    last_selection: Mutex<Option<(SelectionEvent, Point)>>,
    throttle: Mutex<NoticeThrottle>,
}

impl Flow {
    /// `model` comes from the settings.
    pub fn new(app: AppHandle, model: &str) -> reqwest::Result<Self> {
        let explainer = Explainer::new(OllamaProvider::new()?, model);
        Ok(Self {
            app,
            explainer: Arc::new(explainer),
            current: Mutex::new(None),
            next_id: AtomicU64::new(0),
            popup: Mutex::new(None),
            last_selection: Mutex::new(None),
            throttle: Mutex::new(NoticeThrottle::default()),
        })
    }

    /// The settings changed the model: the next selection uses it.
    pub fn set_model(&self, model: &str) {
        self.explainer.set_model(model);
    }

    /// Checks at startup that Ollama runs and has the model. If not, a notice next to the
    /// mascot says what to do, so the first selection doesn't fail without warning.
    pub fn check_ollama(&self) {
        let app = self.app.clone();
        let spawned = thread::Builder::new().name("ollama-check".into()).spawn(move || {
            thread::sleep(STARTUP_NOTICE_DELAY);
            let flow = app.state::<Flow>();
            match async_runtime::block_on(flow.explainer.health()) {
                Ok(()) => log::info!("[ollama] ready"),
                Err(err) => {
                    log::warn!("[ollama] {err}");
                    if let Some(kind) = ollama_notice(&err) {
                        let model = flow.explainer.model();
                        let cx = Context { model: Some(&model), at_startup: true, ..Context::default() };
                        flow.notify(kind, cx, PopupAnchor::Mascot);
                    }
                }
            }
        });
        if let Err(err) = spawned {
            log::warn!("[ollama] startup check not run: {err}");
        }
    }

    /// A problem at startup (the mouse hook or UI Automation didn't start): shown next to the
    /// mascot once the windows are ready.
    pub fn notify_startup(&self, kind: NoticeKind, reason: String) {
        log::error!("[startup] {kind:?}: {reason}");
        let app = self.app.clone();
        let _ = thread::Builder::new().name("startup-notice".into()).spawn(move || {
            thread::sleep(STARTUP_NOTICE_DELAY);
            let cx = Context { reason: Some(&reason), at_startup: true, ..Context::default() };
            app.state::<Flow>().notify(kind, cx, PopupAnchor::Mascot);
        });
    }

    /// Shows a notice in the popup. The same notice for the same app is shown once every few
    /// minutes (`notice::REPEAT_AFTER`); in between only the mascot reacts.
    pub fn notify(&self, kind: NoticeKind, cx: Context, anchor: PopupAnchor) {
        let shown = lock(&self.throttle).should_show(kind, cx.app.unwrap_or(""), Instant::now());
        if !shown {
            emit(&self.app, BuddyEvent::CantRead);
            return;
        }
        log::info!("[notice] {kind:?}");
        self.cancel_answer();
        let id = self.next_id();
        self.open_popup(id, anchor);
        emit(&self.app, BuddyEvent::Notice { id, notice: notice::notice(kind, cx) });
    }

    /// A new selection: cancel the old answer, show the popup and stream the new answer.
    /// `pointer` is where the mouse was released.
    pub fn on_selection(&self, event: SelectionEvent, pointer: Point) {
        if explainer::clean_input(&event.text).is_none() {
            return;
        }
        self.cancel_answer();
        *lock(&self.last_selection) = Some((event.clone(), pointer));
        let id = self.next_id();
        self.open_popup(id, PopupAnchor::Selection { rect: event.rect, pointer });
        emit(&self.app, BuddyEvent::Start { id, title: title_for(&event.text) });

        let app = self.app.clone();
        let explainer = Arc::clone(&self.explainer);
        let text = event.text;
        let task = async_runtime::spawn(async move {
            let result = explainer
                .explain(&text, |piece| emit(&app, BuddyEvent::Chunk { id, text: piece.to_owned() }))
                .await;
            let end = match result {
                Ok(explained) => BuddyEvent::Done { id, cached: explained == Explained::Cached },
                Err(err) => {
                    log::warn!("[answer] {err}");
                    BuddyEvent::Error { id, message: err.to_string() }
                }
            };
            emit(&app, end);
        });
        *lock(&self.current) = Some(task);
    }

    /// Asks again for the last selection (the popup's Retry button).
    pub fn retry(&self) {
        let last = lock(&self.last_selection).clone();
        if let Some((event, pointer)) = last {
            self.on_selection(event, pointer);
        }
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

    /// Hides the popup and stops the running answer.
    pub fn hide(&self) {
        self.cancel_answer();
        let was_shown = lock(&self.popup).take().is_some();
        if was_shown {
            emit(&self.app, BuddyEvent::Hide);
            run_on_popup(&self.app, popup::hide);
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Places the popup for `id` and makes sure it shows, even if the page never reports its size.
    fn open_popup(&self, id: u64, anchor: PopupAnchor) {
        self.prepare_popup(id, anchor);
        let app = self.app.clone();
        thread::spawn(move || {
            thread::sleep(SHOW_FALLBACK);
            app.state::<Flow>().update_popup(id, None, false);
        });
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

    /// Places the popup for `id`. It is shown by `update_popup`.
    fn prepare_popup(&self, id: u64, anchor: PopupAnchor) {
        // Next to a selection: below it if possible. Next to the mascot (bottom-right corner
        // by default): above it.
        let (selection, pointer, prefer) = match anchor {
            PopupAnchor::Selection { rect, pointer } => (rect, pointer, Side::Below),
            PopupAnchor::Mascot => match mascot::bounds(&self.app) {
                Some(r) => (Some(r), Point::new((r.left + r.right) / 2, (r.top + r.bottom) / 2), Side::Above),
                None => (None, Point::new(0, 0), Side::Above),
            },
        };
        let monitor = self
            .app
            .monitor_from_point(f64::from(pointer.x), f64::from(pointer.y))
            .ok()
            .flatten()
            .or_else(|| self.app.primary_monitor().ok().flatten());
        let Some(monitor) = monitor else {
            log::warn!("[popup] no monitor found");
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
        let (bounds, side) = placement::place(anchor, size, work_area, prefer);

        *lock(&self.popup) = Some(PopupState { id, visible: false, anchor, work_area, scale, side, bounds });
    }

    fn cancel_answer(&self) {
        if let Some(task) = lock(&self.current).take() {
            // Dropping the future also drops the HTTP request to Ollama.
            task.abort();
        }
    }
}

/// Which Ollama problems get a notice at startup; others are only logged.
fn ollama_notice(err: &ProviderError) -> Option<NoticeKind> {
    match err {
        ProviderError::NotRunning => Some(NoticeKind::OllamaNotRunning),
        ProviderError::ModelMissing(_) => Some(NoticeKind::ModelMissing),
        _ => None,
    }
}

// --- Commands for the popup window ---

fn only_popup(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == POPUP { Ok(()) } else { Err("not allowed from this window".to_owned()) }
}

/// Called by the popup after rendering selection `id`, with the height its content needs
/// (CSS pixels) and whether the answer is complete.
#[tauri::command]
pub fn popup_resize(window: WebviewWindow, flow: State<'_, Flow>, id: u64, height: f64, complete: bool) {
    if only_popup(&window).is_ok() {
        flow.update_popup(id, Some(height), complete);
    }
}

/// The popup's × button.
#[tauri::command]
pub fn popup_close(window: WebviewWindow, flow: State<'_, Flow>) -> Result<(), String> {
    only_popup(&window)?;
    flow.hide();
    Ok(())
}

/// The popup's Retry button, after an error.
#[tauri::command]
pub fn popup_retry(window: WebviewWindow, flow: State<'_, Flow>) -> Result<(), String> {
    only_popup(&window)?;
    flow.retry();
    Ok(())
}

/// The popup's Copy button: puts the answer on the clipboard. The user asked for it, so unlike
/// the clipboard fallback it is meant to stay there (and in Win+V history).
#[tauri::command]
pub fn popup_copy(window: WebviewWindow, text: String) -> Result<(), String> {
    only_popup(&window)?;
    clipboard::set_text(&text).map_err(str::to_owned)
}

/// The "Open Settings" button on a notice.
#[tauri::command]
pub fn popup_open_settings(window: WebviewWindow, app: AppHandle, flow: State<'_, Flow>) -> Result<(), String> {
    only_popup(&window)?;
    flow.hide();
    app_windows::settings::open(&app).map_err(|e| e.to_string())
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
        log::warn!("[flow] could not emit {BUDDY_EVENT}: {err}");
    }
}

/// Window calls go through the main thread, where Tauri owns the popup window.
fn run_on_popup(app: &AppHandle, f: impl FnOnce(&WebviewWindow) -> popup::Result<()> + Send + 'static) {
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(POPUP)
            && let Err(err) = f(&window)
        {
            log::warn!("[popup] {err}");
        }
    });
    if let Err(err) = result {
        log::warn!("[popup] {err}");
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
    fn notice_event_shape() {
        let notice = notice::notice(NoticeKind::PasswordField, Context::default());
        let value = serde_json::to_value(BuddyEvent::Notice { id: 3, notice }).unwrap();
        assert_eq!(value["kind"], "notice");
        assert_eq!(value["id"], 3);
        assert_eq!(value["notice"]["kind"], "password_field");
        assert_eq!(value["notice"]["action"], serde_json::Value::Null);
        assert!(value["notice"]["title"].is_string() && value["notice"]["detail"].is_string());
    }

    #[test]
    fn only_some_ollama_errors_get_a_startup_notice() {
        assert_eq!(ollama_notice(&ProviderError::NotRunning), Some(NoticeKind::OllamaNotRunning));
        assert_eq!(ollama_notice(&ProviderError::ModelMissing("m".into())), Some(NoticeKind::ModelMissing));
        assert_eq!(ollama_notice(&ProviderError::Timeout), None);
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
