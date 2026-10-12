//! Trigger rules: pure logic, no Windows calls, so it can be unit-tested.
//!
//! Turns raw left-button events from the mouse hook into "the user probably just
//! selected something" gestures, and filters the text that comes back.

use std::time::{Duration, Instant};

use super::Rect;

/// Minimum distance between button down and up that counts as a drag, in physical pixels.
pub const DRAG_THRESHOLD_PX: i32 = 6;
/// Selections shorter than this (in characters) are ignored.
pub const MIN_CHARS: usize = 2;
/// Longer selections are cut to this many characters.
pub const MAX_CHARS: usize = 2000;
/// The same text selected again within this window is ignored.
pub const REPEAT_WINDOW: Duration = Duration::from_secs(2);
/// How far from the selected text a gesture may be and still count as selecting it,
/// in physical pixels.
pub const NEAR_SELECTION_PX: i32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A left-button event from the mouse hook. `time_ms` is the hook's tick count,
/// which wraps around about every 49.7 days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseInput {
    LeftDown { pt: Point, time_ms: u32 },
    LeftUp { pt: Point, time_ms: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    /// Button released at least `DRAG_THRESHOLD_PX` away from where it went down.
    Drag { down: Point, up: Point },
    /// Second click of a double-click, reported when the button is released.
    DoubleClick { pt: Point },
}

/// System double-click settings (`GetDoubleClickTime`, `SM_CXDOUBLECLK`, `SM_CYDOUBLECLK`).
#[derive(Debug, Clone, Copy)]
pub struct DoubleClickSettings {
    pub time_ms: u32,
    /// Width and height of the rectangle, centered on the first click, that the
    /// second click must land in.
    pub width: i32,
    pub height: i32,
}

#[derive(Debug)]
pub struct GestureDetector {
    double_click: DoubleClickSettings,
    /// Where and when the button went down, while it is held.
    down: Option<(Point, u32)>,
    /// The last plain click, which may become the first half of a double-click.
    last_click: Option<(Point, u32)>,
    /// The current press is the second click of a double-click.
    second_click: bool,
}

impl GestureDetector {
    pub fn new(double_click: DoubleClickSettings) -> Self {
        Self { double_click, down: None, last_click: None, second_click: false }
    }

    pub fn feed(&mut self, input: MouseInput) -> Option<Gesture> {
        match input {
            MouseInput::LeftDown { pt, time_ms } => {
                self.second_click = self
                    .last_click
                    .take()
                    .is_some_and(|(first, first_ms)| self.is_double_click(first, first_ms, pt, time_ms));
                self.down = Some((pt, time_ms));
                None
            }
            MouseInput::LeftUp { pt, .. } => {
                let (down, down_ms) = self.down.take()?;
                if distance_sq(down, pt) >= DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX {
                    self.second_click = false;
                    return Some(Gesture::Drag { down, up: pt });
                }
                if std::mem::take(&mut self.second_click) {
                    return Some(Gesture::DoubleClick { pt });
                }
                self.last_click = Some((down, down_ms));
                None
            }
        }
    }

    fn is_double_click(&self, first: Point, first_ms: u32, second: Point, second_ms: u32) -> bool {
        let elapsed = second_ms.wrapping_sub(first_ms);
        elapsed <= self.double_click.time_ms
            && (second.x - first.x).abs() <= self.double_click.width / 2
            && (second.y - first.y).abs() <= self.double_click.height / 2
    }
}

/// Whether the gesture plausibly made the selection found at `rect`.
///
/// Dragging or double-clicking a title bar or scrollbar also triggers a read, which then
/// finds the app's old, unchanged selection. A real selection gesture happens on the text:
/// - Drag: both the press and the release are at the height of the selected lines.
///   Any horizontal position is fine, so dragging past the end of a line still counts.
/// - Double-click: the click is on the selected word.
///
/// Without a rectangle (some apps don't report one), the gesture is accepted.
pub fn gesture_near_selection(gesture: Gesture, rect: Option<Rect>) -> bool {
    let Some(r) = rect else { return true };
    let near_rows = |pt: Point| pt.y >= r.top - NEAR_SELECTION_PX && pt.y <= r.bottom + NEAR_SELECTION_PX;
    match gesture {
        Gesture::Drag { down, up } => near_rows(down) && near_rows(up),
        Gesture::DoubleClick { pt } => {
            near_rows(pt) && pt.x >= r.left - NEAR_SELECTION_PX && pt.x <= r.right + NEAR_SELECTION_PX
        }
    }
}

