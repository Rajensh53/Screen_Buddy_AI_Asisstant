//! Clipboard fallback: used when UI Automation can't give the selection (the app doesn't expose
//! it), or gives wrong text (the PDF viewer built into Edge/Chrome reads "Apprenticeship" as
//! "e Accidental A"; its own Copy is right). UI Automation is always tried first. On by default;
//! the user can turn it off in the settings (`clipboard_fallback`).
//!
//! Rules:
//! - Never when Windows cloud clipboard sync is on: the copied text would leave the PC.
//! - Terminals get their own copy keys (Ctrl+Insert or Ctrl+Shift+C, see `copy_keys`): there
//!   Ctrl+C would stop the running program. Never on controls that hold no selection, such as
//!   buttons or scroll bars (see `uia_reader`), or on a window's frame.
//! - The user's clipboard is saved first and put back afterwards.
//! - The copy keys are sent with `SendInput`. No keyboard hook is installed; `GetAsyncKeyState` only
//!   checks that no other key is held down.
//! - The copied text is never logged.
//!
//! Clipboard: https://learn.microsoft.com/en-us/windows/win32/dataxchg/using-the-clipboard

use std::thread;
use std::time::{Duration, Instant};

use ::windows::Win32::Foundation::{ERROR_SUCCESS, GlobalFree, HANDLE, HGLOBAL, HWND};
use ::windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use ::windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use ::windows::Win32::System::Ole::CF_UNICODETEXT;
use ::windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    KEYEVENTF_EXTENDEDKEY, VIRTUAL_KEY, VK_CONTROL, VK_INSERT, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetForegroundWindow, GetWindowThreadProcessId, HWND_MESSAGE, WINDOW_EX_STYLE,
    WINDOW_STYLE,
};
use ::windows::core::{HSTRING, w};
use serde::Serialize;

/// How long the app may take to put the copied text on the clipboard.
const COPY_TIMEOUT: Duration = Duration::from_millis(400);
/// The clipboard may be open in another app for a moment.
const OPEN_ATTEMPTS: u32 = 10;
/// Bigger clipboard contents (a large image, say) are not saved, and then nothing is copied,
/// so the user's clipboard is never lost.
const MAX_SAVED_BYTES: usize = 64 * 1024 * 1024;
const VK_C: VIRTUAL_KEY = VIRTUAL_KEY(0x43);

/// Windows clipboard features that matter for the fallback.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardPolicy {
    /// "Sync across your devices": copied text is uploaded. The fallback never runs then.
    pub cloud_sync: bool,
    /// Clipboard history (Win+V): copied text stays in the list on this PC. Settings warns.
    pub history: bool,
}

/// Why the fallback did not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    SwitchedOff,
    CloudSync,
}

/// Whether the fallback may copy: switched on in settings and no cloud sync.
pub fn may_copy(enabled: bool, policy: ClipboardPolicy) -> Result<(), Blocked> {
    if !enabled {
        Err(Blocked::SwitchedOff)
    } else if policy.cloud_sync {
        Err(Blocked::CloudSync)
    } else {
        Ok(())
    }
}

/// The keys that copy in an app. Ctrl+C everywhere except terminals, where Ctrl+C stops the
/// running program: there the terminal's own copy keys are used, which never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyKeys {
    CtrlC,
    /// Windows Terminal, the classic console (cmd, PowerShell, WSL), mintty and PuTTY.
    CtrlInsert,
    /// Alacritty, WezTerm, kitty, Tabby, Hyper and Warp.
    CtrlShiftC,
}

/// `process_name` is the exe name, any case.
pub fn copy_keys(process_name: &str) -> CopyKeys {
    const CTRL_INSERT: &[&str] = &[
        "windowsterminal.exe",
        "openconsole.exe",
        "conhost.exe",
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "wsl.exe",
        "wslhost.exe",
        "bash.exe",
        "mintty.exe",
        "putty.exe",
    ];
    const CTRL_SHIFT_C: &[&str] =
        &["alacritty.exe", "wezterm-gui.exe", "kitty.exe", "tabby.exe", "hyper.exe", "warp.exe"];
    let name = process_name.trim().to_lowercase();
    if CTRL_INSERT.contains(&name.as_str()) {
        CopyKeys::CtrlInsert
    } else if CTRL_SHIFT_C.contains(&name.as_str()) {
        CopyKeys::CtrlShiftC
    } else {
        CopyKeys::CtrlC
    }
}

