//! The popup must never take focus, so the selection in the source app survives.
//!
//! Two parts make that work:
//! - `"focusable": false` in `tauri.conf.json`. Tauri then gives the window the
//!   `WS_EX_NOACTIVATE` extended style, so clicking it does not activate it.
//!   https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles
//! - Showing it through `show_at` below, never through Tauri's `show()`/`set_focus()`:
//!   Tauri's `show()` calls `ShowWindow(SW_SHOW)`, which activates the window.
//!
//! Tauri never learns that the popup is visible, so its `hide()` would do nothing.
//! All popup visibility therefore goes through this module (`show_at` / `hide`).
//! Also avoid other Tauri style setters on the popup (always-on-top, decorations,
//! resizable, ...) while it is visible: those re-apply Tauri's "hidden" state.

// Until the popup is wired to selections (Task 6), only the debug tray item uses this module.
#![cfg_attr(not(debug_assertions), allow(dead_code))]

use tauri::{PhysicalPosition, WebviewWindow};
use ::windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SW_HIDE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow,
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Moves the popup's top-left corner to `pos` (physical pixels), puts it on top of other
/// topmost windows and shows it, all without activating it.
///
/// SetWindowPos with SWP_NOACTIVATE | SWP_SHOWWINDOW:
/// https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos
pub fn show_at(window: &WebviewWindow, pos: PhysicalPosition<i32>) -> Result<()> {
    let hwnd = window.hwnd()?;
    // SAFETY: `hwnd` is the live top-level window owned by this Tauri webview window.
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            pos.x,
            pos.y,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_SHOWWINDOW,
        )?;
    }
    Ok(())
}

/// Hides the popup.
///
/// ShowWindow with SW_HIDE:
/// https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindow
pub fn hide(window: &WebviewWindow) -> Result<()> {
    let hwnd = window.hwnd()?;
    // SAFETY: `hwnd` is the live top-level window owned by this Tauri webview window.
    // The return value is the previous visibility, not an error.
    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    Ok(())
}
