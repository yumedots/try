use gpui::{div, rgb, AnyElement, ClickEvent, Context, Div, Pixels, Size, Stateful};
use gpui::IntoElement;
use gpui::prelude::*;
use crate::app::Frame;
use crate::guestSurface::{guestSurface, GuestSurface};
use crate::host::host_cores;
use crate::settings::{memory_index, MEMORY};

impl Frame {
    pub(crate) fn demo(&mut self, viewport: Size<Pixels>, scale: f32) -> AnyElement {
        let size = (
            (viewport.width.to_f64() * scale as f64).round().max(1.0) as u32,
            (viewport.height.to_f64() * scale as f64).round().max(1.0) as u32,
        );
        if self.guest.as_ref().map(GuestSurface::size) != Some(size) {
            self.guest = GuestSurface::new(size.0, size.1).ok();
            if let Some(guest) = &self.guest {
                guest.fill();
                println!(
                    "try: pixel buffer {}x{} iosurface {}",
                    size.0,
                    size.1,
                    guest.io_surface_id()
                );
            }
        }
        let Some(guest) = &self.guest else {
            return div()
                .flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(rgb(0xff7777))
                .child("no pixel buffer")
                .into_any_element();
        };
        guestSurface(guest)
            .w(viewport.width)
            .h(viewport.height)
            .into_any_element()
    }

    pub(crate) fn settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cores = self.settings.cores;
        let memory = self.settings.mem.clone();
        let audio = self.settings.audio;
        let status = self.status.clone().unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .p_8()
            .bg(rgb(0x121212))
            .text_color(rgb(0xdddddd))
            .child(div().text_lg().child("try settings"))
            .child(button("cores-down", "-").on_click(cx.listener(
                |frame, _: &ClickEvent, _, cx| {
                    frame.settings.cores = frame.settings.cores.saturating_sub(1).max(1);
                    frame.apply(cx, false);
                },
            )))
            .child(div().flex_none().child(format!("{cores} cores")))
            .child(
                button("cores-up", "+").on_click(cx.listener(|frame, _: &ClickEvent, _, cx| {
                    frame.settings.cores = (frame.settings.cores + 1).min(host_cores());
                    frame.apply(cx, false);
                })),
            )
            .child(
                button("mem-down", "-").on_click(cx.listener(|frame, _: &ClickEvent, _, cx| {
                    let index = memory_index(&frame.settings.mem).saturating_sub(1);
                    frame.settings.mem = MEMORY[index].to_string();
                    frame.apply(cx, false);
                })),
            )
            .child(div().flex_none().child(format!("{memory} ram")))
            .child(
                button("mem-up", "+").on_click(cx.listener(|frame, _: &ClickEvent, _, cx| {
                    let index = (memory_index(&frame.settings.mem) + 1).min(MEMORY.len() - 1);
                    frame.settings.mem = MEMORY[index].to_string();
                    frame.apply(cx, false);
                })),
            )
            .child(
                button("audio", if audio { "audio on" } else { "audio off" }).on_click(
                    cx.listener(|frame, _: &ClickEvent, _, cx| {
                        frame.settings.audio = !frame.settings.audio;
                        frame.apply(cx, false);
                    }),
                ),
            )
            .child(
                button("reset", "reset guest")
                    .on_click(cx.listener(|frame, _: &ClickEvent, _, cx| frame.apply(cx, true))),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x888888))
                    .child(if status.is_empty() {
                        "saved, applied on restart".to_string()
                    } else {
                        status
                    }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x666666))
                    .child("cmd-, to close"),
            )
    }
}

pub(crate) fn button(id: &'static str, label: impl Into<String>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_2()
        .py_1()
        .rounded_md()
        .bg(rgb(0x333333))
        .text_color(rgb(0xeeeeee))
        .cursor_pointer()
        .child(label.into())
}
