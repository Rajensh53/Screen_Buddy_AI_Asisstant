//! Names for the app that owns a selection: the exe name (`chrome.exe`) and a friendly
//! name from the exe's version info ("Google Chrome"). Also finds which process owns
//! the window under a screen point.

use ::windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT, WPARAM};
use ::windows::core::BOOL;
use ::windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use ::windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GA_ROOT, GetAncestor, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_NCHITTEST, WindowFromPoint,
};
use ::windows::core::{HSTRING, PCWSTR, PWSTR, w};

use super::trigger::Point;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// File name of the executable, for example `chrome.exe`.
    pub process_name: String,
    /// Friendly name, for example `Google Chrome`. Falls back to the exe name without extension.
    pub app_name: String,
}

pub fn lookup(process_id: u32) -> ProcessInfo {
    let Some(path) = image_path(process_id) else {
        let unknown = format!("pid {process_id}");
        return ProcessInfo { process_name: unknown.clone(), app_name: unknown };
    };
    let process_name = file_name(&path);
    let app_name = file_description(&path)
        .or_else(|| std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| process_name.clone());
    ProcessInfo { process_name, app_name }
}

/// Exe file name only, for example `chrome.exe`. Cheaper than `lookup` (no version-info read),
/// so it is used for the privacy check before every read.
pub fn process_name(process_id: u32) -> Option<String> {
    image_path(process_id).map(|path| file_name(&path))
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned())
}

