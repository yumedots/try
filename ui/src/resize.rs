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
 * A corner still held is a drag that has not ended, whatever the size is doing: a hand that
 * pauses on the corner is not a window that has stopped moving, so nothing is asked for until
 * it is let go.  Asking as the drag moves was tried and is worse: a Wayland guest rebuilds its
 * output for every mode it adopts, which is a frame of its own background - grey - per ask, and
 * a drag is a dozen asks a second.  The guest is told once, at the size the drag landed on.
 *
 * The hold lasts until the guest sends the frame for that size, not for a fixed while: a guest
 * that re-modes quickly is uncovered as soon as it is back, and BLUR_HOLD is only the ceiling
 * for one that never sends that size at all (a guest whose console keeps the size it had, which
 * is what `frame_shape` reports).
 *
 * The blur it hands out is a radius to paint with rather than on or off: it comes up over a few
 * frames, lets go the same way, and a drag that starts while the blur is still fading picks the
 * radius up where it stands, so no drag ever makes it jump.
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
    /* the corner of the window is being held right now */
    pub(crate) held: bool,
    /* the size the guest was last told to be */
    pub(crate) asked: Option<WindowSize>,
    /* and the moment a frame at that size arrived, which is the guest being back */
    pub(crate) back: Option<Instant>,
}

impl Resize {
    pub(crate) fn new() -> Self {
        Self {
            size: None,
            since: None,
            drag: None,
            held: false,
            asked: None,
            back: None,
        }
    }

    /* the guest has been asked for this size: the blur stays up until it is back at it */
    pub(crate) fn asking(&mut self, size: WindowSize) {
        self.asked = Some(size);
        self.back = None;
    }

    /*
     * A frame arrived, this big.  One the size that was asked for is the guest back, and the
     * blur has nothing left to hide; any other size is a guest still on its way (or one that
     * will not come), which leaves the ceiling to clear it.
     */
    pub(crate) fn arrived(&mut self, width: u32, height: u32, now: Instant) {
        let Some(asked) = self.asked else {
            return;
        };
        if (asked.width, asked.height) != (width, height) || self.back.is_some() {
            return;
        }
        self.back = Some(now);
    }

    /*
     * The window is this size now.  A size it moves to while the blur is still up is the same
     * drag; one that arrives after it has cleared starts a new one, which is what the blur
     * comes up from; and the size it opened at is no drag at all.
     */
    pub(crate) fn changed(&mut self, size: WindowSize, now: Instant, held: bool) {
        let carried = self.level(now);
        if self.held && !held {
            self.since = Some(now);
        }
        self.held = held;
        if self.drag.is_some() && !self.held && self.unchanged_for(now) >= BLUR_HOLD {
            self.drag = None;
        }
        if self.size == Some(size) {
            return;
        }
        if self.size.is_some() {
            self.drag = Some(now - BLUR_FADE.mul_f32(carried));
        }
        self.size = Some(size);
        self.since = Some(now);
        self.asked = None;
        self.back = None;
    }    pub(crate) fn size(&self) -> Option<WindowSize> {
        self.size
    }

    pub(crate) fn dragging(&self, now: Instant) -> bool {
        self.held || (self.drag.is_some() && self.until(now) > now)
    }

    /*
     * The moment the blur has to be off by: the guest's frame at the size it was asked for,
     * once that has arrived, and otherwise the ceiling the hold is allowed.
     */
    fn until(&self, now: Instant) -> Instant {
        let ceiling = self.since.map(|since| since + BLUR_HOLD).unwrap_or(now);
        let Some(back) = self.back else {
            return ceiling;
        };
        (back + BLUR_FADE).min(ceiling)
    }

    /* what to blur the frame by, or nothing when there is no drag to cover */
    pub(crate) fn blur(&self, now: Instant) -> Option<f32> {
        self.dragging(now)
            .then(|| BLUR_RADIUS * self.level(now))
    }

    /* how much of the blur is out, from 0 when a drag starts to 1 while it is moving */
    pub(crate) fn level(&self, now: Instant) -> f32 {
        if !self.dragging(now) {
            return 0.0;
        }
        let Some(drag) = self.drag else {
            return 0.0;
        };
        let left = if self.held {
            BLUR_HOLD
        } else {
            self.until(now).saturating_duration_since(now)
        };
        let up = now.saturating_duration_since(drag);

        (up.min(left).as_secs_f32() / BLUR_FADE.as_secs_f32()).min(1.0)
    }

    pub(crate) fn landed(&self, now: Instant) -> bool {
        self.size.is_some() && !self.held && self.unchanged_for(now) >= RESIZE_SETTLE
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
        resize.changed(size, now, false);
        if resize.landed(now) && *asked != Some(size) {
            *asked = Some(size);
            return true;
        }
        false
    }