/// Reads the clipboard settings from the registry (Settings > System > Clipboard), including
/// the group-policy overrides.
pub fn policy() -> ClipboardPolicy {
    const USER: &str = r"Software\Microsoft\Clipboard";
    const POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\System";
    policy_from(RegistryValues {
        history: read_dword(HKEY_CURRENT_USER, USER, "EnableClipboardHistory"),
        cloud: read_dword(HKEY_CURRENT_USER, USER, "EnableCloudClipboard"),
        history_allowed: read_dword(HKEY_LOCAL_MACHINE, POLICY, "AllowClipboardHistory"),
        cloud_allowed: read_dword(HKEY_LOCAL_MACHINE, POLICY, "AllowCrossDeviceClipboard"),
    })
}

struct RegistryValues {
    history: Option<u32>,
    cloud: Option<u32>,
    history_allowed: Option<u32>,
    cloud_allowed: Option<u32>,
}

/// A policy value of 0 turns a feature off for everyone. Otherwise the user's switch decides;
/// a switch that was never set is off (the Windows default).
fn policy_from(values: RegistryValues) -> ClipboardPolicy {
    let on = |user: Option<u32>, allowed: Option<u32>| allowed != Some(0) && user.is_some_and(|v| v != 0);
    ClipboardPolicy {
        cloud_sync: on(values.cloud, values.cloud_allowed),
        history: on(values.history, values.history_allowed),
    }
}

/// RegGetValueW: https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-reggetvaluew
fn read_dword(key: HKEY, subkey: &str, value: &str) -> Option<u32> {
    let (subkey, value) = (HSTRING::from(subkey), HSTRING::from(value));
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `data` and `size` outlive the call; RRF_RT_REG_DWORD limits the value to 4 bytes.
    let status = unsafe {
        RegGetValueW(
            key,
            &subkey,
            &value,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

/// Copies the selection of the app in front with its copy `keys`, reads the text, and restores
/// the user's clipboard. `expected_pid` owns the window the selection was made in. Errors are
/// short reasons shown to the user ("copying didn't work: {reason}").
pub fn copy_selection(expected_pid: u32, keys: CopyKeys) -> Result<String, &'static str> {
    if foreground_process() != expected_pid {
        return Err("the app with the selection wasn't in front");
    }
    if modifier_held() {
        return Err("a key was held down");
    }
    let owner = OwnerWindow::new()?;
    let saved = {
        let _open = OpenedClipboard::open(owner.0)?;
        save_formats()?
    };

    // GetClipboardSequenceNumber: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardsequencenumber
    // SAFETY: plain query.
    let before = unsafe { GetClipboardSequenceNumber() };
    send_copy(keys)?;
    if !wait_for_new_content(before) {
        // Nothing was copied, so the user's clipboard is untouched.
        return Err("the app didn't copy anything");
    }

    let text = {
        let _open = OpenedClipboard::open(owner.0)?;
        read_text()
    };
    if let Err(err) = restore(owner.0, &saved) {
        log::warn!("[clipboard] could not restore the previous clipboard: {err}");
    }
    text.filter(|t| !t.trim().is_empty()).ok_or("it copied no text")
}

/// Puts `text` on the clipboard: the popup's Copy button, which the user clicked.
pub fn set_text(text: &str) -> Result<(), &'static str> {
    let owner = OwnerWindow::new()?;
    let _open = OpenedClipboard::open(owner.0)?;
    // SAFETY: the clipboard is open with `owner`; EmptyClipboard makes it the owner.
    unsafe { EmptyClipboard() }.map_err(|_| "could not empty the clipboard")?;
    let units: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_ne_bytes).collect();
    if set_global(u32::from(CF_UNICODETEXT.0), &units) { Ok(()) } else { Err("could not copy") }
}

/// GetForegroundWindow: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getforegroundwindow
fn foreground_process() -> u32 {
    let mut pid = 0;
    // SAFETY: plain queries; `pid` outlives the call.
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid
}

/// GetAsyncKeyState: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate
fn modifier_held() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        // SAFETY: plain query. The high bit (a negative value) means "down".
        .any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0)
}

/// SendInput: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
fn send_copy(keys: CopyKeys) -> Result<(), &'static str> {
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                // Insert is an extended key; without the flag it would be Numpad 0.
                dwFlags: KEYBD_EVENT_FLAGS(
                    if up { KEYEVENTF_KEYUP.0 } else { 0 } | if vk == VK_INSERT { KEYEVENTF_EXTENDEDKEY.0 } else { 0 },
                ),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let (modifiers, main): (&[VIRTUAL_KEY], VIRTUAL_KEY) = match keys {
        CopyKeys::CtrlC => (&[VK_CONTROL], VK_C),
        CopyKeys::CtrlInsert => (&[VK_CONTROL], VK_INSERT),
        CopyKeys::CtrlShiftC => (&[VK_CONTROL, VK_SHIFT], VK_C),
    };
    // Modifiers down, the key down and up, modifiers up in reverse order.
    let inputs: Vec<INPUT> = modifiers
        .iter()
        .map(|&m| key(m, false))
        .chain([key(main, false), key(main, true)])
        .chain(modifiers.iter().rev().map(|&m| key(m, true)))
        .collect();
    // SAFETY: `inputs` is a valid array of keyboard INPUTs.
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() { Ok(()) } else { Err("the copy keys couldn't be sent") }
}