/// Process id of the top-level window under `pt` (physical screen coordinates).
///
/// WindowFromPoint: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-windowfrompoint
pub fn process_at(pt: Point) -> Option<u32> {
    // SAFETY: plain Win32 queries; a null HWND is checked before use.
    unsafe {
        let hwnd = WindowFromPoint(POINT { x: pt.x, y: pt.y });
        if hwnd.is_invalid() {
            return None;
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        let target = if root.is_invalid() { hwnd } else { root };
        let mut pid = 0u32;
        GetWindowThreadProcessId(target, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

/// Whether `pt` is on a window's frame rather than its content: the title bar, a border, a
/// scroll bar, the menu bar or the minimize/maximize/close buttons. Gestures there move or
/// resize windows; they never select text. Asks the windows themselves, without reading any
/// text. A window that doesn't answer within 50 ms counts as content.
///
/// Apps that draw their own title bar can answer "title bar" (HTCAPTION) for their whole
/// window and show their content in a child window: Windows Terminal does, with its text in a
/// `DesktopWindowContentBridge` child that answers HTCLIENT. So a title-bar answer only counts
/// if no visible child window at that point says it is content.
///
/// WM_NCHITTEST: https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-nchittest
pub fn on_window_frame(pt: Point) -> bool {
    // SAFETY: plain Win32 query; a null HWND is checked.
    let hwnd = unsafe { WindowFromPoint(POINT { x: pt.x, y: pt.y }) };
    if hwnd.is_invalid() {
        return false;
    }
    match hit_test(hwnd, pt) {
        Some(HTCAPTION) => !content_child_at(hwnd, pt),
        Some(code) => is_frame_hit(code),
        None => false,
    }
}

const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;

/// SendMessageTimeoutW: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw
fn hit_test(hwnd: HWND, pt: Point) -> Option<isize> {
    let mut result = 0usize;
    // SAFETY: `hwnd` is a window handle (a stale one makes the call fail); `result` outlives it.
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_NCHITTEST,
            WPARAM(0),
            LPARAM(point_lparam(pt)),
            SMTO_ABORTIFHUNG,
            50,
            Some(&mut result),
        )
    };
    (sent.0 != 0).then_some(result as isize)
}

/// Whether a visible child window of `parent` that contains `pt` answers HTCLIENT.
///
/// EnumChildWindows: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumchildwindows
fn content_child_at(parent: HWND, pt: Point) -> bool {
    struct Search {
        pt: Point,
        found: bool,
    }
    unsafe extern "system" fn visit(child: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `Search` passed below, alive for the whole enumeration.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut r = RECT::default();
        // SAFETY: plain queries on a window handle given by EnumChildWindows.
        let contains = unsafe { IsWindowVisible(child).as_bool() && GetWindowRect(child, &mut r).is_ok() }
            && search.pt.x >= r.left
            && search.pt.x < r.right
            && search.pt.y >= r.top
            && search.pt.y < r.bottom;
        if contains && hit_test(child, search.pt) == Some(HTCLIENT) {
            search.found = true;
            return BOOL(0); // stop
        }
        BOOL(1)
    }
    let mut search = Search { pt, found: false };
    // SAFETY: `visit` only uses `search` during this call.
    let _ = unsafe { EnumChildWindows(Some(parent), Some(visit), LPARAM(&raw mut search as isize)) };
    search.found
}

/// The screen point packed like `MAKELPARAM(x, y)`: two signed 16-bit halves, so points on a
/// monitor left of or above the primary one (negative coordinates) survive.
fn point_lparam(pt: Point) -> isize {
    let packed = u32::from(pt.x as u16) | (u32::from(pt.y as u16) << 16);
    packed as i32 as isize
}

/// Hit-test codes: HTCLIENT (1) is the content. Codes from HTCAPTION (2) up are the frame;
/// HTNOWHERE (0), HTTRANSPARENT (-1) and HTERROR (-2) are not.
fn is_frame_hit(code: isize) -> bool {
    code >= HTCAPTION
}

/// Full path of the process's executable.
///
/// QueryFullProcessImageNameW: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew
fn image_path(process_id: u32) -> Option<String> {
    // SAFETY: the handle is closed before returning; the buffer outlives the call.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// `FileDescription` from the exe's version resource.
///
/// GetFileVersionInfoW: https://learn.microsoft.com/en-us/windows/win32/api/winver/nf-winver-getfileversioninfow
/// VerQueryValueW: https://learn.microsoft.com/en-us/windows/win32/api/winver/nf-winver-verqueryvaluew
fn file_description(path: &str) -> Option<String> {
    let path = HSTRING::from(path);
    // SAFETY: `data` outlives every pointer VerQueryValueW returns into it; lengths come
    // from the API.
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, data.as_mut_ptr().cast()).ok()?;
        let block = data.as_ptr().cast();

        // Language and code page of the first translation; US English Unicode as a fallback.
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let translation = if VerQueryValueW(block, w!("\\VarFileInfo\\Translation"), &mut ptr, &mut len).as_bool()
            && len >= 4
        {
            let pair = ptr as *const u16;
            format!("{:04x}{:04x}", *pair, *pair.add(1))
        } else {
            "040904b0".to_owned()
        };

        let key = HSTRING::from(format!("\\StringFileInfo\\{translation}\\FileDescription"));
        if !VerQueryValueW(block, PCWSTR(key.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let chars = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let text = String::from_utf16_lossy(chars);
        let text = text.trim_end_matches('\0').trim();
        (!text.is_empty()).then(|| text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_content_is_not_the_frame() {
        assert!(!is_frame_hit(1)); // HTCLIENT
        assert!(!is_frame_hit(0)); // HTNOWHERE
        assert!(!is_frame_hit(-1)); // HTTRANSPARENT
        assert!(is_frame_hit(2)); // HTCAPTION
        assert!(is_frame_hit(7)); // HTVSCROLL
        assert!(is_frame_hit(20)); // HTCLOSE
    }

    #[test]
    fn point_is_packed_like_makelparam() {
        assert_eq!(point_lparam(Point::new(10, 20)), (20 << 16) | 10);
        // A monitor left of the primary one: x = -100 keeps its sign in the low word.
        let packed = point_lparam(Point::new(-100, 5)) as u32;
        assert_eq!(packed & 0xFFFF, u32::from((-100i16) as u16));
        assert_eq!(packed >> 16, 5);
    }
}
