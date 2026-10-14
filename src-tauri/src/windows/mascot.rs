//! The mascot window. At every start it appears in the middle of the primary monitor's work area,
//! then flies to one of its corners: usually the bottom-right, never the same corner twice in a
//! row. Where the user drags it is its home for the rest of the session.

use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use super::flight::Flight;
use super::placement::Size;

use crate::selection::{Point, Rect};
use crate::settings::SettingsState;

/// Gap between the mascot and the work-area edge, in physical pixels.
const MARGIN: i32 = 24;
/// The mascot waits in the middle this long (it appears as a hologram, Mascot.tsx), then flies
/// to its corner.
const INTRO_DELAY: Duration = Duration::from_millis(1300);
/// The intro is the longest trip, so it is a little slower than the hops to a popup.
const INTRO_FLIGHT: Duration = Duration::from_millis(900);
/// When the mascot has landed in its corner, counted from `setup`.
pub const INTRO_TIME: Duration = INTRO_DELAY.saturating_add(INTRO_FLIGHT);

/// A corner of the work area. The last one is saved, so the next start picks another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Corner {
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

/// How often each corner is picked: the bottom-right is the usual spot.
const CORNER_WEIGHTS: [(Corner, u32); 4] =
    [(Corner::BottomRight, 3), (Corner::BottomLeft, 1), (Corner::TopRight, 1), (Corner::TopLeft, 1)];

/// The corner to fly to: any but `last`, by weight. `roll` is any number (a random one).
pub fn pick_corner(last: Option<Corner>, roll: u32) -> Corner {
    let choices = CORNER_WEIGHTS.iter().filter(|(corner, _)| Some(*corner) != last);
    let total: u32 = choices.clone().map(|(_, weight)| weight).sum();
    let mut left = roll % total;
    for &(corner, weight) in choices {
        if left < weight {
            return corner;
        }
        left -= weight;
    }
    Corner::BottomRight
}

/// The mascot window's top-left corner in `corner` of `work_area`, `MARGIN` from its edges.
pub fn corner_position(corner: Corner, mascot: Size, work_area: Rect) -> Point {
    let left = work_area.left + MARGIN;
    let right = work_area.right - mascot.width - MARGIN;
    let top = work_area.top + MARGIN;
    let bottom = work_area.bottom - mascot.height - MARGIN;
    match corner {
        Corner::BottomRight => Point::new(right, bottom),
        Corner::BottomLeft => Point::new(left, bottom),
        Corner::TopRight => Point::new(right, top),
        Corner::TopLeft => Point::new(left, top),
    }
}

/// The mascot window's top-left corner when it is centered in `work_area`.
fn center_position(mascot: Size, work_area: Rect) -> Point {
    Point::new(
        work_area.left + (work_area.width() - mascot.width) / 2,
        work_area.top + (work_area.height() - mascot.height) / 2,
    )
}

/// Shows the mascot in the middle of the screen, then flies it to a corner it didn't use last
/// time (saved in the settings).
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(super::MASCOT) else {
        return Ok(());
    };
    let Some((size, area)) = layout(&window)? else {
        return window.show();
    };
    let center = center_position(size, area);
    window.set_position(PhysicalPosition::new(center.x, center.y))?;
    window.show()?;

    let settings = app.state::<SettingsState>();
    let corner = pick_corner(settings.get().start_corner, random());
    settings.update(app, |s| s.start_corner = Some(corner));
    let target = corner_position(corner, size, area);
    log::debug!("[mascot] flying to the {corner:?} corner");

    let app = app.clone();
    let spawned = thread::Builder::new().name("mascot-intro".into()).spawn(move || {
        thread::sleep(INTRO_DELAY);
        app.state::<Flight>().relocate(target, INTRO_FLIGHT);
    });
    if let Err(err) = spawned {
        log::warn!("[mascot] intro flight not started: {err}");
        window.set_position(PhysicalPosition::new(target.x, target.y))?;
    }
    Ok(())
}

/// Where the mascot is on screen, for placing startup notices next to it.
pub fn bounds(app: &AppHandle) -> Option<Rect> {
    let window = app.get_webview_window(super::MASCOT)?;
    Some(rect(window.outer_position().ok()?, window.outer_size().ok()?))
}

