//! Connects a selection to an answer: shows the popup next to the selection, streams the
//! explanation into it, and tells the mascot what is happening. Problems the user should know
//! about (an app that can't be read, a privacy skip, Ollama not running, ...) are shown in the
//! same popup as short notices (see `notice`).
//!
//! Flow: selection worker -> `on_selection` -> `buddy` events to the popup and mascot,
//! answer streamed by a Tauri async task. A new selection or a click outside the popup
//! cancels the running answer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use ::windows::Win32::System::SystemInformation::GetLocalTime;
use serde::Serialize;
use tauri::async_runtime::{self, JoinHandle};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::explainer::{self, Explained, Explainer};
use crate::notice::{self, Context, Notice, NoticeKind, NoticeThrottle};
use crate::provider::ProviderError;
use crate::provider::ollama::OllamaProvider;
use crate::selection::{Point, Rect, SelectionEvent, clipboard, prepare_text};
use crate::settings::{self, SettingsState};
use crate::windows::flight::{self, Flight};
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
/// The startup greeting bubble is narrower than an answer.
const BUBBLE_WIDTH: f64 = 260.0;
/// The popup is shown once it has rendered the new selection and reported its size, so it
/// never flashes the previous answer or jumps in size. This is the fallback if no report comes.
const SHOW_FALLBACK: Duration = Duration::from_millis(250);
/// While the mascot flies to the popup, the popup waits for it to land; this much longer than
/// the flight at most.
const LANDING_GRACE: Duration = Duration::from_millis(150);
/// Startup notices wait until the windows have loaded and listen for events, and, when the
/// greeting is on, a little longer, so they rarely cut it short.
const STARTUP_NOTICE_DELAY: Duration = Duration::from_secs(3);
const STARTUP_NOTICE_DELAY_AFTER_GREETING: Duration = Duration::from_secs(6);
/// The greeting bubble appears once the mascot has landed from its intro flight, after a breath.
const GREETING_DELAY: Duration = mascot::INTRO_TIME.saturating_add(Duration::from_millis(300));
/// How long the greeting waits for the model to write it, counted from the request.
const GREETING_WAIT: Duration = Duration::from_secs(6);
/// A new greeting every time: much livelier than the explanations.
const GREETING_TEMPERATURE: f64 = 0.9;
/// How far (physical pixels) the mascot window may be from the popup's edge for the popup to
/// show a pointer toward it.
const TAIL_REACH: i32 = 32;

/// What the windows are told. Serialized as `{ "kind": "start", ... }`; mirrored in
/// `src/shared/events.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuddyEvent {
    /// `clipboard_offer`: the app's name if the popup should offer "Wrong text? Use the clipboard
    /// in {app}" (see `clipboard_offer`). `look`: how the popup comes out of the mascot.
    Start { id: u64, title: String, clipboard_offer: Option<String>, look: PopupLook },
    Chunk { id: u64, text: String },
    Done { id: u64, cached: bool },
    Error { id: u64, message: String },
    /// A problem shown in the popup instead of an answer.
    Notice { id: u64, notice: Notice, look: PopupLook },
    /// The model is writing the startup greeting (the mascot thinks).
    GreetingPending,
    /// The startup greeting, shown as a speech bubble next to the mascot.
    Greeting { id: u64, text: String, look: PopupLook },
    /// The popup window has just appeared: its bloom animation starts now.
    Shown { id: u64 },
    CantRead,
    Hide,
}

/// How the popup comes out of the mascot: it scales up from the mascot's direction
/// (`origin_*`, fractions of the popup, used as the CSS transform-origin), and when the mascot is
/// right next to it, a speech-balloon pointer on that `tail` side, `tail_offset` CSS pixels along
/// that edge (from the top for left/right, from the left for top/bottom).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PopupLook {
    pub origin_x: f64,
    pub origin_y: f64,
    pub tail: Option<TailSide>,
    pub tail_offset: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TailSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// The look of a popup at `popup` for a mascot window at `mascot` (both physical pixels).
