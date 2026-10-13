//! Reads the current selection through Windows UI Automation. No clipboard, no screenshots.
//!
//! Must be created and used on one thread; it initializes COM (multithreaded) for that thread.
//!
//! Where the selection is looked for, in order (see `resolve`):
//! 1. the focused element, then its ancestors (focus on a link inside a web page -> the page document);
//! 2. the element under the mouse, then its ancestors (selections inside iframes);
//! 3. both again after `RETRY_DELAY`, if a Text pattern was found but nothing was selected yet.
//!
//! UI Automation client overview: https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-clientsoverview
//! Text pattern (GetSelection, GetText, GetBoundingRectangles):
//! https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nn-uiautomationclient-iuiautomationtextpattern
//! https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nn-uiautomationclient-iuiautomationtextrange

use std::thread;
use std::time::Duration;

use ::windows::Win32::Foundation::POINT;
use ::windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, SAFEARRAY,
};
use ::windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use ::windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextRange,
    IUIAutomationTreeWalker, UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_TextPatternId,
};

use super::Rect;
use super::trigger::{MAX_CHARS, Point};

/// Upper bound for `GetText`, in UTF-16 units. Generous enough for `MAX_CHARS` characters
/// (some take two units), while a select-all in a huge document stays cheap.
const MAX_TEXT_UNITS: i32 = (MAX_CHARS * 2) as i32;
/// How many ancestors to check for the Text pattern before giving up. Web apps nest deeply:
/// in the new Outlook the page document is 9-12 levels above the focused element or the
/// element under the mouse. The walk also stops at the desktop, so this is only a safety limit.
const MAX_ANCESTORS: usize = 32;
/// Wait before the single retry when the selection was still empty.
const RETRY_DELAY: Duration = Duration::from_millis(150);
/// How the selection was found. Logged in debug builds to learn which fallbacks real apps need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Focus,
    Parent,
    Point,
    Retry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    Selection { text: String, process_id: u32, rect: Option<Rect>, source: Source },
    /// A password field was found. Its text was not read.
    Password { process_id: u32 },
    /// The user is in a text area (edit box or document) that doesn't expose the Text
    /// pattern, so this app can't be read.
    NoTextPattern { process_id: u32 },
    /// The app has readable text here but reports nothing selected.
    EmptySelection { process_id: u32 },
    /// The gesture wasn't in a text area (title bar, tab, button, ...).
    NoSelection,
    /// Nothing has keyboard focus and nothing is under the mouse, or both are unreachable
    /// (for example an app running as administrator).
    NoFocus,
}

/// What one starting element (focused or under the mouse) and its ancestors offer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Probe {
    Selection { text: String, process_id: u32, rect: Option<Rect>, via_parent: bool },
    Password { process_id: u32 },
    /// A Text pattern exists but nothing is selected.
    Empty { process_id: u32 },
    /// No Text pattern on the element or its ancestors. `text_like`: the element is an
    /// edit box or document, so the user probably expected it to be readable.
    NoPattern { process_id: u32, text_like: bool },
    /// No element, or UI Automation failed for it.
    Unavailable,
}

trait SelectionProbe {
    fn focused(&self) -> Probe;
    fn at_point(&self, pt: Point) -> Probe;
}

enum Attempt {
    Done(ReadOutcome),
    /// Not readable yet, worth one retry; the outcome to report if the retry doesn't help.
    /// Covers a Text pattern with nothing selected yet, and a text area without a Text
    /// pattern yet (WebView2 apps like the new Outlook turn accessibility on only after
    /// the first request).
    Retry(ReadOutcome),
}

/// Decides where the selection comes from. Pure logic over `SelectionProbe`, so it is unit-tested.
fn resolve(probe: &impl SelectionProbe, pt: Point, wait: impl FnOnce()) -> ReadOutcome {
    match try_once(probe, pt) {
        Attempt::Done(outcome) => outcome,
        Attempt::Retry(_) => {
            wait();
            match try_once(probe, pt) {
                Attempt::Done(ReadOutcome::Selection { text, process_id, rect, .. }) => {
                    ReadOutcome::Selection { text, process_id, rect, source: Source::Retry }
                }
                Attempt::Done(outcome) | Attempt::Retry(outcome) => outcome,
            }
        }
    }
}

