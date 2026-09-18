use std::sync::{atomic::{AtomicU64, Ordering}, Arc, Mutex};
use crate::guestSurface::GuestSurface;
use crate::surfacePort::publish;

/*
 * The frames the console reads back go into a surface of ours instead of over the socket.
 * There is more than one because the window draws one of them: the console writes into the
 * next while the window still holds the last, and the one after that is the slack between
 * them, which is what keeps a frame from being drawn as it is being written.
 *
 * The ring is only advanced when the name has actually been handed over, so a burst of
 * frames arriving before the hand-off cannot walk it past a surface the console still
 * writes into.
 */
pub(crate) const RING: usize = 3;

static NAMES: AtomicU64 = AtomicU64::new(0);

pub(crate) type SharedRing = Arc<Mutex<Ring>>;

/*
 * The ring is made on the display bridge and drawn from the window.  A pixel buffer is a
 * refcounted handle to a surface rather than the surface itself, and the mutex is what
 * keeps one thread out of the frame the other is using, so it travels.
 */
unsafe impl Send for Ring {}

#[derive(Default)]
pub(crate) struct Ring {
    surfaces: Vec<GuestSurface>,
    names: Vec<String>,
    width: u32,
    height: u32,
    /* the surface the console holds, or none before it has been given one */
    handed: Option<usize>,
    /* the last surface a frame landed in, which is what the window draws */
    ready: Option<usize>,
    /*
     * The last frame of the ring before this one.  The console is asked for a size
     * before it is at that size, so there is a while where no surface of the new ring
     * has been written into: the window goes on drawing the frame it has, stretched to
     * the window and behind the blur, rather than a picture that arrives some other way.
     */
    shown: Option<GuestSurface>,
    /* the console is owed a name */
    wanted: bool,
}

impl Ring {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub(crate) fn stride(&self) -> u32 {
        self.ready().map(GuestSurface::stride).unwrap_or(0)
    }

    pub(crate) fn ready(&self) -> Option<&GuestSurface> {
        self.ready
            .map(|index| &self.surfaces[index])
            .or(self.shown.as_ref())
    }

    /* the frame the window would be drawing, as the socket would have sent it */
    pub(crate) fn pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        let surface = self.ready()?;
        let (width, height) = surface.size();

        Some((width, height, surface.read()?))
    }

    /* a frame landed in the surface the console holds: it is what the window shows now */
    pub(crate) fn landed(&mut self) {
        if let Some(handed) = self.handed {
            self.ready = Some(handed);
            self.shown = None;
            self.wanted = true;
        }
    }

    /*
     * The console is about to be this size, so the ring is made this size first: the
     * frames it sends on its way to the new mode are the ones that would otherwise have
     * come over the socket, because a surface of ours was the size it had stopped being.
     */
    pub(crate) fn rebuild(&mut self, width: u32, height: u32) {
        if self.size() != (width, height) {
            self.build(width, height);
        }
    }

    /* the name the console must be given, or nothing when it already holds the right one */
    pub(crate) fn take_request(&mut self) -> Option<String> {
        if !self.wanted || self.surfaces.len() != RING {
            return None;
        }
        self.wanted = false;
        self.handed = Some(match self.handed {
            None => 0,
            Some(index) => (index + 1) % RING,
        });
        Some(self.names[self.handed.unwrap()].clone())
    }

    /*
     * A frame still came over the socket, which it does whenever the console has no surface
     * of ours or one that is not this size: a ring is made for it, and the frame itself is
     * already on its way to the window as an image.
     */
    pub(crate) fn software_frame(&mut self, width: u32, height: u32) {
        if self.surfaces.len() == RING && self.size() == (width, height) {
            return;
        }
        self.build(width, height);
    }

    #[cfg(test)]
    fn handed_id(&self) -> u32 {
        self.handed
            .map(|index| self.surfaces[index].io_surface_id())
            .unwrap_or(0)
    }

    fn build(&mut self, width: u32, height: u32) {
        let mut surfaces = Vec::with_capacity(RING);
        let mut names = Vec::with_capacity(RING);
        let shown = self.ready.and_then(|index| self.surfaces.get(index)).cloned();

        for _ in 0..RING {
            let name = name();
            match own(width, height, &name) {
                Ok(surface) => {
                    surfaces.push(surface);
                    names.push(name);
                }
                Err(error) => {
                    println!("try: the console keeps its frames: {error}");
                    return;
                }
            }
        }
        self.surfaces = surfaces;
        self.names = names;
        self.width = width;
        self.height = height;
        self.handed = None;
        self.ready = None;
        self.shown = shown;
        self.wanted = true;
    }
}