pub fn popup_look(popup: Rect, mascot: Option<Rect>, scale: f64) -> PopupLook {
    let Some(m) = mascot.filter(|_| popup.width() > 0 && popup.height() > 0) else {
        return PopupLook { origin_x: 0.5, origin_y: 0.5, tail: None, tail_offset: 0.0 };
    };
    let center_x = f64::from(m.left + m.right) / 2.0;
    // The robot's head floats above its glow, in the upper part of its window: the pointer aims at
    // the middle of the head.
    let head_y = f64::from(m.top) + f64::from(m.height()) * 0.42;
    let fraction = |value: f64, start: i32, length: i32| ((value - f64::from(start)) / f64::from(length)).clamp(0.0, 1.0);
    let css = |physical: f64| physical / scale;

    let rows_overlap = m.top < popup.bottom && popup.top < m.bottom;
    let columns_overlap = m.left < popup.right && popup.left < m.right;
    let near = |gap: i32| (0..=TAIL_REACH).contains(&gap);
    let (tail, tail_offset) = if rows_overlap && near(m.left - popup.right) {
        (Some(TailSide::Right), css(head_y - f64::from(popup.top)))
    } else if rows_overlap && near(popup.left - m.right) {
        (Some(TailSide::Left), css(head_y - f64::from(popup.top)))
    } else if columns_overlap && near(m.top - popup.bottom) {
        (Some(TailSide::Bottom), css(center_x - f64::from(popup.left)))
    } else if columns_overlap && near(popup.top - m.bottom) {
        (Some(TailSide::Top), css(center_x - f64::from(popup.left)))
    } else {
        (None, 0.0)
    };
    PopupLook {
        origin_x: fraction(center_x, popup.left, popup.width()),
        origin_y: fraction(head_y, popup.top, popup.height()),
        tail,
        tail_offset,
    }
}

/// A different greeting mood each launch (an index into `notice::GREETING_MOODS`), taken from
/// the clock, so no random-number crate is needed.
fn launch_mood() -> usize {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as usize)
}