fn try_once(probe: &impl SelectionProbe, pt: Point) -> Attempt {
    let focused = probe.focused();
    match focused {
        // A focused password field ends the read: nothing else is tried.
        Probe::Password { process_id } => return Attempt::Done(ReadOutcome::Password { process_id }),
        Probe::Selection { text, process_id, rect, via_parent } => {
            let source = if via_parent { Source::Parent } else { Source::Focus };
            return Attempt::Done(ReadOutcome::Selection { text, process_id, rect, source });
        }
        _ => {}
    }

    let at_point = probe.at_point(pt);
    match at_point {
        Probe::Password { process_id } => return Attempt::Done(ReadOutcome::Password { process_id }),
        Probe::Selection { text, process_id, rect, .. } => {
            return Attempt::Done(ReadOutcome::Selection { text, process_id, rect, source: Source::Point });
        }
        _ => {}
    }

    // The text under the mouse matters most, so its process is reported first.
    let empty = [&at_point, &focused].into_iter().find_map(|p| match p {
        Probe::Empty { process_id } => Some(*process_id),
        _ => None,
    });
    if let Some(process_id) = empty {
        return Attempt::Retry(ReadOutcome::EmptySelection { process_id });
    }
    let text_like = [&focused, &at_point].into_iter().find_map(|p| match p {
        Probe::NoPattern { process_id, text_like: true } => Some(*process_id),
        _ => None,
    });
    match (text_like, &focused, &at_point) {
        (Some(process_id), _, _) => Attempt::Retry(ReadOutcome::NoTextPattern { process_id }),
        (None, Probe::Unavailable, Probe::Unavailable) => Attempt::Done(ReadOutcome::NoFocus),
        _ => Attempt::Done(ReadOutcome::NoSelection),
    }
}

/// Initializes COM on the current thread and uninitializes it on drop.
struct ComGuard;

impl ComGuard {
    fn new() -> ::windows::core::Result<Self> {
        // CoInitializeEx: https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-coinitializeex
        // SAFETY: plain COM initialization for this thread, paired with CoUninitialize in Drop.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: matches the successful CoInitializeEx in `new`.
        unsafe { CoUninitialize() };
    }
}

pub struct UiaReader {
    automation: IUIAutomation,
    /// Walks the control view of the UI tree (the elements users interact with).
    walker: IUIAutomationTreeWalker,
    /// The desktop: the ancestor walk stops before reaching it.
    root: IUIAutomationElement,
    // Declared last so COM is uninitialized after the COM objects above are released.
    _com: ComGuard,
}

impl UiaReader {
    pub fn new() -> ::windows::core::Result<Self> {
        let com = ComGuard::new()?;
        // SAFETY: COM is initialized on this thread.
        unsafe {
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            // ControlViewWalker: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation-get_controlviewwalker
            let walker = automation.ControlViewWalker()?;
            let root = automation.GetRootElement()?;
            Ok(Self { automation, walker, root, _com: com })
        }
    }

    /// Reads the user's selection. `at` is where the gesture ended (physical screen pixels).
    pub fn read(&self, at: Point) -> ReadOutcome {
        resolve(self, at, || thread::sleep(RETRY_DELAY))
    }

