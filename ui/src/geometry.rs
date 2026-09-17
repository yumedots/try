use gpui::{Pixels, Point, Size};
use std::sync::OnceLock;

pub(crate) const GUEST_MAX_DISPLAY: (u32, u32) = (2560, 1440);

pub(crate) static DISPLAY_MAX: OnceLock<(u32, u32)> = OnceLock::new();

pub(crate) fn guest_position(
    surface: (u32, u32),
    viewport: Size<Pixels>,
    position: Point<Pixels>,
) -> Option<(u32, u32)> {
    let (width, height) = surface;
    if width == 0 || height == 0 {
        return None;
    }
    let (surface_width, surface_height) = (width as f64, height as f64);
    let (viewport_width, viewport_height) = (
        viewport.width.to_f64().max(1.0),
        viewport.height.to_f64().max(1.0),
    );
    /* the frame covers the window: one scale, with what overflows cropped evenly */
    let scale = (viewport_width / surface_width).max(viewport_height / surface_height);
    let x = (position.x.to_f64() + (surface_width * scale - viewport_width) / 2.0) / scale;
    let y = (position.y.to_f64() + (surface_height * scale - viewport_height) / 2.0) / scale;
    Some((
        x.clamp(0.0, surface_width - 1.0) as u32,
        y.clamp(0.0, surface_height - 1.0) as u32,
    ))
}

/*
 * What the guest sent against what the window asked for, so a screen that does not
 * fill the window can be told from one that is a resize behind.
 */
pub(crate) fn frame_shape(surface: (u32, u32), want: Option<WindowSize>) -> Option<String> {
    let size = want?;
    ((surface.0, surface.1) != (size.width, size.height)).then(|| {
        format!(
            "guest sent {}x{} for a {}x{} window",
            surface.0, surface.1, size.width, size.height
        )
    })
}

pub(crate) fn display_max(argv: &[String]) -> (u32, u32) {
    for entry in argv {
        if let Some(rest) = entry.split("xres=").nth(1) {
            let width = rest.split(',').next().and_then(|w| w.parse().ok());
            let height = rest
                .split("yres=")
                .nth(1)
                .and_then(|h| h.split(',').next())
                .and_then(|h| h.parse().ok());
            if let (Some(width), Some(height)) = (width, height) {
                return (width, height);
            }
        }
    }
    GUEST_MAX_DISPLAY
}