    /*
     * The hold is the guest's own return, not a duration: the frame at the size it was asked
     * for takes the blur down as soon as it comes, and a frame of any other size leaves the
     * ceiling to do it.
     */
    #[test]
    fn the_frame_for_the_size_we_asked_for_lets_the_blur_go() {
        let size = |width: f32| window_size(gpui::size(gpui::px(width), gpui::px(600.0)), 1.0);
        let start = Instant::now();
        let mut resize = Resize::new();

        let mut asked = None;
        ask(&mut resize, &mut asked, size(800.0), start);
        resize.changed(size(900.0), start, false);
        let asked_at = start + RESIZE_SETTLE;
        resize.asking(size(900.0));
        assert_eq!(resize.blur(asked_at), Some(BLUR_RADIUS));

        /* the guest came back at the size it was asked for */
        resize.arrived(900, 600, asked_at + Duration::from_millis(120));
        assert_eq!(
            resize.blur(asked_at + Duration::from_millis(120)),
            Some(BLUR_RADIUS)
        );
        let going = resize
            .blur(asked_at + Duration::from_millis(120) + BLUR_FADE / 2)
            .unwrap();
        assert!(going > 0.0 && going < BLUR_RADIUS, "the blur did not let go");
        assert_eq!(
            resize.blur(asked_at + Duration::from_millis(120) + BLUR_FADE),
            None,
            "the blur outlasted the frame it was waiting for"
        );
        assert!(
            asked_at + Duration::from_millis(120) + BLUR_FADE < start + BLUR_HOLD,
            "this test does not tell the ceiling from the frame"
        );
    }

    #[test]
    fn a_frame_of_another_size_leaves_the_ceiling_to_clear_the_blur() {
        let size = |width: f32| window_size(gpui::size(gpui::px(width), gpui::px(600.0)), 1.0);
        let start = Instant::now();
        let mut resize = Resize::new();

        let mut asked = None;
        ask(&mut resize, &mut asked, size(800.0), start);
        resize.changed(size(900.0), start, false);
        resize.asking(size(900.0));

        /* a guest whose console kept the size it had: the ceiling is all there is */
        resize.arrived(1390, 600, start + BLUR_HOLD / 2);
        assert_eq!(resize.blur(start + BLUR_HOLD / 2), Some(BLUR_RADIUS));
        assert_eq!(resize.blur(start + BLUR_HOLD), None);
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

        resize.changed(size(804.0), start, false);
        assert!(resize.dragging(start));
        assert!(
            resize.landed(start + RESIZE_SETTLE),
            "the guest has to be asked while the blur is still up"
        );

        /* a size the window repeats holds nothing: the drag has not moved on */
        resize.changed(size(804.0), start + BLUR_HOLD / 2, false);
        assert!(resize.dragging(start + BLUR_HOLD - Duration::from_millis(1)));
        assert!(!resize.dragging(start + BLUR_HOLD));

        /* a size it moves on to holds it from the moment it moved */
        resize.changed(size(900.0), start + BLUR_HOLD / 2, false);
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

        resize.changed(size(804.0), start, false);
        assert_eq!(resize.blur(start), Some(0.0));
        let coming_up = resize.blur(start + BLUR_FADE / 2).unwrap();
        assert!(coming_up > 0.0 && coming_up < BLUR_RADIUS);
        assert_eq!(resize.blur(start + BLUR_FADE), Some(BLUR_RADIUS));

        /* the ramp is only at the ends: a drag still moving is blurred the whole way */
        resize.changed(size(900.0), start + BLUR_HOLD / 2, false);
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

    #[test]
    fn a_drag_that_starts_while_the_blur_fades_picks_it_up_where_it_stands() {
        let size = |width: f32| window_size(gpui::size(gpui::px(width), gpui::px(600.0)), 1.0);
        let start = Instant::now();
        let mut resize = Resize::new();

        resize.changed(size(800.0), start, false);
        resize.changed(size(900.0), start + Duration::from_millis(50), true);
        let let_go = start + BLUR_HOLD;
        resize.changed(size(900.0), let_go, false);

        let fading = let_go + BLUR_HOLD - BLUR_FADE / 2;
        let before = resize.blur(fading).unwrap();
        assert!(before > 0.0 && before < BLUR_RADIUS);

        resize.changed(size(940.0), fading, true);
        let after = resize.blur(fading).unwrap();
        assert!(
            (after - before).abs() < 0.01,
            "a new drag jumped the blur from {before} to {after}"
        );
        assert_eq!(resize.blur(fading + BLUR_FADE), Some(BLUR_RADIUS));
    }

    #[test]
    fn a_corner_held_asks_for_nothing_until_it_is_let_go() {
        let size = |width: f32| window_size(gpui::size(gpui::px(width), gpui::px(600.0)), 1.0);
        let start = Instant::now();
        let mut resize = Resize::new();

        let mut asked = None;
        ask(&mut resize, &mut asked, size(800.0), start);

        /* the corner is grabbed and dragged, then held still with the size it reached */
        resize.changed(size(804.0), start + Duration::from_millis(50), true);
        resize.changed(size(900.0), start + Duration::from_millis(100), true);
        let held = start + BLUR_HOLD * 4;
        assert!(
            !resize.landed(held),
            "a corner still held asked for a resize"
        );
        assert_eq!(
            resize.blur(held),
            Some(BLUR_RADIUS),
            "the blur has to stay up while the corner is held"
        );

        /* let go, and the size it was let go at is what the guest is asked for */
        let let_go = held;
        resize.changed(size(900.0), let_go, false);
        assert!(!resize.landed(let_go));
        assert!(resize.landed(let_go + RESIZE_SETTLE));
        assert_eq!(resize.blur(let_go), Some(BLUR_RADIUS));
        assert_eq!(resize.blur(let_go + BLUR_HOLD), None);
    }
}
