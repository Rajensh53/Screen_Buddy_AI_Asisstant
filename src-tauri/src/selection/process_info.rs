//! Names for the app that owns a selection: the exe name (`chrome.exe`) and a friendly
//! name from the exe's version info ("Google Chrome"). Also finds which process owns
//! the window under a screen point.

use ::windows::Win32::Foundation::{CloseHandle, POINT};
use ::windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use ::windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use ::windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetWindowThreadProcessId, WindowFromPoint};
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
