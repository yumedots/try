use gpui::{
    Context, FocusHandle, Keystroke, Modifiers, ModifiersChangedEvent, MouseButton, Pixels,
    Point, RenderImage, ScrollDelta, Size,
};
use image::{Frame as ImageFrame, RgbaImage};
use smallvec::SmallVec;
use std::{env, sync::{mpsc::Receiver, Arc, Mutex}, time::{Duration, Instant}};
use crate::bridge::{spawn_bridge, start_bridge, Bridge, Event};
use crate::dbusSession::LAST_FRAME;
use crate::frameDump::dump_frame;
use crate::geometry::{frame_shape, guest_position, WindowSize};
use crate::guestSurface::GuestSurface;
use crate::input::{
    is_settings_toggle, keycode, ALT, CAPS_LOCK, CONTROL, Input, SHIFT, SUPER,
};
use crate::settings::{read_settings, write_settings, Settings};

pub(crate) struct Frame {
    pub(crate) bridge: Option<Bridge>,
    pub(crate) pending: Option<Receiver<Result<Bridge, String>>>,
    pub(crate) settings: Settings,
    pub(crate) settings_open: bool,
    pub(crate) focus: FocusHandle,
    pub(crate) surface: (u32, u32),
    pub(crate) held: Modifiers,
    pub(crate) capslock: bool,
    pub(crate) status: Option<String>,
    pub(crate) image: Option<Arc<RenderImage>>,
    pub(crate) ready: bool,
    pub(crate) error: Option<String>,
    pub(crate) guest: Option<GuestSurface>,
    pub(crate) surface_demo: bool,
    pub(crate) wanted: Option<WindowSize>,
    pub(crate) shape_note: Option<String>,
}

impl Frame {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let settings = read_settings();
        let (bridge, error) = match start_bridge() {
            Ok(bridge) => (Some(bridge), None),
            Err(error) => (None, Some(error)),
        };
        cx.observe_keystrokes(|frame, event, _, cx| {
            let keystroke = &event.keystroke;
            if is_settings_toggle(keystroke) {
                frame.settings_open = !frame.settings_open;
                cx.notify();
                return;
            }
            if frame.settings_open && keystroke.key == "escape" {
                frame.settings_open = false;
                cx.notify();
            }
        })
        .detach();
        cx.spawn(async move |this, cx| loop {
            this.update(cx, |frame, cx| {
                frame.poll();
                cx.notify();
            })
            .ok();
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
        })
        .detach();
        Self {
            bridge,
            pending: None,
            settings,
            settings_open: false,
            focus: cx.focus_handle(),
            surface: (0, 0),
            held: Modifiers::default(),
            capslock: false,
            status: None,
            image: None,
            ready: false,
            error,
            guest: None,
            surface_demo: env::var_os("TRY_SURFACE").is_some(),
            wanted: None,
            shape_note: None,
        }
    }

    pub(crate) fn send(&self, input: Input) {
        if let Some(bridge) = &self.bridge {
            let _ = bridge.input.send(input);
        }
    }

    pub(crate) fn sync_modifiers(&mut self, modifiers: Modifiers) {
        for (before, after, code) in [
            (self.held.shift, modifiers.shift, SHIFT),
            (self.held.control, modifiers.control, CONTROL),
            (self.held.alt, modifiers.alt, ALT),
            (self.held.platform, modifiers.platform, SUPER),
        ] {
            if after != before {
                self.send(Input::Key { code, down: after });
            }
        }
        self.held = modifiers;
    }

    pub(crate) fn key(&mut self, keystroke: &Keystroke, down: bool) {
        if self.settings_open || (down && is_settings_toggle(keystroke)) {
            return;
        }
        if down {
            self.sync_modifiers(keystroke.modifiers);
        }
        if let Some(code) = keycode(&keystroke.key) {
            self.send(Input::Key { code, down });
        }
    }

    pub(crate) fn modifiers_changed(&mut self, event: &ModifiersChangedEvent) {
        self.sync_modifiers(event.modifiers);
        if event.capslock.on != self.capslock {
            self.capslock = event.capslock.on;
            self.send(Input::Key {
                code: CAPS_LOCK,
                down: true,
            });
            self.send(Input::Key {
                code: CAPS_LOCK,
                down: false,
            });
        }
    }

    pub(crate) fn pointer(&mut self, position: Point<Pixels>, viewport: Size<Pixels>) {
        if self.settings_open {
            return;
        }
        if let Some((x, y)) = guest_position(self.surface, viewport, position) {
            self.send(Input::Move { x, y });
        }
    }

    pub(crate) fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        position: Point<Pixels>,
        viewport: Size<Pixels>,
    ) {
        if self.settings_open {
            return;
        }
        let button = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            _ => return,
        };
        if down {
            if let Some((x, y)) = guest_position(self.surface, viewport, position) {
                self.send(Input::Move { x, y });
            }
        }
        self.send(Input::Button { button, down });
    }

    pub(crate) fn wheel(&mut self, delta: ScrollDelta) {
        if self.settings_open {
            return;
        }
        let up = match delta {
            ScrollDelta::Lines(point) => point.y,
            ScrollDelta::Pixels(point) => point.y.to_f64() as f32,
        };
        if up != 0.0 {
            self.send(Input::Wheel { up: up > 0.0 });
        }
    }

    pub(crate) fn poll(&mut self) {
        let events: Vec<Event> = self
            .bridge
            .as_ref()
            .map(|bridge| bridge.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Frame {
                    width,
                    height,
                    pixels,
                } => {
                    let now = Instant::now();
                    let slot = LAST_FRAME.get_or_init(|| Mutex::new(None));
                    let mut last = slot.lock().unwrap();
                    if let Some(previous) = *last {
                        println!(
                            "tryfps: {width}x{height} after {:?}",
                            now.duration_since(previous)
                        );
                    }
                    *last = Some(now);
                    drop(last);
                    dump_frame(width, height, &pixels);
                    self.surface = (width, height);
                    self.image = RgbaImage::from_raw(width, height, pixels).map(|buffer| {
                        Arc::new(RenderImage::new(SmallVec::from_elem(
                            ImageFrame::new(buffer),
                            1,
                        )))
                    });
                    self.error = None;
                    let note = frame_shape(self.surface, self.wanted);
                    if note != self.shape_note {
                        if let Some(note) = &note {
                            println!("{note}");
                        }
                        self.shape_note = note;
                    }
                }
                Event::Ready => self.ready = true,
                Event::Error(error) => self.error = Some(error),
            }
        }
        if let Some(result) = self
            .pending
            .as_ref()
            .and_then(|pending| pending.try_recv().ok())
        {
            self.pending = None;
            self.status = None;
            match result {
                Ok(bridge) => self.bridge = Some(bridge),
                Err(error) => {
                    self.ready = false;
                    self.error = Some(error);
                }
            }
        }
    }

    pub(crate) fn apply(&mut self, cx: &mut Context<Self>, reset: bool) {
        write_settings(&self.settings);
        self.restart(cx, reset);
    }

    pub(crate) fn restart(&mut self, cx: &mut Context<Self>, reset: bool) {
        self.bridge = None;
        self.pending = Some(spawn_bridge(reset));
        self.image = None;
        self.ready = false;
        self.error = None;
        self.status = Some(
            if reset {
                "rebuilding the guest disk"
            } else {
                "restarting the guest"
            }
            .to_string(),
        );
        cx.notify();
    }

}