    /// Checks `start`, then its ancestors, for the Text pattern and reads its selection.
    /// Every element is checked for `IsPassword` before any text is read.
    fn probe_from(&self, start: &IUIAutomationElement) -> Probe {
        // SAFETY: COM calls on the thread that owns `self` and initialized COM.
        unsafe {
            let Ok(process_id) = start.CurrentProcessId() else { return Probe::Unavailable };
            let process_id = process_id as u32;
            let text_like = is_text_like(start);

            let mut current = start.clone();
            for depth in 0..=MAX_ANCESTORS {
                if is_password(&current) {
                    return Probe::Password { process_id };
                }
                if let Ok(pattern) = current.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
                    let process_id = current.CurrentProcessId().map_or(process_id, |p| p as u32);
                    return match read_selection(&pattern) {
                        Ok(Some((text, rect))) => Probe::Selection { text, process_id, rect, via_parent: depth > 0 },
                        Ok(None) => Probe::Empty { process_id },
                        Err(_) => Probe::Unavailable,
                    };
                }
                // GetParentElement: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationtreewalker-getparentelement
                let Ok(parent) = self.walker.GetParentElement(&current) else { break };
                if self.automation.CompareElements(&parent, &self.root).is_ok_and(|same| same.as_bool()) {
                    break;
                }
                current = parent;
            }
            Probe::NoPattern { process_id, text_like }
        }
    }
}

impl SelectionProbe for UiaReader {
    fn focused(&self) -> Probe {
        // GetFocusedElement: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation-getfocusedelement
        // SAFETY: COM call on the thread that owns `self`.
        match unsafe { self.automation.GetFocusedElement() } {
            Ok(element) => self.probe_from(&element),
            Err(_) => Probe::Unavailable,
        }
    }

    fn at_point(&self, pt: Point) -> Probe {
        // ElementFromPoint: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation-elementfrompoint
        // SAFETY: COM call on the thread that owns `self`.
        match unsafe { self.automation.ElementFromPoint(POINT { x: pt.x, y: pt.y }) } {
            Ok(element) => self.probe_from(&element),
            Err(_) => Probe::Unavailable,
        }
    }
}

/// `IsPassword` property. Errors count as "password" so a failing check never leads to a read.
///
/// https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-get_currentispassword
fn is_password(element: &IUIAutomationElement) -> bool {
    // SAFETY: COM property read on the owning thread.
    unsafe { element.CurrentIsPassword().map_or(true, |b| b.as_bool()) }
}

/// Edit boxes and documents are where users select text.
///
/// https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-controltypesoverview
fn is_text_like(element: &IUIAutomationElement) -> bool {
    // SAFETY: COM property read on the owning thread.
    unsafe {
        element
            .CurrentControlType()
            .is_ok_and(|t| t == UIA_EditControlTypeId || t == UIA_DocumentControlTypeId)
    }
}

/// Text and screen rectangle of the pattern's selection, or `None` if nothing is selected.
fn read_selection(pattern: &IUIAutomationTextPattern) -> ::windows::core::Result<Option<(String, Option<Rect>)>> {
    // SAFETY: COM calls on the owning thread.
    unsafe {
        let ranges = pattern.GetSelection()?;
        let mut parts = Vec::new();
        let mut rect: Option<Rect> = None;
        for i in 0..ranges.Length()? {
            let range = ranges.GetElement(i)?;
            let text = range.GetText(MAX_TEXT_UNITS)?;
            if !text.is_empty() {
                parts.push(String::from_utf16_lossy(&text));
            }
            if let Some(r) = bounding_rect(&range) {
                rect = Some(rect.map_or(r, |acc| acc.union(r)));
            }
        }
        Ok((!parts.is_empty()).then(|| (parts.join("\n"), rect)))
    }
}

/// Union of the on-screen rectangles of a text range, in physical screen pixels.
fn bounding_rect(range: &IUIAutomationTextRange) -> Option<Rect> {
    // SAFETY: the returned SAFEARRAY is owned by us and destroyed in `read_doubles`.
    let values = unsafe { read_doubles(range.GetBoundingRectangles().ok()?) };
    union_rects(&values)
}

