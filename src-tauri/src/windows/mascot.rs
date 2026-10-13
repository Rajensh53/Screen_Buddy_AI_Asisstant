//! The mascot window: placed where the user left it (saved in the settings), or in the
//! bottom-right corner of the primary monitor's work area.

use std::sync::Mutex;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};

use crate::selection::{Point, Rect};
use crate::settings::{Position, SettingsState};

/// Gap between the mascot and the work-area edge, in physical pixels.
const MARGIN: i32 = 24;
/// The position is saved once the mascot has stopped moving for this long.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// The last position set by the app itself. Moves to it are not the user's, so they are not saved.
static PLACED_AT: Mutex<Option<PhysicalPosition<i32>>> = Mutex::new(None);

/// Places the mascot at `saved` if that is still on a connected monitor, otherwise in the
/// corner, then shows it and starts saving where the user drags it.
pub fn setup(app: &AppHandle, saved: Option<Position>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(super::MASCOT) else {
        return Ok(());
    };

    let size = window.outer_size()?;
    let monitors: Vec<Rect> = window
        .available_monitors()?
        .iter()
        .map(|m| rect(*m.position(), *m.size()))
        .collect();
    match saved.filter(|pos| position_on_screen(*pos, size, &monitors)) {
        Some(pos) => place(&window, PhysicalPosition::new(pos.x, pos.y))?,
        None => place_in_corner(&window)?,
    }
    save_when_moved(app, &window);
    window.show()
}

/// Where the mascot is on screen, for placing startup notices next to it.
pub fn bounds(app: &AppHandle) -> Option<Rect> {
    let window = app.get_webview_window(super::MASCOT)?;
    Some(rect(window.outer_position().ok()?, window.outer_size().ok()?))
}

/// Puts the mascot back in the corner ("Reset mascot position" in the settings).
pub fn move_to_corner(app: &AppHandle) -> tauri::Result<()> {
    match app.get_webview_window(super::MASCOT) {
        Some(window) => place_in_corner(&window),
        None => Ok(()),
    }
}

fn place_in_corner(window: &WebviewWindow) -> tauri::Result<()> {
    if let Some(monitor) = window.primary_monitor()? {
        let area = monitor.work_area();
        let size = window.outer_size()?;
        let x = area.position.x + area.size.width as i32 - size.width as i32 - MARGIN;
        let y = area.position.y + area.size.height as i32 - size.height as i32 - MARGIN;
        place(window, PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

fn place(window: &WebviewWindow, pos: PhysicalPosition<i32>) -> tauri::Result<()> {
    *PLACED_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(pos);
    window.set_position(pos)
}

/// Saves the mascot position after the user drags it. Dragging sends many `Moved` events, so a
/// thread waits until the mascot has been still for `SAVE_DELAY` and saves only the last one.
fn save_when_moved(app: &AppHandle, window: &WebviewWindow) {
    let (tx, rx) = mpsc::channel::<PhysicalPosition<i32>>();
    let app = app.clone();
    let spawned = thread::Builder::new().name("mascot-position".into()).spawn(move || {
        while let Ok(mut pos) = rx.recv() {
            loop {
                match rx.recv_timeout(SAVE_DELAY) {
                    Ok(newer) => pos = newer,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            app.state::<SettingsState>().update(&app, |s| s.mascot_position = Some(Position { x: pos.x, y: pos.y }));
        }
    });
    if let Err(err) = spawned {
        log::warn!("[mascot] position will not be saved: {err}");
        return;
    }
    window.on_window_event(move |event| {
        if let WindowEvent::Moved(pos) = event
            && *PLACED_AT.lock().unwrap_or_else(|e| e.into_inner()) != Some(*pos)
        {
            let _ = tx.send(*pos);
        }
    });
}

fn rect(pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> Rect {
    Rect { left: pos.x, top: pos.y, right: pos.x + size.width as i32, bottom: pos.y + size.height as i32 }
}

/// Whether a mascot of `size` at `pos` has its center on one of the `monitors`, so a position
/// saved on a monitor that is now unplugged is not used.
fn position_on_screen(pos: Position, size: PhysicalSize<u32>, monitors: &[Rect]) -> bool {
    let center = Point::new(pos.x + size.width as i32 / 2, pos.y + size.height as i32 / 2);
    monitors.iter().any(|m| m.contains(center))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: PhysicalSize<u32> = PhysicalSize { width: 140, height: 140 };
    const PRIMARY: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };
    const LEFT: Rect = Rect { left: -1920, top: 0, right: 0, bottom: 1080 };

    fn at(x: i32, y: i32) -> Position {
        Position { x, y }
    }

    #[test]
    fn position_on_the_primary_monitor() {
        assert!(position_on_screen(at(1700, 900), SIZE, &[PRIMARY]));
    }

    #[test]
    fn position_on_a_second_monitor_left_of_the_primary() {
        assert!(position_on_screen(at(-500, 300), SIZE, &[PRIMARY, LEFT]));
    }

    #[test]
    fn position_on_an_unplugged_monitor_is_not_used() {
        assert!(!position_on_screen(at(-500, 300), SIZE, &[PRIMARY]));
    }

    #[test]
    fn mostly_off_screen_is_not_used() {
        // Only a corner of the mascot would be visible.
        assert!(!position_on_screen(at(1850, 1000), SIZE, &[PRIMARY]));
        assert!(position_on_screen(at(1800, 960), SIZE, &[PRIMARY]));
    }
}
