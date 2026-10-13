//! Where the popup goes: next to the selection, always inside the monitor's work area.
//! Pure logic in physical pixels, so every rule is unit-tested.

use crate::selection::{Point, Rect};

/// Space between the selection and the popup.
pub const GAP: i32 = 8;
/// Without a selection rectangle the popup is placed around the pointer; this keeps it
/// clear of the mouse cursor.
const POINTER_PAD: i32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Below,
    Above,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

/// What the popup is placed next to: the selection, or the pointer (where the mouse was
/// released) when the selection rectangle is missing, very tall or off screen.
pub fn anchor(selection: Option<Rect>, pointer: Point, work_area: Rect) -> Rect {
    match selection {
        Some(r) if r.height() <= work_area.height() / 3 && r.intersects(work_area) => r,
        _ => Rect {
            left: pointer.x,
            top: pointer.y - POINTER_PAD,
            right: pointer.x,
            bottom: pointer.y + POINTER_PAD,
        },
    }
}

/// The popup's screen rectangle, and the side of the anchor it ended up on.
///
/// `prefer` is tried first. Pass the previous side when the popup grows, so it doesn't jump
/// from below the selection to above it in the middle of an answer.
pub fn place(anchor: Rect, size: Size, work_area: Rect, prefer: Side) -> (Rect, Side) {
    let below_top = anchor.bottom + GAP;
    let above_top = anchor.top - GAP - size.height;
    let fits = |side| match side {
        Side::Below => below_top + size.height <= work_area.bottom,
        Side::Above => above_top >= work_area.top,
    };
    let other = match prefer {
        Side::Below => Side::Above,
        Side::Above => Side::Below,
    };
    let side = if fits(prefer) {
        prefer
    } else if fits(other) {
        other
    } else if work_area.bottom - anchor.bottom >= anchor.top - work_area.top {
        Side::Below
    } else {
        Side::Above
    };

    let top = match side {
        Side::Below => below_top,
        Side::Above => above_top,
    };
    let top = clamp(top, work_area.top, work_area.bottom - size.height);
    let left = clamp(anchor.left, work_area.left, work_area.right - size.width);
    (Rect { left, top, right: left + size.width, bottom: top + size.height }, side)
}

/// Popup height rules, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeightLimits {
    pub min: i32,
    pub max: i32,
    /// The popup grows in jumps of this size.
    pub step: i32,
}

/// The popup's next height when its content needs `needed` pixels.
///
/// Every resize makes the web view repaint, which looks like a jerk. So while the answer
/// streams in, the popup only grows, a whole `step` at a time. Once the answer is complete it
/// fits the content: it grows to fit, and shrinks only when that frees at least a `step`.
pub fn popup_height(current: i32, needed: i32, complete: bool, limits: HeightLimits) -> i32 {
    let needed = needed.clamp(limits.min, limits.max);
    if complete {
        return if needed > current || current - needed >= limits.step { needed } else { current };
    }
    if needed <= current {
        return current;
    }
    let steps = (needed - current + limits.step - 1) / limits.step;
    (current + steps * limits.step).min(limits.max)
}

