//! The mascot's flight: from its home spot to the popup when a selection is explained, and back
//! home when the popup closes. The robot's look in flight (bank, thruster, shadow) is CSS in the
//! mascot window, driven by `mascot-flight` events.
//!
//! The window moves on its own thread, one step per screen refresh (`DwmFlush`), along an eased
//! curve computed from the elapsed time, so the motion is smooth even if a frame comes late.
//! It never activates a window.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::Graphics::Dwm::DwmFlush;
use ::windows::Win32::UI::WindowsAndMessaging::{
    SET_WINDOW_POS_FLAGS, SPI_GETCLIENTAREAANIMATION, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowPos, SystemParametersInfoW,
};
use ::windows::core::BOOL;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

use super::MASCOT;
use super::mascot;
use super::placement::{Side, Size};
use crate::selection::{Point, Rect};

/// Event name for `FlightEvent`, listened to by the mascot window.
pub const MASCOT_FLIGHT: &str = "mascot-flight";

/// Space between the popup and the mascot window.
const GAP: i32 = 8;
const SHORTEST_FLIGHT: Duration = Duration::from_millis(400);
const LONGEST_FLIGHT: Duration = Duration::from_millis(600);
/// Flights this long (in physical pixels) or longer take `LONGEST_FLIGHT`.
const LONG_DISTANCE: f64 = 1500.0;

/// Set while the window is being moved by a flight, so the position saver ignores those moves
/// (`mascot::save_when_moved`): only the user's own drags change the home spot.
static ANIMATING: AtomicBool = AtomicBool::new(false);