/* a name belongs to the session and outlives the surface, so no two are ever the same */
fn name() -> String {
    format!(
        "try.surface.{}.{}",
        std::process::id(),
        NAMES.fetch_add(1, Ordering::Relaxed)
    )
}

fn own(width: u32, height: u32, name: &str) -> Result<GuestSurface, String> {
    let surface = GuestSurface::new(width, height)?;

    publish(surface.buffer(), name)?;
    Ok(surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(width: u32, height: u32) -> Ring {
        let mut ring = Ring::new();

        ring.software_frame(width, height);
        assert_eq!(ring.size(), (width, height), "the ring was not made");
        ring
    }

    fn shown(ring: &Ring) -> u32 {
        ring.ready().expect("no frame to show").io_surface_id()
    }

    #[test]
    fn the_console_is_given_a_surface_and_then_the_one_after_it() {
        let mut ring = ring(64, 48);
        let first = ring.take_request().expect("the console was given nothing");

        ring.landed();
        let second = ring.take_request().expect("the console was owed the next one");
        assert_ne!(first, second, "the console was given the same surface twice");

        ring.landed();
        let third = ring.take_request().expect("the console was owed the next one");
        assert_ne!(third, first);
        assert_ne!(third, second);
    }

    #[test]
    fn the_window_never_shows_the_surface_the_console_writes_into() {
        let mut ring = ring(64, 48);
        ring.take_request();

        for _ in 0..6 {
            ring.landed();
            assert!(
                ring.take_request().is_some(),
                "the console was not given the next surface"
            );
            assert_ne!(
                shown(&ring),
                ring.handed_id(),
                "the frame on screen is the surface being written into"
            );
        }
    }

    #[test]
    fn frames_that_land_before_the_hand_over_do_not_walk_the_ring_past_it() {
        let mut ring = ring(64, 48);
        ring.take_request();
        ring.landed();

        let frame = shown(&ring);
        ring.landed();
        assert_eq!(
            shown(&ring),
            frame,
            "the ring moved on while the console still wrote into one surface"
        );

        /* the hand-off is what moves it on, and the frame it holds stays up until the next one */
        ring.take_request();
        assert_eq!(shown(&ring), frame);
        ring.landed();
        assert_ne!(shown(&ring), frame, "the frame after the hand-off never landed");
    }

    #[test]
    fn a_frame_at_another_size_is_what_makes_a_new_ring() {
        let mut ring = ring(64, 48);
        ring.take_request();
        ring.landed();
        ring.take_request();

        ring.software_frame(64, 48);
        assert!(
            ring.take_request().is_none(),
            "a frame the ring is already the size of made a new one"
        );

        ring.software_frame(96, 32);
        assert_eq!(ring.size(), (96, 32));
        assert!(
            ring.take_request().is_some(),
            "the new ring was never handed over"
        );
    }

    /*
     * A console at another size cannot write into the ring it has: the ring is made that
     * size before the console's next frame, which is what keeps those frames off the socket.
     */
    #[test]
    fn a_console_at_another_size_is_written_into_a_ring_of_that_size() {
        let mut ring = ring(64, 48);
        let first = ring.take_request().expect("the console was given nothing");
        ring.landed();
        let frame = shown(&ring);

        ring.rebuild(96, 32);
        assert_eq!(ring.size(), (96, 32), "the ask did not make a new ring");
        assert_ne!(
            ring.take_request().expect("the new ring was never handed over"),
            first,
            "the console was handed a surface of the ring it is not being asked to leave"
        );
        assert_eq!(
            ring.ready().expect("no frame to show").size(),
            (64, 48),
            "the window went blank while the guest was on its way to the new mode"
        );

        assert_eq!(shown(&ring), frame);
        ring.landed();
        assert_eq!(
            ring.ready().expect("no frame to show").size(),
            (96, 32),
            "the frame of the new ring is not the one being shown"
        );
    }
}