fn wait_for_new_content(before: u32) -> bool {
    let start = Instant::now();
    while start.elapsed() < COPY_TIMEOUT {
        // SAFETY: plain query.
        if unsafe { GetClipboardSequenceNumber() } != before {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

/// A message-only window that owns the clipboard while we restore it. Restoring needs an
/// owner: with none, `SetClipboardData` fails.
///
/// Message-only windows: https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features#message-only-windows
struct OwnerWindow(HWND);

impl OwnerWindow {
    fn new() -> Result<Self, &'static str> {
        // SAFETY: creates a hidden STATIC window under HWND_MESSAGE; destroyed in Drop.
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .map(Self)
        .map_err(|_| "the clipboard couldn't be opened")
    }
}

impl Drop for OwnerWindow {
    fn drop(&mut self) {
        // SAFETY: the window was created on this thread by `new`.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

/// The clipboard, open until dropped.
///
/// OpenClipboard: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard
struct OpenedClipboard;

impl OpenedClipboard {
    fn open(owner: HWND) -> Result<Self, &'static str> {
        for _ in 0..OPEN_ATTEMPTS {
            // SAFETY: `owner` is a live window of this thread.
            if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
                return Ok(Self);
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err("the clipboard was busy")
    }
}

impl Drop for OpenedClipboard {
    fn drop(&mut self) {
        // SAFETY: matches the successful OpenClipboard in `open`.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Formats whose data is a memory block that can be copied and put back. GDI handles
/// (bitmaps, metafiles, palettes) can't; Windows rebuilds CF_BITMAP from CF_DIB anyway.
///
/// Standard formats: https://learn.microsoft.com/en-us/windows/win32/dataxchg/standard-clipboard-formats
fn restorable(format: u32) -> bool {
    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_ENHMETAFILE: u32 = 14;
    const CF_OWNERDISPLAY: u32 = 0x80;
    const CF_DSPBITMAP: u32 = 0x82;
    const CF_DSPMETAFILEPICT: u32 = 0x83;
    const CF_DSPENHMETAFILE: u32 = 0x8E;
    const CF_GDIOBJFIRST: u32 = 0x300;
    const CF_GDIOBJLAST: u32 = 0x3FF;
    !matches!(
        format,
        CF_BITMAP
            | CF_METAFILEPICT
            | CF_PALETTE
            | CF_ENHMETAFILE
            | CF_OWNERDISPLAY
            | CF_DSPBITMAP
            | CF_DSPMETAFILEPICT
            | CF_DSPENHMETAFILE
            | CF_GDIOBJFIRST..=CF_GDIOBJLAST
    )
}

/// Copies every restorable format. The clipboard must be open.
///
/// EnumClipboardFormats: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumclipboardformats
/// GetClipboardData: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboarddata
fn save_formats() -> Result<Vec<(u32, Vec<u8>)>, &'static str> {
    let mut saved = Vec::new();
    let mut total = 0;
    let mut format = 0;
    loop {
        // SAFETY: the clipboard is open (caller).
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            return Ok(saved);
        }
        if !restorable(format) {
            continue;
        }
        // SAFETY: the clipboard is open; for restorable formats the handle is an HGLOBAL.
        let Ok(handle) = (unsafe { GetClipboardData(format) }) else { continue };
        let Some(bytes) = (unsafe { read_global(HGLOBAL(handle.0)) }) else { continue };
        total += bytes.len();
        if total > MAX_SAVED_BYTES {
            return Err("your clipboard holds too much to restore afterwards");
        }
        saved.push((format, bytes));
    }
}

/// The clipboard's text, up to the first NUL. The clipboard must be open.
fn read_text() -> Option<String> {
    // SAFETY: the clipboard is open (caller); CF_UNICODETEXT is an HGLOBAL of UTF-16 units.
    let bytes = unsafe { read_global(HGLOBAL(GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok()?.0))? };
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&pair| u16::from_ne_bytes(pair)).collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    Some(String::from_utf16_lossy(&units[..end]))
}

/// GlobalLock: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-globallock
///
/// # Safety
/// `global` must be a valid HGLOBAL (here: owned by the open clipboard).
unsafe fn read_global(global: HGLOBAL) -> Option<Vec<u8>> {
    // SAFETY: see the function's contract; the lock is released before returning.
    unsafe {
        let size = GlobalSize(global);
        if size == 0 {
            return None;
        }
        let data = GlobalLock(global);
        if data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}

/// Puts the saved formats back. The restored content is marked so clipboard history and
/// cloud clipboard skip it: it is the user's old content, not something new.
///
/// SetClipboardData: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setclipboarddata
/// History and cloud formats: https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats#cloud-clipboard-and-clipboard-history-formats
fn restore(owner: HWND, saved: &[(u32, Vec<u8>)]) -> Result<(), &'static str> {
    let _open = OpenedClipboard::open(owner)?;
    // SAFETY: the clipboard is open with `owner`; EmptyClipboard makes it the owner.
    unsafe { EmptyClipboard() }.map_err(|_| "could not empty the clipboard")?;
    if saved.is_empty() {
        return Ok(());
    }
    for (format, bytes) in saved {
        set_global(*format, bytes);
    }
    let no = 0u32.to_ne_bytes();
    // SAFETY: plain calls with valid wide strings.
    let (history, cloud) = unsafe {
        (RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")), RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")))
    };
    for marker in [history, cloud].into_iter().filter(|&f| f != 0) {
        set_global(marker, &no);
    }
    Ok(())
}

/// Copies `bytes` into a new HGLOBAL and gives it to the clipboard, which must be open.
/// Returns whether the clipboard took it.
fn set_global(format: u32, bytes: &[u8]) -> bool {
    // SAFETY: a fresh HGLOBAL of at least `bytes.len()` bytes; ownership passes to the
    // clipboard when SetClipboardData succeeds, otherwise it is freed here.
    unsafe {
        let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else { return false };
        let data = GlobalLock(global);
        if data.is_null() {
            let _ = GlobalFree(Some(global));
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(global);
        if SetClipboardData(format, Some(HANDLE(global.0))).is_err() {
            let _ = GlobalFree(Some(global));
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(history: Option<u32>, cloud: Option<u32>) -> RegistryValues {
        RegistryValues { history, cloud, history_allowed: None, cloud_allowed: None }
    }

    #[test]
    fn never_set_means_off() {
        assert_eq!(policy_from(values(None, None)), ClipboardPolicy::default());
    }

    #[test]
    fn user_switches_are_read() {
        let p = policy_from(values(Some(1), Some(1)));
        assert!(p.history && p.cloud_sync);
        let p = policy_from(values(Some(0), Some(0)));
        assert!(!p.history && !p.cloud_sync);
    }

    #[test]
    fn group_policy_can_turn_features_off() {
        let p = policy_from(RegistryValues {
            history: Some(1),
            cloud: Some(1),
            history_allowed: Some(0),
            cloud_allowed: Some(0),
        });
        assert_eq!(p, ClipboardPolicy::default());
    }

    #[test]
    fn copy_only_when_switched_on_and_no_cloud_sync() {
        let none = ClipboardPolicy::default();
        let history = ClipboardPolicy { history: true, cloud_sync: false };
        let cloud = ClipboardPolicy { history: true, cloud_sync: true };
        assert_eq!(may_copy(false, none), Err(Blocked::SwitchedOff));
        assert_eq!(may_copy(true, none), Ok(()));
        assert_eq!(may_copy(true, history), Ok(()));
        assert_eq!(may_copy(true, cloud), Err(Blocked::CloudSync));
    }

    #[test]
    fn terminals_never_get_ctrl_c() {
        assert_eq!(copy_keys("WindowsTerminal.exe"), CopyKeys::CtrlInsert);
        assert_eq!(copy_keys("cmd.exe"), CopyKeys::CtrlInsert);
        assert_eq!(copy_keys("pwsh.exe"), CopyKeys::CtrlInsert);
        assert_eq!(copy_keys("wezterm-gui.exe"), CopyKeys::CtrlShiftC);
        assert_eq!(copy_keys("Alacritty.exe"), CopyKeys::CtrlShiftC);
    }

    #[test]
    fn other_apps_use_ctrl_c() {
        assert_eq!(copy_keys("msedge.exe"), CopyKeys::CtrlC);
        assert_eq!(copy_keys("Antigravity IDE.exe"), CopyKeys::CtrlC);
        assert_eq!(copy_keys(""), CopyKeys::CtrlC);
    }

    #[test]
    fn gdi_formats_are_not_saved() {
        assert!(restorable(u32::from(CF_UNICODETEXT.0)));
        assert!(restorable(1)); // CF_TEXT
        assert!(restorable(8)); // CF_DIB
        assert!(restorable(0xC123)); // a registered format such as "HTML Format"
        assert!(!restorable(2)); // CF_BITMAP
        assert!(!restorable(14)); // CF_ENHMETAFILE
        assert!(!restorable(0x350)); // private GDI object
    }
}