/// Puts the mascot in the bottom-right corner ("Reset mascot position" in the settings).
pub fn move_to_corner(app: &AppHandle) -> tauri::Result<()> {
    // The corner is the new home, also if it was away next to a popup.
    if let Some(flight) = app.try_state::<Flight>() {
        flight.cancel();
    }
    let Some(window) = app.get_webview_window(super::MASCOT) else {
        return Ok(());
    };
    if let Some((size, area)) = layout(&window)? {
        let pos = corner_position(Corner::BottomRight, size, area);
        window.set_position(PhysicalPosition::new(pos.x, pos.y))?;
    }
    Ok(())
}

/// The mascot window's size and the primary monitor's work area, if there is a primary monitor.
fn layout(window: &WebviewWindow) -> tauri::Result<Option<(Size, Rect)>> {
    let Some(monitor) = window.primary_monitor()? else {
        return Ok(None);
    };
    let area = monitor.work_area();
    let size = window.outer_size()?;
    let mascot = Size { width: size.width as i32, height: size.height as i32 };
    Ok(Some((mascot, rect(area.position, area.size))))
}

/// The user pressed the mouse on the mascot (it may be about to drag it): any flight stops, and
/// wherever it is dropped becomes the new home.
#[tauri::command]
pub fn mascot_grabbed(window: WebviewWindow, flight: tauri::State<'_, Flight>) -> Result<(), String> {
    if window.label() != super::MASCOT {
        return Err("not allowed from this window".to_owned());
    }
    flight.cancel();
    Ok(())
}

/// A different number each start, from the clock, so no random-number crate is needed.
fn random() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos())
}

fn rect(pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> Rect {
    Rect { left: pos.x, top: pos.y, right: pos.x + size.width as i32, bottom: pos.y + size.height as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size { width: 140, height: 140 };
    const PRIMARY: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1032 };
    const ALL: [Corner; 4] = [Corner::BottomRight, Corner::BottomLeft, Corner::TopRight, Corner::TopLeft];

    #[test]
    fn never_the_same_corner_twice_in_a_row() {
        for last in ALL {
            for roll in 0..1000 {
                assert_ne!(pick_corner(Some(last), roll), last);
            }
        }
    }

    #[test]
    fn every_corner_can_be_picked() {
        let picked: std::collections::HashSet<Corner> = (0..6).map(|roll| pick_corner(None, roll)).collect();
        assert_eq!(picked.len(), 4);
        for last in ALL {
            let others: std::collections::HashSet<Corner> = (0..6).map(|roll| pick_corner(Some(last), roll)).collect();
            assert_eq!(others.len(), 3, "after {last:?}");
        }
    }

    #[test]
    fn bottom_right_is_the_usual_spot() {
        let count = |last, corner| (0..600).filter(|&roll| pick_corner(last, roll) == corner).count();
        assert_eq!(count(None, Corner::BottomRight), 300);
        assert_eq!(count(None, Corner::TopLeft), 100);
        assert_eq!(count(Some(Corner::TopLeft), Corner::BottomRight), 360);
    }

    #[test]
    fn corners_keep_the_margin() {
        assert_eq!(corner_position(Corner::BottomRight, SIZE, PRIMARY), Point::new(1920 - 140 - 24, 1032 - 140 - 24));
        assert_eq!(corner_position(Corner::TopLeft, SIZE, PRIMARY), Point::new(24, 24));
        assert_eq!(corner_position(Corner::TopRight, SIZE, PRIMARY), Point::new(1756, 24));
        assert_eq!(corner_position(Corner::BottomLeft, SIZE, PRIMARY), Point::new(24, 868));
    }

    #[test]
    fn corners_on_a_monitor_left_of_the_main_one() {
        let area = Rect { left: -1920, top: 0, right: 0, bottom: 1040 };
        for corner in ALL {
            let p = corner_position(corner, SIZE, area);
            assert!(p.x >= area.left && p.x + SIZE.width <= area.right, "{corner:?}");
            assert!(p.y >= area.top && p.y + SIZE.height <= area.bottom, "{corner:?}");
        }
    }

    #[test]
    fn starts_in_the_middle() {
        assert_eq!(center_position(SIZE, PRIMARY), Point::new(890, 446));
    }
}