fn distance_sq(a: Point, b: Point) -> i32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
}

/// Trims the selection, drops it if it is too short and cuts it to `MAX_CHARS`
/// characters (never inside a multi-byte character, so Urdu text stays valid).
pub fn prepare_text(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.chars().take(MIN_CHARS).count() < MIN_CHARS {
        return None;
    }
    let text = match text.char_indices().nth(MAX_CHARS) {
        Some((cut, _)) => text[..cut].trim_end(),
        None => text,
    };
    Some(text.to_owned())
}

/// Remembers the last accepted selection to ignore the same text again within `REPEAT_WINDOW`.
#[derive(Debug, Default)]
pub struct RepeatFilter {
    last: Option<(String, Instant)>,
}

impl RepeatFilter {
    /// Returns `true` if `text` should be handled, and remembers it.
    pub fn accept(&mut self, text: &str, now: Instant) -> bool {
        if let Some((last, at)) = &self.last
            && last == text
            && now.saturating_duration_since(*at) < REPEAT_WINDOW
        {
            return false;
        }
        self.last = Some((text.to_owned(), now));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: DoubleClickSettings = DoubleClickSettings { time_ms: 500, width: 4, height: 4 };

    fn down(x: i32, y: i32, time_ms: u32) -> MouseInput {
        MouseInput::LeftDown { pt: Point::new(x, y), time_ms }
    }

    fn up(x: i32, y: i32, time_ms: u32) -> MouseInput {
        MouseInput::LeftUp { pt: Point::new(x, y), time_ms }
    }

    fn run(inputs: &[MouseInput]) -> Vec<Gesture> {
        let mut detector = GestureDetector::new(SETTINGS);
        inputs.iter().filter_map(|i| detector.feed(*i)).collect()
    }

    #[test]
    fn drag_at_threshold_triggers() {
        let gestures = run(&[down(100, 100, 0), up(106, 100, 300)]);
        assert_eq!(gestures, vec![Gesture::Drag { down: Point::new(100, 100), up: Point::new(106, 100) }]);
    }

    #[test]
    fn diagonal_drag_uses_real_distance() {
        // 4,4 is about 5.7 px: not a drag. 5,4 is about 6.4 px: a drag.
        assert!(run(&[down(0, 0, 0), up(4, 4, 100)]).is_empty());
        assert_eq!(run(&[down(0, 0, 0), up(5, 4, 100)]).len(), 1);
    }

    #[test]
    fn small_movement_is_a_click_not_a_drag() {
        assert!(run(&[down(100, 100, 0), up(105, 100, 100)]).is_empty());
    }

    #[test]
    fn single_click_does_not_trigger() {
        assert!(run(&[down(10, 10, 0), up(10, 10, 50)]).is_empty());
    }

    #[test]
    fn double_click_triggers_on_second_release() {
        let gestures = run(&[down(10, 10, 0), up(10, 10, 50), down(11, 11, 200), up(11, 11, 250)]);
        assert_eq!(gestures, vec![Gesture::DoubleClick { pt: Point::new(11, 11) }]);
    }

    #[test]
    fn slow_second_click_is_not_a_double_click() {
        assert!(run(&[down(10, 10, 0), up(10, 10, 50), down(10, 10, 501), up(10, 10, 550)]).is_empty());
    }

    #[test]
    fn far_second_click_is_not_a_double_click() {
        assert!(run(&[down(10, 10, 0), up(10, 10, 50), down(13, 10, 200), up(13, 10, 250)]).is_empty());
    }

    #[test]
    fn double_click_works_across_tick_wraparound() {
        let start = u32::MAX - 100;
        let gestures =
            run(&[down(10, 10, start), up(10, 10, start.wrapping_add(50)), down(10, 10, 150), up(10, 10, 200)]);
        assert_eq!(gestures.len(), 1);
    }

    #[test]
    fn third_click_does_not_trigger_again() {
        let gestures = run(&[
            down(10, 10, 0),
            up(10, 10, 50),
            down(10, 10, 100),
            up(10, 10, 150),
            down(10, 10, 200),
            up(10, 10, 250),
        ]);
        assert_eq!(gestures.len(), 1);
    }

    #[test]
    fn drag_after_click_is_a_drag() {
        let gestures = run(&[down(10, 10, 0), up(10, 10, 50), down(10, 10, 100), up(40, 10, 400)]);
        assert_eq!(gestures, vec![Gesture::Drag { down: Point::new(10, 10), up: Point::new(40, 10) }]);
    }

    #[test]
    fn release_without_press_is_ignored() {
        assert!(run(&[up(10, 10, 0)]).is_empty());
    }

    /// One selected line of text, like the Notepad selections seen in manual testing.
    const LINE: Rect = Rect { left: 280, top: 400, right: 740, bottom: 422 };

    fn drag(from: (i32, i32), to: (i32, i32)) -> Gesture {
        Gesture::Drag { down: Point::new(from.0, from.1), up: Point::new(to.0, to.1) }
    }

    #[test]
    fn drag_across_the_text_is_near_selection() {
        assert!(gesture_near_selection(drag((285, 410), (735, 412)), Some(LINE)));
    }

    #[test]
    fn drag_past_end_of_line_is_near_selection() {
        assert!(gesture_near_selection(drag((285, 410), (1200, 415)), Some(LINE)));
    }

    #[test]
    fn title_bar_drag_is_not_near_selection() {
        // Window dragged by its title bar, 100 px above the selected line.
        assert!(!gesture_near_selection(drag((500, 300), (450, 300)), Some(LINE)));
    }

    #[test]
    fn drag_ending_far_below_is_not_near_selection() {
        assert!(!gesture_near_selection(drag((285, 410), (300, 700)), Some(LINE)));
    }

    #[test]
    fn double_click_on_word_is_near_selection() {
        let word = Rect { left: 336, top: 393, right: 416, bottom: 415 };
        assert!(gesture_near_selection(Gesture::DoubleClick { pt: Point::new(370, 404) }, Some(word)));
    }

    #[test]
    fn title_bar_double_click_is_not_near_selection() {
        let word = Rect { left: 336, top: 393, right: 416, bottom: 415 };
        assert!(!gesture_near_selection(Gesture::DoubleClick { pt: Point::new(370, 300) }, Some(word)));
        assert!(!gesture_near_selection(Gesture::DoubleClick { pt: Point::new(900, 404) }, Some(word)));
    }

    #[test]
    fn gesture_without_rect_is_accepted() {
        assert!(gesture_near_selection(drag((0, 0), (500, 500)), None));
        assert!(gesture_near_selection(Gesture::DoubleClick { pt: Point::new(0, 0) }, None));
    }

    #[test]
    fn prepare_text_rejects_short_selections() {
        assert_eq!(prepare_text(""), None);
        assert_eq!(prepare_text("a"), None);
        assert_eq!(prepare_text("  a \n"), None);
        assert_eq!(prepare_text("ok"), Some("ok".into()));
    }

    #[test]
    fn prepare_text_counts_characters_not_bytes() {
        // One Urdu letter is 2 bytes but 1 character.
        assert_eq!(prepare_text("ب"), None);
        assert_eq!(prepare_text("دل"), Some("دل".into()));
    }

    #[test]
    fn prepare_text_trims_whitespace() {
        assert_eq!(prepare_text("  hello world \r\n"), Some("hello world".into()));
    }

    #[test]
    fn prepare_text_truncates_english() {
        let long = "a".repeat(MAX_CHARS + 50);
        assert_eq!(prepare_text(&long).unwrap().chars().count(), MAX_CHARS);
    }

    #[test]
    fn prepare_text_truncates_urdu_on_char_boundary() {
        let long = "سلام".repeat(MAX_CHARS);
        let text = prepare_text(&long).unwrap();
        assert_eq!(text.chars().count(), MAX_CHARS);
        assert!(long.starts_with(&text));
    }

    #[test]
    fn repeat_filter_skips_same_text_within_window() {
        let mut filter = RepeatFilter::default();
        let t0 = Instant::now();
        assert!(filter.accept("hello", t0));
        assert!(!filter.accept("hello", t0 + Duration::from_millis(1999)));
    }

    #[test]
    fn repeat_filter_allows_same_text_after_window() {
        let mut filter = RepeatFilter::default();
        let t0 = Instant::now();
        assert!(filter.accept("hello", t0));
        assert!(filter.accept("hello", t0 + REPEAT_WINDOW));
    }

    #[test]
    fn repeat_filter_allows_different_text() {
        let mut filter = RepeatFilter::default();
        let t0 = Instant::now();
        assert!(filter.accept("hello", t0));
        assert!(filter.accept("world", t0 + Duration::from_millis(100)));
    }
}