/// Like `i32::clamp`, but when the popup is bigger than the area it sticks to `min`.
fn clamp(value: i32, min: i32, max: i32) -> i32 {
    if max < min { min } else { value.clamp(min, max) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1920x1080 monitor with a 48 px taskbar at the bottom.
    const WORK: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1032 };
    const SIZE: Size = Size { width: 360, height: 200 };

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    #[test]
    fn below_the_selection_when_there_is_room() {
        let word = rect(300, 400, 380, 422);
        let (popup, side) = place(word, SIZE, WORK, Side::Below);
        assert_eq!(side, Side::Below);
        assert_eq!(popup, rect(300, 430, 660, 630));
    }

    #[test]
    fn above_the_selection_near_the_bottom_of_the_screen() {
        let word = rect(300, 950, 380, 972);
        let (popup, side) = place(word, SIZE, WORK, Side::Below);
        assert_eq!(side, Side::Above);
        assert_eq!(popup, rect(300, 742, 660, 942));
    }

    #[test]
    fn neither_side_fits_takes_the_bigger_side_and_stays_on_screen() {
        let tall = Size { width: 360, height: 700 };
        let word = rect(300, 400, 380, 422);
        let (popup, side) = place(word, tall, WORK, Side::Below);
        // 610 px below vs 400 px above: below, clamped to the bottom of the work area.
        assert_eq!(side, Side::Below);
        assert_eq!(popup.bottom, WORK.bottom);
        assert!(popup.top >= WORK.top);
    }

    #[test]
    fn selection_partly_off_screen_on_the_left() {
        // Seen in manual testing: Notepad dragged partly off screen gave left = -525.
        let line = rect(-525, 400, 200, 422);
        let (popup, _) = place(line, SIZE, WORK, Side::Below);
        assert_eq!(popup.left, 0);
    }

    #[test]
    fn selection_near_the_right_edge() {
        let word = rect(1850, 400, 1910, 422);
        let (popup, _) = place(word, SIZE, WORK, Side::Below);
        assert_eq!(popup.right, WORK.right);
        assert_eq!(popup.width(), SIZE.width);
    }

    #[test]
    fn missing_rect_uses_the_pointer() {
        let a = anchor(None, Point::new(500, 300), WORK);
        assert_eq!(a, rect(500, 284, 500, 316));
        let (popup, _) = place(a, SIZE, WORK, Side::Below);
        assert_eq!((popup.left, popup.top), (500, 324));
    }

    #[test]
    fn very_tall_selection_uses_the_pointer() {
        let page = rect(100, 50, 1200, 1000);
        assert_eq!(anchor(Some(page), Point::new(700, 600), WORK), rect(700, 584, 700, 616));
    }

    #[test]
    fn selection_outside_the_work_area_uses_the_pointer() {
        let hidden = rect(-900, 400, -600, 422);
        assert_eq!(anchor(Some(hidden), Point::new(10, 410), WORK).left, 10);
    }

    #[test]
    fn normal_selection_is_the_anchor() {
        let word = rect(300, 400, 380, 422);
        assert_eq!(anchor(Some(word), Point::new(340, 410), WORK), word);
    }

    #[test]
    fn kept_side_does_not_flip_while_growing() {
        let word = rect(300, 600, 380, 622);
        // Placed above (prefer Above), then grows: still fits above, so it stays above
        // even though it would also fit below.
        let small = Size { width: 360, height: 100 };
        let bigger = Size { width: 360, height: 300 };
        assert_eq!(place(word, small, WORK, Side::Above).1, Side::Above);
        let (popup, side) = place(word, bigger, WORK, Side::Above);
        assert_eq!(side, Side::Above);
        assert_eq!(popup.bottom, word.top - GAP);
    }

    const LIMITS: HeightLimits = HeightLimits { min: 80, max: 320, step: 48 };

    #[test]
    fn streaming_never_shrinks() {
        assert_eq!(popup_height(200, 75, false, LIMITS), 200);
        assert_eq!(popup_height(200, 200, false, LIMITS), 200);
    }

    #[test]
    fn streaming_grows_in_whole_steps() {
        assert_eq!(popup_height(200, 201, false, LIMITS), 248);
        assert_eq!(popup_height(200, 248, false, LIMITS), 248);
        assert_eq!(popup_height(200, 260, false, LIMITS), 296);
        assert_eq!(popup_height(296, 400, false, LIMITS), 320);
    }

    #[test]
    fn complete_answer_fits_its_content() {
        // Grows to the exact height.
        assert_eq!(popup_height(200, 230, true, LIMITS), 230);
        // A short answer or error frees more than a step: shrink once.
        assert_eq!(popup_height(200, 100, true, LIMITS), 100);
        // Freeing less than a step is not worth a resize.
        assert_eq!(popup_height(248, 220, true, LIMITS), 248);
    }

    #[test]
    fn height_stays_within_limits() {
        assert_eq!(popup_height(200, 30, true, LIMITS), 80);
        assert_eq!(popup_height(200, 900, true, LIMITS), 320);
    }

    #[test]
    fn second_monitor_left_of_the_primary() {
        let work = rect(-1920, 0, 0, 1040);
        let word = rect(-100, 400, -20, 422);
        let (popup, side) = place(word, SIZE, work, Side::Below);
        assert_eq!(side, Side::Below);
        assert_eq!(popup.right, 0);
        assert_eq!(popup.left, -360);
    }
}
