use gpui::{prelude::*, size, App, Bounds, WindowBounds, WindowOptions};
use gpui_platform::application;
use std::{env, path::PathBuf};
use crate::app::Frame;
use crate::geometry::DISPLAY_MAX;
use crate::grab::grab;
use crate::host::host_display_max;

mod app;
mod bridge;
mod geometry;
mod grab;
mod host;
mod input;
mod paths;
mod qemu;
mod resize;
mod settings;

#[allow(non_snake_case)]
mod dbusDisplay;
#[allow(non_snake_case)]
mod dbusSession;
#[allow(non_snake_case)]
mod frameDump;
#[allow(non_snake_case)]
mod frameRender;
#[allow(non_snake_case)]
mod frameView;
#[allow(non_snake_case)]
mod guestSurface;
#[allow(non_snake_case)]
mod mouseButtons;
mod panel;
#[allow(non_snake_case)]
mod surfacePort;
#[allow(non_snake_case)]
mod surfaceRing;

fn main() {
    if let Some(path) = env::var_os("TRY_GRAB") {
        grab(PathBuf::from(path));
        return;
    }
    application().run(|cx: &mut App| {
        let _ = DISPLAY_MAX.set(host_display_max(cx));
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let bounds = Bounds::centered(None, size(gpui::px(1280.0), gpui::px(720.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let frame = cx.new(Frame::new);
                let focus = frame.read(cx).focus.clone();
                focus.focus(window, cx);
                frame
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