pub(crate) fn clamp_to_display(size: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let scale = (max.0 as f64 / size.0 as f64)
        .min(max.1 as f64 / size.1 as f64)
        .min(1.0);
    (
        ((size.0 as f64 * scale) as u32).max(1),
        ((size.1 as f64 * scale) as u32).max(1),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WindowSize {
    pub(crate) width_mm: u16,
    pub(crate) height_mm: u16,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/*
 * The guest reads the window twice over from the EDID we send: the pixel size asks for
 * that many guest pixels, and the physical size is how it learns how many of them are
 * one of the window's points.  Describing the window at 110 dpi puts a 1x, 1.5x and 2x
 * host on the guest's 1, 1.5 and 2 scales, and in whole centimetres because that is the
 * only precision a base EDID carries.  The pixel size is rounded down to whole logical
 * pixels so the guest never has to round a fractional one.
 */
pub(crate) const LOGICAL_DPI: f64 = 110.0;

pub(crate) const MM_PER_CM: u16 = 10;

pub(crate) const MAX_CM: f64 = 255.0;

pub(crate) fn window_size(viewport: Size<Pixels>, scale: f32) -> WindowSize {
    let factor = if scale > 0.0 { scale as f64 } else { 1.0 };
    let divisor = if factor > 1.75 {
        2
    } else if factor > 1.25 {
        3
    } else {
        1
    };
    let (width, height) = (viewport.width.to_f64(), viewport.height.to_f64());
    let cm = |value: f64| (value * 2.54 / LOGICAL_DPI).round().clamp(1.0, MAX_CM) as u16;
    let px = |value: f64| {
        let pixels = (value * factor) as u32;
        (pixels / divisor * divisor).max(divisor)
    };
    WindowSize {
        width_mm: cm(width) * MM_PER_CM,
        height_mm: cm(height) * MM_PER_CM,
        width: px(width),
        height: px(height),
    }
}

/*
 * The guest builds its framebuffer for the size it is first told and the kernel never
 * grows that on its own, so the guest opens at the largest size the device advertises and
 * takes the window's own size once its display service is up.  Every resize after that is
 * a size inside the framebuffer, which is what lets the guest's console follow the window
 * and not just its compositor.
 */
pub(crate) fn opening_size(wanted: WindowSize, display: (u32, u32), ready: bool) -> WindowSize {
    if ready {
        return wanted;
    }
    WindowSize {
        width: display.0,
        height: display.1,
        ..wanted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_is_one_guest_pixel_per_device_pixel_and_fills_the_window() {
        let surface = (2560, 1440);
        let viewport = gpui::size(gpui::px(1280.0), gpui::px(720.0));
        assert_eq!(
            guest_position(
                surface,
                viewport,
                gpui::point(gpui::px(1280.0), gpui::px(720.0))
            ),
            Some((2559, 1439))
        );
        /* a guest of another shape covers the window, cropped evenly, never squeezed */
        let capped = (1390, 1440);
        let wider = gpui::size(gpui::px(1000.0), gpui::px(720.0));
        assert_eq!(
            guest_position(
                capped,
                wider,
                gpui::point(gpui::px(1000.0), gpui::px(720.0))
            ),
            Some((1389, 1220))
        );
        /* the window's middle is the guest's middle whatever the shapes are */
        assert_eq!(
            guest_position(capped, wider, gpui::point(gpui::px(500.0), gpui::px(360.0))),
            Some((695, 720))
        );
        assert!(frame_shape(capped, Some(window_size(viewport, 2.0))).is_some());
    }

    #[test]
    fn the_guest_opens_at_the_device_size_and_then_follows_the_window() {
        let wanted = window_size(gpui::size(gpui::px(1000.0), gpui::px(600.0)), 2.0);
        let opened = opening_size(wanted, (2560, 1440), false);
        assert_eq!((opened.width, opened.height), (2560, 1440));
        assert_eq!(
            (opened.width_mm, opened.height_mm),
            (wanted.width_mm, wanted.height_mm)
        );
        assert_eq!(opening_size(wanted, (2560, 1440), true), wanted);
    }

    #[test]
    fn the_request_keeps_the_window_shape_within_the_device_max() {
        assert_eq!(clamp_to_display((2000, 1200), (2560, 1440)), (2000, 1200));
        assert_eq!(clamp_to_display((3024, 1964), (2560, 1440)), (2217, 1440));
        assert_eq!(clamp_to_display((1512, 1514), (2560, 1440)), (1438, 1440));
        assert_eq!(
            display_max(&[
                "-device".into(),
                "virtio-gpu-gl-pci,xres=2560,yres=1440".into()
            ]),
            (2560, 1440)
        );
    }

    #[test]
    fn the_window_reaches_the_guest_in_its_pixels_and_in_its_own_scale() {
        let viewport = gpui::size(gpui::px(756.0), gpui::px(502.0));
        assert_eq!(
            window_size(viewport, 2.0),
            WindowSize {
                width_mm: 170,
                height_mm: 120,
                width: 1512,
                height: 1004,
            }
        );
        assert_eq!(
            window_size(viewport, 1.0),
            WindowSize {
                width_mm: 170,
                height_mm: 120,
                width: 756,
                height: 502,
            }
        );
    }

    #[test]
    fn the_desktop_keeps_the_size_of_the_window_at_any_size() {
        /*
         * Two things have to hold for the guest's desktop to come out as the size of the
         * window at every size it is dragged to: the density the millimetres imply has to
         * fall in the band the guest reads the host's scale factor from, and the pixels
         * have to divide into whole logical ones at that scale - where the window's own
         * points are what the logical size comes out as.  The bands mirror the guest's
         * decision (1x, 1.5x or 2x), so the two cannot drift apart quietly.
         */
        for (factor, below, above) in [
            (1.0f64, 0.0, 140.0),
            (1.5, 140.0, 200.0),
            (2.0, 200.0, f64::MAX),
        ] {
            for (width, height) in [
                (1512.0, 982.0),
                (756.0, 491.0),
                (1200.0, 700.0),
                (320.0, 480.0),
            ] {
                let viewport = gpui::size(gpui::px(width as f32), gpui::px(height as f32));
                let size = window_size(viewport, factor as f32);
                let diagonal_mm =
                    ((size.width_mm as f64).powi(2) + (size.height_mm as f64).powi(2)).sqrt();
                let density = (f64::from(size.width).powi(2) + f64::from(size.height).powi(2))
                    .sqrt()
                    / (diagonal_mm / 25.4);
                assert!(
                    density > below && density <= above,
                    "{width}x{height} pt at {factor}x: the guest reads {density} dpi"
                );
                let logical = f64::from(size.width) / factor;
                assert!(
                    (logical - width).abs() < 1.0,
                    "{width}x{height} pt at {factor}x: {} px is {logical} logical px",
                    size.width
                );
            }
        }
    }

    #[test]
    fn pointer_maps_through_the_cover() {
        let surface = (2560, 1440);
        let same_aspect = gpui::size(gpui::px(1280.0), gpui::px(720.0));
        assert_eq!(
            guest_position(
                surface,
                same_aspect,
                gpui::point(gpui::px(0.0), gpui::px(0.0))
            ),
            Some((0, 0))
        );
        assert_eq!(
            guest_position(
                surface,
                same_aspect,
                gpui::point(gpui::px(1280.0), gpui::px(720.0))
            ),
            Some((2559, 1439))
        );
        /*
         * A window of another shape shows the middle of the guest: the overflow is
         * cropped in half on each side, so the window's centre is the guest's centre.
         */
        let wider = gpui::size(gpui::px(1600.0), gpui::px(720.0));
        assert_eq!(
            guest_position(surface, wider, gpui::point(gpui::px(160.0), gpui::px(0.0))),
            Some((256, 144))
        );
        assert_eq!(
            guest_position(
                surface,
                wider,
                gpui::point(gpui::px(800.0), gpui::px(360.0))
            ),
            Some((1280, 720))
        );
        let taller = gpui::size(gpui::px(1280.0), gpui::px(900.0));
        assert_eq!(
            guest_position(surface, taller, gpui::point(gpui::px(0.0), gpui::px(450.0))),
            Some((256, 720))
        );
        assert_eq!(
            guest_position((0, 0), taller, gpui::point(gpui::px(0.0), gpui::px(0.0))),
            None
        );
    }

    /* what the bridge does with the size the window is at: ask the guest, once, when it holds still */
}