/// Copies a one-dimensional SAFEARRAY of doubles and destroys it.
///
/// SafeArrayAccessData: https://learn.microsoft.com/en-us/windows/win32/api/oleauto/nf-oleauto-safearrayaccessdata
unsafe fn read_doubles(array: *mut SAFEARRAY) -> Vec<f64> {
    if array.is_null() {
        return Vec::new();
    }
    let mut values = Vec::new();
    // SAFETY: `array` is a valid VT_R8 SAFEARRAY returned by UI Automation.
    unsafe {
        if let (Ok(lower), Ok(upper)) = (SafeArrayGetLBound(array, 1), SafeArrayGetUBound(array, 1)) {
            let len = (upper - lower + 1).max(0) as usize;
            let mut data = std::ptr::null_mut();
            if len > 0 && SafeArrayAccessData(array, &mut data).is_ok() {
                values.extend_from_slice(std::slice::from_raw_parts(data as *const f64, len));
                let _ = SafeArrayUnaccessData(array);
            }
        }
        let _ = SafeArrayDestroy(array);
    }
    values
}

/// `GetBoundingRectangles` returns `left, top, width, height` for each line of the range.
/// Empty rectangles (for example blank lines) are skipped.
fn union_rects(values: &[f64]) -> Option<Rect> {
    let (rects, _partial) = values.as_chunks::<4>();
    rects
        .iter()
        .filter(|&&[_, _, width, height]| width > 0.0 && height > 0.0)
        .map(|&[left, top, width, height]| Rect {
            left: left.round() as i32,
            top: top.round() as i32,
            right: (left + width).round() as i32,
            bottom: (top + height).round() as i32,
        })
        .reduce(Rect::union)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    const PT: Point = Point::new(100, 200);

    /// Fake probe: returns scripted answers per attempt and counts calls.
    struct Fake {
        focused: RefCell<Vec<Probe>>,
        at_point: RefCell<Vec<Probe>>,
        point_calls: Cell<usize>,
    }

    impl Fake {
        /// Answers are used in order, one per attempt (first try, then retry).
        fn new(focused: Vec<Probe>, at_point: Vec<Probe>) -> Self {
            Self { focused: RefCell::new(focused), at_point: RefCell::new(at_point), point_calls: Cell::new(0) }
        }
    }

    impl SelectionProbe for Fake {
        fn focused(&self) -> Probe {
            self.focused.borrow_mut().remove(0)
        }
        fn at_point(&self, pt: Point) -> Probe {
            assert_eq!(pt, PT);
            self.point_calls.set(self.point_calls.get() + 1);
            self.at_point.borrow_mut().remove(0)
        }
    }

    fn selection(text: &str, via_parent: bool) -> Probe {
        Probe::Selection { text: text.into(), process_id: 7, rect: None, via_parent }
    }

    fn empty(process_id: u32) -> Probe {
        Probe::Empty { process_id }
    }

    fn outcome(text: &str, source: Source) -> ReadOutcome {
        ReadOutcome::Selection { text: text.into(), process_id: 7, rect: None, source }
    }

    fn run(fake: &Fake) -> (ReadOutcome, bool) {
        let waited = Cell::new(false);
        let result = resolve(fake, PT, || waited.set(true));
        (result, waited.get())
    }

    #[test]
    fn focused_selection_is_used_without_asking_the_point() {
        let fake = Fake::new(vec![selection("hello", false)], vec![]);
        assert_eq!(run(&fake), (outcome("hello", Source::Focus), false));
        assert_eq!(fake.point_calls.get(), 0);
    }

    #[test]
    fn selection_found_on_a_parent_is_marked() {
        let fake = Fake::new(vec![selection("hello", true)], vec![]);
        assert_eq!(run(&fake).0, outcome("hello", Source::Parent));
    }

    #[test]
    fn focused_password_stops_without_asking_the_point() {
        let fake = Fake::new(vec![Probe::Password { process_id: 3 }], vec![]);
        assert_eq!(run(&fake), (ReadOutcome::Password { process_id: 3 }, false));
        assert_eq!(fake.point_calls.get(), 0);
    }

    #[test]
    fn point_selection_is_used_when_focus_has_none() {
        let fake = Fake::new(vec![empty(9)], vec![selection("in iframe", false)]);
        assert_eq!(run(&fake), (outcome("in iframe", Source::Point), false));
    }

    #[test]
    fn point_password_is_never_read() {
        let fake = Fake::new(
            vec![Probe::NoPattern { process_id: 1, text_like: false }],
            vec![Probe::Password { process_id: 2 }],
        );
        assert_eq!(run(&fake).0, ReadOutcome::Password { process_id: 2 });
    }

    #[test]
    fn empty_selection_is_retried_once() {
        let fake = Fake::new(vec![empty(9), empty(9)], vec![empty(9), selection("late", false)]);
        assert_eq!(run(&fake), (outcome("late", Source::Retry), true));
    }

    #[test]
    fn still_empty_after_retry_is_empty_selection() {
        let fake = Fake::new(vec![empty(9), empty(9)], vec![empty(9), empty(9)]);
        assert_eq!(run(&fake), (ReadOutcome::EmptySelection { process_id: 9 }, true));
        assert_eq!(fake.point_calls.get(), 2);
    }

    #[test]
    fn empty_selection_reports_the_process_under_the_mouse() {
        let fake = Fake::new(vec![empty(1), empty(1)], vec![empty(2), empty(2)]);
        assert_eq!(run(&fake).0, ReadOutcome::EmptySelection { process_id: 2 });
        let fake = Fake::new(vec![empty(1), empty(1)], vec![Probe::Unavailable, Probe::Unavailable]);
        assert_eq!(run(&fake).0, ReadOutcome::EmptySelection { process_id: 1 });
    }

    #[test]
    fn no_pattern_on_a_button_is_silent_and_not_retried() {
        let fake = Fake::new(
            vec![Probe::NoPattern { process_id: 4, text_like: false }],
            vec![Probe::NoPattern { process_id: 4, text_like: false }],
        );
        assert_eq!(run(&fake), (ReadOutcome::NoSelection, false));
    }

    #[test]
    fn text_area_still_without_pattern_after_retry_is_cant_read() {
        let no_pattern = Probe::NoPattern { process_id: 5, text_like: true };
        let fake = Fake::new(
            vec![no_pattern.clone(), no_pattern.clone()],
            vec![Probe::NoPattern { process_id: 5, text_like: false }, Probe::Unavailable],
        );
        assert_eq!(run(&fake), (ReadOutcome::NoTextPattern { process_id: 5 }, true));
    }

    #[test]
    fn text_area_readable_on_retry_gives_selection() {
        // WebView2 apps (new Outlook) expose the Text pattern only after the first request.
        let fake = Fake::new(
            vec![Probe::NoPattern { process_id: 5, text_like: true }, empty(9)],
            vec![Probe::Unavailable, selection("Security info was added", false)],
        );
        assert_eq!(run(&fake), (outcome("Security info was added", Source::Retry), true));
    }

    #[test]
    fn nothing_reachable_is_no_focus() {
        let fake = Fake::new(vec![Probe::Unavailable], vec![Probe::Unavailable]);
        assert_eq!(run(&fake), (ReadOutcome::NoFocus, false));
    }

    #[test]
    fn union_of_no_rects_is_none() {
        assert_eq!(union_rects(&[]), None);
    }

    #[test]
    fn union_of_one_rect() {
        assert_eq!(
            union_rects(&[10.0, 20.0, 100.0, 15.0]),
            Some(Rect { left: 10, top: 20, right: 110, bottom: 35 })
        );
    }

    #[test]
    fn union_covers_all_lines() {
        let lines = [50.0, 100.0, 300.0, 20.0, 10.0, 120.0, 80.0, 20.0];
        assert_eq!(union_rects(&lines), Some(Rect { left: 10, top: 100, right: 350, bottom: 140 }));
    }

    #[test]
    fn union_skips_empty_rects_and_partial_entries() {
        let values = [0.0, 0.0, 0.0, 0.0, 10.0, 10.0, 5.0, 5.0, 99.0];
        assert_eq!(union_rects(&values), Some(Rect { left: 10, top: 10, right: 15, bottom: 15 }));
    }
}