pub fn is_animating() -> bool {
    ANIMATING.load(Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    None,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct FlightEvent {
    flying: bool,
    direction: Direction,
}

/// Slow start, fast middle, slow end. `t` from 0 to 1.
pub fn ease_in_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Short hops are quick; long flights take a little longer, so the speed stays believable.
pub fn flight_duration(distance: f64) -> Duration {
    let share = (distance / LONG_DISTANCE).clamp(0.0, 1.0);
    SHORTEST_FLIGHT + (LONGEST_FLIGHT - SHORTEST_FLIGHT).mul_f64(share)
}

/// Which way the robot leans while flying from `from` to `to`.
pub fn bank(from: Point, to: Point) -> Direction {
    const STRAIGHT: i32 = 4;
    match to.x - from.x {
        dx if dx > STRAIGHT => Direction::Right,
        dx if dx < -STRAIGHT => Direction::Left,
        _ => Direction::None,
    }
}

/// Where the mascot window docks next to the popup: beside it (right, or left if the right has
/// no room), lined up with the popup's edge next to the selection, so it never covers the
/// selected text. It never overlaps the popup: the mascot window's transparent corners would
/// catch the clicks meant for the popup's buttons. `None` if neither side has room.
pub fn dock_position(popup: Rect, side: Side, mascot: Size, work_area: Rect) -> Option<Point> {
    let y = match side {
        Side::Below => popup.top,
        Side::Above => popup.bottom - mascot.height,
    };
    let max_y = work_area.bottom - mascot.height;
    let y = if max_y < work_area.top { work_area.top } else { y.clamp(work_area.top, max_y) };

    let right = popup.right + GAP;
    if right + mascot.width <= work_area.right {
        return Some(Point::new(right, y));
    }
    let left = popup.left - GAP - mascot.width;
    (left >= work_area.left).then(|| Point::new(left, y))
}

/// Whether Windows animations are on (Settings > Accessibility > Visual effects > Animation
/// effects). If not, the mascot moves in one step instead of flying.
///
/// SystemParametersInfoW: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow
fn animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to `enabled`, which outlives the call.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_err() || enabled.as_bool()
}

/// The mascot's flights. Managed state; used by `Flow` and the mascot's `mascot_grabbed` command.
pub struct Flight {
    app: AppHandle,
    /// Where the mascot flew from: set when it leaves home, cleared when it is back (or when the
    /// user drags it, which makes the drop spot the new home).
    home: Mutex<Option<PhysicalPosition<i32>>>,
    /// Bumped by every flight and every cancel; a flight stops when it is no longer the latest.
    generation: AtomicU64,
}

impl Flight {
    pub fn new(app: AppHandle) -> Self {
        Self { app, home: Mutex::new(None), generation: AtomicU64::new(0) }
    }

    /// Flies to `target` (the window's top-left corner, physical pixels). Remembers home on the
    /// first flight away; a flight from one popup to the next keeps the same home.
    pub fn fly_to(&self, target: Point) {
        let Some(from) = self.position() else { return };
        lock(&self.home).get_or_insert(from);
        self.start(from, PhysicalPosition::new(target.x, target.y), false);
    }

    /// Flies back home, if away.
    pub fn return_home(&self) {
        let Some(home) = *lock(&self.home) else { return };
        let Some(from) = self.position() else { return };
        self.start(from, home, true);
    }

    /// The user grabbed the mascot: stop flying. Wherever it is dropped becomes the new home
    /// (saved by `mascot::save_when_moved`).
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        lock(&self.home).take();
        ANIMATING.store(false, Ordering::SeqCst);
        self.emit(false, Direction::None);
    }

    fn position(&self) -> Option<PhysicalPosition<i32>> {
        self.app.get_webview_window(MASCOT)?.outer_position().ok()
    }

    fn start(&self, from: PhysicalPosition<i32>, to: PhysicalPosition<i32>, returning: bool) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(hwnd) = self.app.get_webview_window(MASCOT).and_then(|w| w.hwnd().ok()) else { return };
        // HWND is not Send; the thread rebuilds it from the raw value.
        let hwnd_raw = hwnd.0 as isize;
        let app = self.app.clone();

        let spawned = thread::Builder::new().name("mascot-flight".into()).spawn(move || {
            let flight = app.state::<Flight>();
            let current = || flight.generation.load(Ordering::SeqCst) == generation;
            let hwnd = HWND(hwnd_raw as *mut _);
            ANIMATING.store(true, Ordering::SeqCst);

            if from != to && animations_enabled() {
                let (a, b) = (Point::new(from.x, from.y), Point::new(to.x, to.y));
                let distance = f64::from(b.x - a.x).hypot(f64::from(b.y - a.y));
                let duration = flight_duration(distance);
                flight.emit(true, bank(a, b));
                let start = Instant::now();
                loop {
                    if !current() {
                        return; // A newer flight or a drag took over.
                    }
                    let t = start.elapsed().as_secs_f64() / duration.as_secs_f64();
                    let k = ease_in_out_cubic(t);
                    let x = f64::from(a.x) + f64::from(b.x - a.x) * k;
                    let y = f64::from(a.y) + f64::from(b.y - a.y) * k;
                    move_window(hwnd, x.round() as i32, y.round() as i32);
                    if t >= 1.0 {
                        break;
                    }
                    wait_for_next_frame();
                }
            } else {
                move_window(hwnd, to.x, to.y);
            }

            if current() {
                // The window's last `Moved` event arrives after this: it must not count as a user move.
                mascot::placed_at(to);
                ANIMATING.store(false, Ordering::SeqCst);
                if returning {
                    lock(&flight.home).take();
                }
                flight.emit(false, Direction::None);
            }
        });
        if let Err(err) = spawned {
            log::warn!("[mascot] flight not started: {err}");
        }
    }

    fn emit(&self, flying: bool, direction: Direction) {
        if let Err(err) = self.app.emit_to(MASCOT, MASCOT_FLIGHT, FlightEvent { flying, direction }) {
            log::warn!("[mascot] could not emit {MASCOT_FLIGHT}: {err}");
        }
    }
}