/// The local hour (0-23) and weekday (0 = Sunday), for the greeting.
///
/// GetLocalTime: https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getlocaltime
fn local_time() -> (u16, u16) {
    // SAFETY: plain query without pointers.
    let now = unsafe { GetLocalTime() };
    (now.wHour, now.wDayOfWeek)
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
    /// The mascot is flying to the popup: it appears when the mascot lands.
    wait_for_mascot: bool,
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

    /// Checks at startup that Ollama runs and has the model, after the windows have loaded. If
    /// not, a notice next to the mascot says what to do, so the first selection doesn't fail
    /// without warning. (With the greeting on, `greet` runs this check once the bubble is gone.)
    pub fn check_ollama(&self) {
        let app = self.app.clone();
        let spawned = thread::Builder::new().name("ollama-check".into()).spawn(move || {
            thread::sleep(STARTUP_NOTICE_DELAY);
            app.state::<Flow>().check_ollama_now();
        });
        if let Err(err) = spawned {
            log::warn!("[ollama] startup check not run: {err}");
        }
    }

    fn check_ollama_now(&self) {
        match async_runtime::block_on(self.explainer.health()) {
            Ok(()) => log::info!("[ollama] ready"),
            Err(err) => {
                log::warn!("[ollama] {err}");
                if let Some(kind) = ollama_notice(&err) {
                    let model = self.explainer.model();
                    let cx = Context { model: Some(&model), at_startup: true, ..Context::default() };
                    self.notify(kind, cx, PopupAnchor::Mascot);
                }
            }
        }
    }

    /// A problem at startup (the mouse hook or UI Automation didn't start): shown next to the
    /// mascot once the windows are ready.
    pub fn notify_startup(&self, kind: NoticeKind, reason: String) {
        log::error!("[startup] {kind:?}: {reason}");
        let app = self.app.clone();
        let delay = self.startup_notice_delay();
        let _ = thread::Builder::new().name("startup-notice".into()).spawn(move || {
            thread::sleep(delay);
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
        // Notices are shown with the mascot at home, where it reacts to them.
        self.app.state::<Flight>().return_home();
        let id = self.next_id();
        self.open_popup(id, anchor, POPUP_WIDTH);
        let look = self.look(mascot::bounds(&self.app));
        emit(&self.app, BuddyEvent::Notice { id, notice: notice::notice(kind, cx), look });
    }

    /// The startup greeting: the local model writes a new one each time, with the user's name,
    /// shown as a speech bubble next to the mascot once it has landed from its intro flight.
    /// If Ollama isn't running or the model is missing, one of the saved greetings is shown
    /// instead, without waiting; also if the model can't answer within `GREETING_WAIT` (a cold
    /// start). The request still loads the model, so the first real answer is fast. Only the name
    /// and the time of day are sent, to the local model. The popup never takes focus.
    /// Then the Ollama startup check runs, so its notice never cuts the greeting short.
    pub fn greet(&self) {
        let app = self.app.clone();
        let _ = thread::Builder::new().name("greeting".into()).spawn(move || {
            let show_at = Instant::now() + GREETING_DELAY;
            let flow = app.state::<Flow>();
            // The name from the settings, or else the Windows account's first name.
            let name = settings::greeting_name(&app.state::<SettingsState>().get());
            let (hour, weekday) = local_time();
            let roll = launch_mood();

            // Ask the model right away, so it loads while the mascot flies in. Checked first:
            // without Ollama or the model there is nothing to wait for.
            let asked = match async_runtime::block_on(flow.explainer.health()) {
                Ok(()) => {
                    let (system, user) = notice::greeting_prompt(&name, hour, weekday, roll);
                    let (tx, rx) = mpsc::channel();
                    let explainer = Arc::clone(&flow.explainer);
                    async_runtime::spawn(async move {
                        let _ = tx.send(explainer.ask(&system, &user, GREETING_TEMPERATURE).await);
                    });
                    Some((rx, Instant::now() + GREETING_WAIT))
                }
                Err(err) => {
                    log::info!("[greeting] using a saved greeting: {err}");
                    None
                }
            };
            thread::sleep(show_at.saturating_duration_since(Instant::now()));

            let written = asked.and_then(|(rx, deadline)| {
                // Still writing once the mascot has landed: it thinks meanwhile.
                let reply = rx.try_recv().or_else(|_| {
                    emit(&app, BuddyEvent::GreetingPending);
                    rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).map_err(|_| ())
                });
                match reply {
                    Ok(Ok(text)) => notice::clean_greeting(&text).map(|g| notice::ensure_name(g, &name)),
                    Ok(Err(err)) => {
                        log::info!("[greeting] the model couldn't write one: {err}");
                        None
                    }
                    Err(()) => {
                        log::info!("[greeting] the model is still loading; using a saved greeting");
                        None
                    }
                }
            });
            let text = written.unwrap_or_else(|| notice::saved_greeting(roll, &name));

            // A selection made meanwhile wins; no bubble then.
            if !flow.popup_visible() {
                let id = flow.next_id();
                flow.open_popup(id, PopupAnchor::Mascot, BUBBLE_WIDTH);
                let look = flow.look(mascot::bounds(&app));
                let shown_for = notice::greeting_duration(text.chars().count());
                emit(&app, BuddyEvent::Greeting { id, text, look });
                thread::sleep(shown_for);
                flow.hide_if(id);
            }
            flow.check_ollama_now();
        });
    }

    /// The mascot landed next to the popup: the popup appears now, as if out of the mascot.
    pub fn mascot_landed(&self) {
        self.stop_waiting_for_mascot(None);
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
        self.open_popup(id, PopupAnchor::Selection { rect: event.rect, pointer }, POPUP_WIDTH);
        let settings = self.app.state::<SettingsState>().get();
        // The mascot flies beside the popup, and the popup waits to come out of it on landing.
        let flight = if settings.fly_to_selection { self.fly_mascot_beside_popup() } else { None };
        if let Some((_, Some(duration))) = flight {
            self.wait_for_mascot(id, duration);
        }
        let mascot_at = flight.map(|(rect, _)| rect).or_else(|| mascot::bounds(&self.app));
        let look = self.look(mascot_at);
        let offer = clipboard_offer(
            &event,
            settings.clipboard_fallback,
            clipboard::policy().cloud_sync,
            settings.reads_clipboard_first(&event.process_name),
        );
        emit(&self.app, BuddyEvent::Start { id, title: title_for(&event.text), clipboard_offer: offer, look });

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

    /// The popup's "Use the clipboard in {app}" link: puts the app on the clipboard-first list,
    /// then reads the same selection again through the clipboard and explains it. The source app
    /// is still in front and its selection still highlighted: the popup never takes focus.
    /// Runs on its own thread, as the copy can take a few hundred milliseconds.
    pub fn use_clipboard_for_last_app(&self) {
        let Some((event, pointer)) = lock(&self.last_selection).clone() else { return };
        let app = self.app.clone();
        let _ = thread::Builder::new().name("clipboard-first".into()).spawn(move || {
            let flow = app.state::<Flow>();
            let exe = event.process_name.clone();
            app.state::<SettingsState>().update(&app, |s| s.clipboard_first_apps.push(exe.clone()));
            log::info!("[selection] {exe} now reads through the clipboard first");

            let copied = match event.window_pid {
                Some(pid) => clipboard::copy_selection(pid, clipboard::copy_keys(&exe)),
                None => Err("the app's window wasn't found"),
            };
            let text = copied.and_then(|text| prepare_text(&text).ok_or("it copied no text"));
            match text {
                Ok(text) => flow.on_selection(SelectionEvent { text, via_clipboard: true, ..event }, pointer),
                Err(why) => {
                    let cx = Context { app: Some(&event.app_name), reason: Some(why), ..Context::default() };
                    flow.notify(NoticeKind::NothingSelected, cx, PopupAnchor::Selection { rect: event.rect, pointer });
                }
            }
        });
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

    /// Hides the popup and stops the running answer. The mascot flies home if it was away.
    pub fn hide(&self) {
        self.cancel_answer();
        self.app.state::<Flight>().return_home();
        let was_shown = lock(&self.popup).take().is_some();
        if was_shown {
            emit(&self.app, BuddyEvent::Hide);
            run_on_popup(&self.app, popup::hide);
        }
    }

    /// Flies the mascot next to the popup just placed, if there is room beside it. Returns where
    /// the mascot will be, and how long the flight takes (`None` if it moves in one step).
    fn fly_mascot_beside_popup(&self) -> Option<(Rect, Option<Duration>)> {
        let popup = (*lock(&self.popup))?;
        let mascot = mascot::bounds(&self.app)?;
        let size = Size { width: mascot.width(), height: mascot.height() };
        let target = flight::dock_position(popup.bounds, popup.side, size, popup.work_area)?;
        let duration = self.app.state::<Flight>().fly_to(target);
        let rect = Rect { left: target.x, top: target.y, right: target.x + size.width, bottom: target.y + size.height };
        Some((rect, duration))
    }

    /// How the current popup comes out of a mascot at `mascot`.
    fn look(&self, mascot: Option<Rect>) -> PopupLook {
        match *lock(&self.popup) {
            Some(popup) => popup_look(popup.bounds, mascot, popup.scale),
            None => popup_look(Rect { left: 0, top: 0, right: 0, bottom: 0 }, None, 1.0),
        }
    }

    /// The popup of `id` appears when the mascot lands, at the latest `duration` + `LANDING_GRACE`.
    fn wait_for_mascot(&self, id: u64, duration: Duration) {
        if let Some(state) = lock(&self.popup).as_mut().filter(|s| s.id == id) {
            state.wait_for_mascot = true;
        }
        let app = self.app.clone();
        thread::spawn(move || {
            thread::sleep(duration + LANDING_GRACE);
            app.state::<Flow>().stop_waiting_for_mascot(Some(id));
        });
    }

    /// Shows the popup that waited for the mascot (`id`: only if it is still that popup).
    fn stop_waiting_for_mascot(&self, id: Option<u64>) {
        let waiting = {
            let mut guard = lock(&self.popup);
            match guard.as_mut() {
                Some(state) if state.wait_for_mascot && id.is_none_or(|id| id == state.id) => {
                    state.wait_for_mascot = false;
                    Some(state.id)
                }
                _ => None,
            }
        };
        if let Some(id) = waiting {
            self.update_popup(id, None, false);
        }
    }

    /// Hides the popup if it is still `id` (the greeting, after its few seconds).
    fn hide_if(&self, id: u64) {
        let current = lock(&self.popup).is_some_and(|s| s.id == id);
        if current {
            self.hide();
        }
    }

    fn startup_notice_delay(&self) -> Duration {
        let greeting = self.app.state::<SettingsState>().get().greeting;
        if greeting { STARTUP_NOTICE_DELAY_AFTER_GREETING } else { STARTUP_NOTICE_DELAY }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Places the popup for `id` and makes sure it shows, even if the page never reports its size.
    /// `width` in CSS pixels.
    fn open_popup(&self, id: u64, anchor: PopupAnchor, width: f64) {
        self.prepare_popup(id, anchor, width);
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
            // A popup that waits for the mascot to land is placed but not shown yet.
            let show = !state.visible && !state.wait_for_mascot;
            if !show && !resized {
                return;
            }
            state.visible |= show;
            (state.bounds, show)
        };
        let app = self.app.clone();
        run_on_popup(&self.app, move |window| {
            if show {
                popup::show_at(window, bounds)?;
                emit(&app, BuddyEvent::Shown { id });
                Ok(())
            } else {
                popup::set_bounds(window, bounds)
            }
        });
    }

    /// Places the popup for `id`, `width` CSS pixels wide. It is shown by `update_popup`.
    fn prepare_popup(&self, id: u64, anchor: PopupAnchor, width: f64) {
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
            width: (width * scale).round() as i32,
            height: (POPUP_START_HEIGHT * scale).round() as i32,
        };
        let anchor = placement::anchor(selection, pointer, work_area);
        let (bounds, side) = placement::place(anchor, size, work_area, prefer);

        *lock(&self.popup) =
            Some(PopupState { id, visible: false, wait_for_mascot: false, anchor, work_area, scale, side, bounds });
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

/// The popup's "Wrong text? Use the clipboard in {app}" link.
#[tauri::command]
pub fn popup_use_clipboard(window: WebviewWindow, flow: State<'_, Flow>) -> Result<(), String> {
    only_popup(&window)?;
    flow.use_clipboard_for_last_app();
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

/// Whether the popup offers "Wrong text? Use the clipboard in {app}": only for text read through
/// UI Automation, with the clipboard fallback on and usable (no cloud sync), in an app that
/// isn't on the clipboard-first list yet. Returns the app's name for the link.
fn clipboard_offer(event: &SelectionEvent, fallback_on: bool, cloud_sync: bool, already_listed: bool) -> Option<String> {
    let offer = !event.via_clipboard && fallback_on && !cloud_sync && !already_listed && event.window_pid.is_some();
    offer.then(|| event.app_name.clone())
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
            (
                BuddyEvent::Start { id: 1, title: "word".into(), clipboard_offer: None, look: CENTER },
                json!({"kind": "start", "id": 1, "title": "word", "clipboard_offer": null, "look": center_json()}),
            ),
            (
                BuddyEvent::Start { id: 1, title: "word".into(), clipboard_offer: Some("Notepad".into()), look: CENTER },
                json!({"kind": "start", "id": 1, "title": "word", "clipboard_offer": "Notepad", "look": center_json()}),
            ),
            (
                BuddyEvent::Greeting { id: 4, text: "Hi".into(), look: CENTER },
                json!({"kind": "greeting", "id": 4, "text": "Hi", "look": center_json()}),
            ),
            (BuddyEvent::Shown { id: 4 }, json!({"kind": "shown", "id": 4})),
            (BuddyEvent::GreetingPending, json!({"kind": "greeting_pending"})),
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
        let value = serde_json::to_value(BuddyEvent::Notice { id: 3, notice, look: CENTER }).unwrap();
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

    const CENTER: PopupLook = PopupLook { origin_x: 0.5, origin_y: 0.5, tail: None, tail_offset: 0.0 };
    fn center_json() -> serde_json::Value {
        json!({"origin_x": 0.5, "origin_y": 0.5, "tail": null, "tail_offset": 0.0})
    }

    /// An answer popup (360x200 at 100%) and the mascot window (160x150).
    const POPUP: Rect = Rect { left: 300, top: 430, right: 660, bottom: 630 };

    fn mascot_at(left: i32, top: i32) -> Option<Rect> {
        Some(Rect { left, top, right: left + 160, bottom: top + 150 })
    }

    #[test]
    fn mascot_docked_right_gets_a_pointer_on_the_right() {
        let look = popup_look(POPUP, mascot_at(668, 430), 1.0);
        assert_eq!(look.tail, Some(TailSide::Right));
        assert_eq!(look.origin_x, 1.0);
        assert!((look.tail_offset - 63.0).abs() < 0.01); // the robot's head
    }

    #[test]
    fn mascot_docked_left_gets_a_pointer_on_the_left() {
        let look = popup_look(POPUP, mascot_at(132, 430), 1.0);
        assert_eq!(look.tail, Some(TailSide::Left));
        assert_eq!(look.origin_x, 0.0);
    }

    #[test]
    fn greeting_above_the_mascot_points_down() {
        // The bubble is placed 8 px above the mascot.
        let bubble = Rect { left: 1500, top: 700, right: 1760, bottom: 832 };
        let look = popup_look(bubble, mascot_at(1600, 840), 1.0);
        assert_eq!(look.tail, Some(TailSide::Bottom));
        assert_eq!(look.origin_y, 1.0);
        assert!((look.tail_offset - 180.0).abs() < 0.01);
    }

    #[test]
    fn mascot_far_away_scales_from_its_direction_without_a_pointer() {
        let look = popup_look(POPUP, mascot_at(1700, 900), 1.0);
        assert_eq!(look.tail, None);
        assert_eq!((look.origin_x, look.origin_y), (1.0, 1.0));
        assert_eq!(popup_look(POPUP, None, 1.0).tail, None);
    }

    #[test]
    fn pointer_offset_is_in_css_pixels() {
        let look = popup_look(POPUP, mascot_at(668, 430), 1.5);
        assert!((look.tail_offset - 63.0 / 1.5).abs() < 0.01);
    }

    fn event(via_clipboard: bool) -> SelectionEvent {
        SelectionEvent {
            text: "word".into(),
            app_name: "Notepad".into(),
            process_name: "notepad.exe".into(),
            rect: None,
            window_pid: Some(4),
            via_clipboard,
        }
    }

    #[test]
    fn clipboard_is_offered_for_uia_text_when_it_can_be_used() {
        assert_eq!(clipboard_offer(&event(false), true, false, false), Some("Notepad".into()));
    }

    #[test]
    fn clipboard_is_not_offered_otherwise() {
        // Already read through the clipboard.
        assert_eq!(clipboard_offer(&event(true), true, false, false), None);
        // Fallback off, cloud sync on, or the app already listed.
        assert_eq!(clipboard_offer(&event(false), false, false, false), None);
        assert_eq!(clipboard_offer(&event(false), true, true, false), None);
        assert_eq!(clipboard_offer(&event(false), true, false, true), None);
        // No window to copy from.
        let no_window = SelectionEvent { window_pid: None, ..event(false) };
        assert_eq!(clipboard_offer(&no_window, true, false, false), None);
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
