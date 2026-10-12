//! Low-level mouse hook (`WH_MOUSE_LL`) on its own thread with a message loop.
//!
//! The hook procedure only forwards left-button down/up events to a channel and returns.
//! Windows silently removes a low-level hook that takes too long
//! (`LowLevelHooksTimeout`), so all real work happens on other threads.
//!
//! Mouse only: this app never installs a keyboard hook.
//!
//! LowLevelMouseProc: https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelmouseproc
//! SetWindowsHookExW: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowshookexw

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Sender, sync_channel};
use std::thread;

use ::windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_QUIT,
};

use super::trigger::{MouseInput, Point};

/// The hook procedure is a plain function, so it reaches the channel through a static.
static SENDER: OnceLock<Sender<MouseInput>> = OnceLock::new();
/// Thread id of the hook thread, used by `stop` to end its message loop. 0 = not running.
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// Installs the hook on a new thread. Left-button events are sent to `tx`.
/// Returns once the hook is installed, or with the error if it could not be.
pub fn start(tx: Sender<MouseInput>) -> Result<(), String> {
    SENDER.set(tx).map_err(|_| "mouse hook already started".to_owned())?;

    let (ready_tx, ready_rx) = sync_channel::<Result<(), String>>(1);
    thread::Builder::new()
        .name("mouse-hook".into())
        .spawn(move || run_hook_thread(ready_tx))
        .map_err(|e| format!("could not start mouse hook thread: {e}"))?;

    ready_rx.recv().map_err(|_| "mouse hook thread ended unexpectedly".to_owned())?
}

/// Ends the hook thread's message loop, which removes the hook.
pub fn stop() {
    let thread_id = HOOK_THREAD_ID.swap(0, Ordering::SeqCst);
    if thread_id != 0 {
        // SAFETY: posting WM_QUIT to a thread id we recorded; fails harmlessly if it is gone.
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

fn run_hook_thread(ready: std::sync::mpsc::SyncSender<Result<(), String>>) {
    // SAFETY: plain Win32 calls; the hook procedure is a valid `extern "system"` fn
    // that lives for the whole program.
    unsafe {
        let module = match GetModuleHandleW(None) {
            Ok(m) => m,
            Err(e) => {
                let _ = ready.send(Err(format!("GetModuleHandleW failed: {e}")));
                return;
            }
        };
        let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), Some(HINSTANCE(module.0)), 0) {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(format!("SetWindowsHookExW failed: {e}")));
                return;
            }
        };
        HOOK_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
        let _ = ready.send(Ok(()));

        // Low-level hooks are called through this thread's message loop.
        // GetMessageW returns 0 on WM_QUIT and -1 on error.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}

        let _ = UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && let Some(tx) = SENDER.get()
    {
        // SAFETY: for HC_ACTION, lParam points to an MSLLHOOKSTRUCT (see LowLevelMouseProc docs).
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let pt = Point::new(info.pt.x, info.pt.y);
        let input = match wparam.0 as u32 {
            WM_LBUTTONDOWN => Some(MouseInput::LeftDown { pt, time_ms: info.time }),
            WM_LBUTTONUP => Some(MouseInput::LeftUp { pt, time_ms: info.time }),
            _ => None,
        };
        if let Some(input) = input {
            // Unbounded channel: never blocks. An error only means the receiver is gone.
            let _ = tx.send(input);
        }
    }
    // SAFETY: passing the hook arguments through unchanged, as the docs require.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
