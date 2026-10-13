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
//! All popup visibility and sizing therefore goes through this module.
//! Also avoid Tauri setters on the popup (size, position, always-on-top, ...) while it is
//! visible: those re-apply Tauri's "hidden" state.

use ::windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SET_WINDOW_POS_FLAGS, SW_HIDE, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowPos,
    ShowWindow,
};
use tauri::WebviewWindow;

use crate::selection::Rect;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Moves and sizes the popup to `rect` (physical pixels), puts it on top of other topmost
/// windows and shows it, all without activating it.
///
/// SetWindowPos with SWP_NOACTIVATE | SWP_SHOWWINDOW:
/// https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos
pub fn show_at(window: &WebviewWindow, rect: Rect) -> Result<()> {
    set_window_pos(window, rect, SWP_NOACTIVATE | SWP_SHOWWINDOW)
}

/// Moves and sizes the visible popup without activating it or changing its z-order.
pub fn set_bounds(window: &WebviewWindow, rect: Rect) -> Result<()> {
    set_window_pos(window, rect, SWP_NOACTIVATE | SWP_NOZORDER)
}

fn set_window_pos(window: &WebviewWindow, rect: Rect, flags: SET_WINDOW_POS_FLAGS) -> Result<()> {
    let hwnd = window.hwnd()?;
    // SAFETY: `hwnd` is the live top-level window owned by this Tauri webview window.
    // HWND_TOPMOST is ignored when SWP_NOZORDER is set.
    unsafe {
        SetWindowPos(hwnd, Some(HWND_TOPMOST), rect.left, rect.top, rect.width(), rect.height(), flags)?;
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
