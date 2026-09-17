use std::{sync::{Arc, Mutex}, time::{Duration, Instant}};
use crate::geometry::WindowSize;

pub(crate) type SharedResize = Arc<Mutex<Resize>>;

/*
 * The window's size, and when it last changed.  Both halves of a resize read it: the frame
 * is blurred while a drag is still moving the window, and the display bridge waits for the
 * size to hold still before it asks the guest for it, because the guest re-reads its EDID and
 * re-applies its mode for every size it is told and a stream of them leaves its desktop
 * tracing a console that is still changing modes.  The blur outlasts the request: the guest
 * needs a moment to re-mode, and only then does it send a frame that size.
 *
 * The blur it hands out is a radius to paint with rather than on or off, so it comes up over a
 * few frames and lets go the same way: neither end of a drag is the window jumping from sharp
 * to blurred in a single frame.
 */
pub(crate) const RESIZE_SETTLE: Duration = Duration::from_millis(150);

pub(crate) const BLUR_HOLD: Duration = Duration::from_millis(500);

pub(crate) const BLUR_FADE: Duration = Duration::from_millis(60);

pub(crate) const BLUR_RADIUS: f32 = 28.0;

pub(crate) struct Resize {
    pub(crate) size: Option<WindowSize>,
    pub(crate) since: Option<Instant>,
    /* when the drag in flight started, which is what the blur ramps up over */
    pub(crate) drag: Option<Instant>,
}

impl Resize {
    pub(crate) fn new() -> Self {
        Self {
            size: None,
            since: None,
            drag: None,
        }
    }

    /*
     * The window is this size now.  A size it moves to while the blur is still up is the same
     * drag; one that arrives after it has cleared starts a new one, which is what the blur
     * comes up from; and the size it opened at is no drag at all.
     */
    pub(crate) fn changed(&mut self, size: WindowSize, now: Instant) {
        if self.size == Some(size) {
            return;
        }
        if self.size.is_some() && (self.drag.is_none() || self.unchanged_for(now) >= BLUR_HOLD) {
            self.drag = Some(now);
        }
        self.size = Some(size);
        self.since = Some(now);
    }    pub(crate) fn size(&self) -> Option<WindowSize> {
        self.size
    }

    pub(crate) fn dragging(&self, now: Instant) -> bool {
        self.drag.is_some() && self.unchanged_for(now) < BLUR_HOLD
    }

    /* what to blur the frame by, or nothing when there is no drag to cover */
    pub(crate) fn blur(&self, now: Instant) -> Option<f32> {
        if !self.dragging(now) {
            return None;
        }
        let left = BLUR_HOLD.checked_sub(self.unchanged_for(now))?;
        let up = now.saturating_duration_since(self.drag?);
        let eased = up.min(left).as_secs_f32() / BLUR_FADE.as_secs_f32();

        Some(BLUR_RADIUS * eased.min(1.0))
    }

    pub(crate) fn landed(&self, now: Instant) -> bool {
        self.size.is_some() && self.unchanged_for(now) >= RESIZE_SETTLE
    }

    pub(crate) fn unchanged_for(&self, now: Instant) -> Duration {
        self.since
            .map(|since| now.saturating_duration_since(since))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::window_size;

    fn ask(
        resize: &mut Resize,
        asked: &mut Option<WindowSize>,
        size: WindowSize,
        now: Instant,
    ) -> bool {
        resize.changed(size, now);
        if resize.landed(now) && *asked != Some(size) {
            *asked = Some(size);
            return true;
        }
        false
    }

    #[test]
    fn a_drag_asks_for_one_resize_at_the_size_it_lands_on() {
        let mut resize = Resize::new();
        let mut asked = None;
        let start = Instant::now();
        for step in 0..39u32 {
            let viewport = gpui::size(gpui::px(800.0 + step as f32 * 4.0), gpui::px(600.0));
            let at = start + Duration::from_millis(u64::from(step) * 10);
            assert!(
                !ask(&mut resize, &mut asked, window_size(viewport, 2.0), at),
                "a drag asked for a resize mid-flight"
            );
        }
        let landed = window_size(gpui::size(gpui::px(956.0), gpui::px(600.0)), 2.0);
        let at = start + Duration::from_millis(400);
        assert!(!ask(&mut resize, &mut asked, landed, at));
        assert!(ask(&mut resize, &mut asked, landed, at + RESIZE_SETTLE));
        assert!(!ask(&mut resize, &mut asked, landed, at + RESIZE_SETTLE * 2));
        assert_eq!(asked, Some(landed));
    }

    #[test]
    fn a_resize_blurs_and_then_clears_itself() {
        let size = |width: f32| window_size(gpui::size(gpui::px(width), gpui::px(600.0)), 1.0);
        let start = Instant::now();
        let mut resize = Resize::new();

        /* the size the window opens at is where it started, not a drag */
        let mut asked = None;
        assert!(!ask(&mut resize, &mut asked, size(800.0), start));
        assert!(!resize.dragging(start));

        resize.changed(size(804.0), start);
        assert!(resize.dragging(start));
        assert!(
            resize.landed(start + RESIZE_SETTLE),
            "the guest has to be asked while the blur is still up"
        );

        /* a size the window repeats holds nothing: the drag has not moved on */
        resize.changed(size(804.0), start + BLUR_HOLD / 2);
        assert!(resize.dragging(start + BLUR_HOLD - Duration::from_millis(1)));
        assert!(!resize.dragging(start + BLUR_HOLD));

        /* a size it moves on to holds it from the moment it moved */
        resize.changed(size(900.0), start + BLUR_HOLD / 2);
        assert!(resize.dragging(start + BLUR_HOLD));
        assert!(!resize.dragging(start + BLUR_HOLD + BLUR_HOLD / 2));
    }

    #[test]
    fn the_blur_comes_up_and_lets_go_over_a_few_frames() {
        let size = |width: f32| window_size(gpui::size(gpui::px(width), gpui::px(600.0)), 1.0);
        let start = Instant::now();
        let mut resize = Resize::new();

        let mut asked = None;
        ask(&mut resize, &mut asked, size(800.0), start);
        assert_eq!(
            resize.blur(start),
            None,
            "the size the window opened at is no drag"
        );

        resize.changed(size(804.0), start);
        assert_eq!(resize.blur(start), Some(0.0));
        let coming_up = resize.blur(start + BLUR_FADE / 2).unwrap();
        assert!(coming_up > 0.0 && coming_up < BLUR_RADIUS);
        assert_eq!(resize.blur(start + BLUR_FADE), Some(BLUR_RADIUS));

        /* the ramp is only at the ends: a drag still moving is blurred the whole way */
        resize.changed(size(900.0), start + BLUR_HOLD / 2);
        assert_eq!(
            resize.blur(start + BLUR_HOLD / 2 + BLUR_FADE),
            Some(BLUR_RADIUS)
        );

        let going_away = resize
            .blur(start + BLUR_HOLD / 2 + BLUR_HOLD - BLUR_FADE / 2)
            .unwrap();
        assert!(going_away > 0.0 && going_away < BLUR_RADIUS);
        assert_eq!(resize.blur(start + BLUR_HOLD / 2 + BLUR_HOLD), None);
    }
}
