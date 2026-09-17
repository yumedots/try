use gpui::{
    div, img, px, rgb, Context, ImageSource, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Render,
    ScrollWheelEvent, Window,
};
use std::time::Instant;
use gpui::IntoElement;
use gpui::prelude::*;
use crate::app::Frame;
use crate::geometry::window_size;

impl Render for Frame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let scale = window.scale_factor();
        /*
         * The window's size leaves for the guest on every frame, whatever it is showing:
         * the guest's boot mode is the whole screen, so holding it back until the first
         * frame arrives would letterbox the guest inside a smaller window.
         */
        let wanted = window_size(viewport, scale);
        self.wanted = Some(wanted);
        let blur = self
            .bridge
            .as_ref()
            .and_then(|bridge| bridge.resized(wanted, Instant::now()));
        let content = if self.settings_open {
            self.settings(cx).into_any_element()
        } else if self.surface_demo {
            self.demo(viewport, scale)
        } else {
            match (&self.image, &self.error, self.ready) {
                (Some(image), _, true) => {
                    /*
                     * The frame covers the window: drawn at the window's size with its
                     * aspect kept and the overflow cropped evenly off both sides, so the
                     * guest fills the window without ever being squeezed, and a guest that
                     * is the size of the window is drawn one guest pixel per device pixel
                     * with nothing cropped at all.  `frame_shape` reports the case where a
                     * resize never arrived, which is a guest that did not adopt it.
                     */
                    img(ImageSource::Render(image.clone()))
                        .object_fit(ObjectFit::Cover)
                        .w(viewport.width)
                        .h(viewport.height)
                        .into_any_element()
                }
                (_, Some(error), _) => div()
                    .flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0xff7777))
                    .child(error.clone())
                    .into_any_element(),
                _ => div()
                    .flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0x111111))
                    .text_color(rgb(0xffffff))
                    .child("waiting for Linux graphical session")
                    .into_any_element(),
            }
        };
        let mut root = div()
            .id("frame")
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |frame, event: &KeyDownEvent, _, _| {
                frame.key(&event.keystroke, true)
            }))
            .on_key_up(cx.listener(move |frame, event: &KeyUpEvent, _, _| {
                frame.key(&event.keystroke, false)
            }))
            .on_modifiers_changed(
                cx.listener(move |frame, event: &ModifiersChangedEvent, _, _| {
                    frame.modifiers_changed(event)
                }),
            )
            .on_mouse_move(cx.listener(move |frame, event: &MouseMoveEvent, _, _| {
                frame.pointer(event.position, viewport)
            }))
            .on_scroll_wheel(
                cx.listener(move |frame, event: &ScrollWheelEvent, _, _| frame.wheel(event.delta)),
            );
        if let Some(blur) = blur {
            root = root.blur(px(blur));
        }
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            root = root
                .on_mouse_down(
                    button,
                    cx.listener(move |frame, event: &MouseDownEvent, _, _| {
                        frame.button(button, true, event.position, viewport)
                    }),
                )
                .on_mouse_up(
                    button,
                    cx.listener(move |frame, event: &MouseUpEvent, _, _| {
                        frame.button(button, false, event.position, viewport)
                    }),
                );
        }
        root.child(content)
    }
}