/// Moves the mascot window without activating it, resizing it or changing its z-order. Asynchronous,
/// so a busy UI thread never holds the flight back.
///
/// SetWindowPos: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos
fn move_window(hwnd: HWND, x: i32, y: i32) {
    let flags: SET_WINDOW_POS_FLAGS = SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER | SWP_ASYNCWINDOWPOS;
    // SAFETY: `hwnd` is the mascot's window, alive for the app's lifetime; a failed call is harmless.
    let _ = unsafe { SetWindowPos(hwnd, None, x, y, 0, 0, flags) };
}

/// Waits for the next screen refresh, so each step lands on a new frame. Falls back to a short
/// sleep if the compositor can't be asked.
///
/// DwmFlush: https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmflush
fn wait_for_next_frame() {
    // SAFETY: plain call without arguments.
    if unsafe { DwmFlush() }.is_err() {
        thread::sleep(Duration::from_millis(8));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1032 };
    const MASCOT_SIZE: Size = Size { width: 140, height: 140 };

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    #[test]
    fn ease_starts_and_ends_still() {
        assert_eq!(ease_in_out_cubic(0.0), 0.0);
        assert_eq!(ease_in_out_cubic(1.0), 1.0);
        assert!((ease_in_out_cubic(0.5) - 0.5).abs() < 1e-9);
        // Out of range is clamped.
        assert_eq!(ease_in_out_cubic(-1.0), 0.0);
        assert_eq!(ease_in_out_cubic(2.0), 1.0);
    }

    #[test]
    fn ease_always_moves_forward() {
        let steps: Vec<f64> = (0..=100).map(|i| ease_in_out_cubic(f64::from(i) / 100.0)).collect();
        assert!(steps.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn flight_takes_400_to_600_ms() {
        assert_eq!(flight_duration(0.0), SHORTEST_FLIGHT);
        assert_eq!(flight_duration(10_000.0), LONGEST_FLIGHT);
        let mid = flight_duration(750.0);
        assert!(mid > SHORTEST_FLIGHT && mid < LONGEST_FLIGHT);
    }

    #[test]
    fn robot_leans_into_the_flight() {
        assert_eq!(bank(Point::new(1000, 900), Point::new(300, 200)), Direction::Left);
        assert_eq!(bank(Point::new(300, 900), Point::new(1000, 200)), Direction::Right);
        assert_eq!(bank(Point::new(500, 900), Point::new(502, 200)), Direction::None);
    }

    #[test]
    fn docks_right_of_a_popup_below_the_selection() {
        let popup = rect(300, 430, 660, 630);
        let dock = dock_position(popup, Side::Below, MASCOT_SIZE, WORK).unwrap();
        assert_eq!(dock, Point::new(668, 430));
    }

    #[test]
    fn docks_left_when_the_right_has_no_room() {
        let popup = rect(1500, 430, 1860, 630);
        let dock = dock_position(popup, Side::Below, MASCOT_SIZE, WORK).unwrap();
        assert_eq!(dock, Point::new(1500 - GAP - 140, 430));
    }

    #[test]
    fn lines_up_with_the_bottom_of_a_popup_above_the_selection() {
        let popup = rect(300, 600, 660, 900);
        let dock = dock_position(popup, Side::Above, MASCOT_SIZE, WORK).unwrap();
        assert_eq!(dock.y, 900 - 140);
    }

    #[test]
    fn no_dock_when_neither_side_has_room() {
        let narrow = rect(0, 0, 500, 1032);
        assert_eq!(dock_position(rect(70, 100, 430, 300), Side::Below, MASCOT_SIZE, narrow), None);
    }

    #[test]
    fn dock_stays_in_the_work_area_and_off_the_popup() {
        // A popup at the very bottom: the mascot is moved up to fit.
        let popup = rect(300, 950, 660, 1032);
        let dock = dock_position(popup, Side::Below, MASCOT_SIZE, WORK).unwrap();
        let mascot = rect(dock.x, dock.y, dock.x + 140, dock.y + 140);
        assert!(mascot.bottom <= WORK.bottom && mascot.top >= WORK.top);
        assert!(!mascot.intersects(popup));
    }
}
