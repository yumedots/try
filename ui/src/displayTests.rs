/*
 * The window draws the guest's frame two ways and both have to agree on the colour
 * order: the caller surface where the console read the frame, and the image built from
 * the copy the console had to send over the socket.  A frame drawn through a blurred
 * layer is a third pass over the same pixels.  A mismatch here is not subtle - it is
 * the window flipping between the desktop and the same picture with red and blue
 * exchanged - so each of these checks a saturated colour's channels.
 */

use gpui::{
    point, px, size, Bounds, ContentMask, DevicePixels, Filter, PaintSurface, Scene, ScaledPixels,
};
use gpui_apple::metal_renderer::{Context, MetalRenderer};
use image::Rgba;

use crate::guestSurface::GuestSurface;

const STORED: [u8; 4] = [0x20, 0x40, 0xc0, 0xff];
const SHOWN: [u8; 4] = [0xc0, 0x40, 0x20, 0xff];
const SIDES: u32 = 128;

fn filled() -> GuestSurface {
    let surface = GuestSurface::new(SIDES, SIDES).unwrap();

    for y in 0..SIDES {
        for x in 0..SIDES {
            surface.pixel(x, y, STORED);
        }
    }
    surface
}

fn scene(surface: &GuestSurface, blur: f32) -> Scene {
    let sides = size(px(SIDES as f32), px(SIDES as f32));
    let bounds = Bounds::new(point(px(0.), px(0.)), sides);
    let scaled: Bounds<ScaledPixels> = bounds.scale(1.0);
    let mut scene = Scene::default();

    if blur > 0.0 {
        scene.push_filter(
            scaled.dilate(ScaledPixels(blur * 3.0)),
            scaled.center(),
            scaled,
            Filter {
                blur,
                ..Default::default()
            },
        );
    }
    scene.insert_primitive(PaintSurface {
        order: 0,
        bounds: scaled,
        content_mask: ContentMask { bounds }.scale(1.0),
        image_buffer: surface.buffer().clone(),
    });
    if blur > 0.0 {
        scene.pop_filter();
    }
    scene
}

fn drawable() -> MetalRenderer {
    let mut renderer = MetalRenderer::new(Context::default(), false);

    renderer.update_drawable_size(size(DevicePixels(SIDES as i32), DevicePixels(SIDES as i32)));
    renderer
}

fn middle(image: &image::RgbaImage) -> [u8; 4] {
    image.get_pixel(image.width() / 2, image.height() / 2).0
}

fn assert_shown(seen: [u8; 4], what: &str) {
    let expected = Rgba(SHOWN);

    for (channel, (seen, expected)) in seen.iter().zip(expected.0.iter()).enumerate() {
        let difference = i32::from(*seen) - i32::from(*expected);

        assert!(
            difference.abs() <= 2,
            "{what} came back {seen:?} where {expected:?} was drawn, so channel {channel} is \
             off by {difference}"
        );
    }
}

#[test]
fn a_drawable_draws_the_surface_as_the_colour_it_holds() {
    let surface = filled();
    let image = drawable().render_to_image(&scene(&surface, 0.0)).unwrap();

    assert_shown(middle(&image), "the surface, drawn to a drawable");
}

#[test]
fn a_blurred_layer_keeps_the_colour_it_was_given_through_a_drawable() {
    let surface = filled();
    let image = drawable().render_to_image(&scene(&surface, 28.0)).unwrap();

    assert_shown(middle(&image), "the surface, drawn through a blurred layer");
}

/*
 * The blur's own radius ramps, so a blurred layer is drawn at a radius of almost nothing
 * while the hold comes and goes.  A frame like that is as crisp as the one under it, which
 * is why a channel order that only the filter path gets wrong reads as a flicker rather
 * than as a permanent tint.
 */
#[test]
fn a_blurred_layer_at_the_smallest_radius_keeps_the_colour_it_was_given() {
    let surface = filled();
    let image = drawable().render_to_image(&scene(&surface, 0.5)).unwrap();

    assert_shown(middle(&image), "the surface, drawn through a barely focused layer");
}

/*
 * The blur is worn as a backdrop over the guest rather than as a filter around the layer
 * the guest's surface is drawn in, so this is the shape the window draws while it holds:
 * the surface, and a blurred copy of it over the top.
 */
#[test]
fn a_backdrop_blur_over_the_surface_keeps_the_colour_it_was_given() {
    use gpui::{Backdrop, Corners};

    let surface = filled();
    let mut scene = scene(&surface, 0.0);
    let sides = size(px(SIDES as f32), px(SIDES as f32));
    let bounds = Bounds::new(point(px(0.), px(0.)), sides);
    let scaled: Bounds<ScaledPixels> = bounds.scale(1.0);

    scene.insert_primitive(Backdrop {
        order: 1,
        pad: 0,
        blur: 28.0,
        opacity: 1.0,
        bounds: scaled,
        content_mask: ContentMask { bounds }.scale(1.0),
        corner_radii: Corners::default(),
    });

    let image = drawable().render_to_image(&scene).unwrap();

    assert_shown(middle(&image), "the surface, held under a blurred backdrop");
}

/*
 * The frame the console cannot put in a surface of ours comes over the socket with the
 * same bytes in the same order, so the picture it paints is the picture the surface
 * paints, and the window cannot flip between the two.
 */
/*
 * The window draws this surface on every frame it is up, a hundred and twenty times a
 * second, and the flicker it showed was one frame in six with the channels the other way
 * round - so a check that draws once proves nothing about the draw that repeats.
 */
#[test]
fn drawing_the_same_surface_over_and_over_keeps_the_colour() {
    let surface = filled();
    let mut renderer = drawable();

    for frame in 0..240 {
        let image = renderer.render_to_image(&scene(&surface, 0.0)).unwrap();

        assert_shown(middle(&image), &format!("frame {frame} of the same surface"));
    }
}

/*
 * The blur comes and goes while the window is up, so the same surface is drawn through a
 * filtered layer and straight into the drawable, one frame after the other.  If the pass
 * that a filtered frame goes through leaves anything behind, this is the loop that finds
 * it: the colours have to stay put across both.
 */
#[test]
fn a_filtered_frame_and_a_plain_one_in_the_same_loop_keep_the_colour() {
    let surface = filled();
    let mut renderer = drawable();

    for round in 0..80 {
        let blurred = renderer.render_to_image(&scene(&surface, 28.0)).unwrap();

        assert_shown(
            middle(&blurred),
            &format!("round {round}, through a blurred layer"),
        );

        let plain = renderer.render_to_image(&scene(&surface, 0.0)).unwrap();

        assert_shown(middle(&plain), &format!("round {round}, straight through"));
    }
}

/*
 * The console is writing into a surface of ours while the window draws another, and it
 * holds the one it is writing locked for the length of the write.  A window that draws a
 * surface the console has hold of is drawing one that is busy, which is a state no
 * headless harness gets into by itself - and if the draw changes then, this is where it
 * shows.
 */
#[test]
fn a_surface_the_console_is_writing_is_still_drawn_as_the_colour_it_holds() {
    let surface = filled();

    assert!(
        crate::surfacePort::lock(surface.buffer()),
        "the harness could not lock the surface the way the console does"
    );
    let image = drawable().render_to_image(&scene(&surface, 0.0)).unwrap();
    crate::surfacePort::unlock(surface.buffer());

    assert_shown(middle(&image), "the surface, drawn while the console held it");
}

/*
 * The same shapes as the window draws, at the size it draws them: a texture for every
 * filter step at the size of the drawable is what a small harness never makes, and a
 * region that comes back the wrong way round is a region the pass that made it wrote.
 */
#[test]
fn a_blurred_backdrop_at_the_window_size_keeps_the_colour_it_was_given() {
    use gpui::{Backdrop, Corners};

    const WIDE: u32 = 2560;
    const TALL: u32 = 1440;

    let surface = GuestSurface::new(WIDE, TALL).unwrap();

    for y in 0..TALL {
        for x in 0..WIDE {
            surface.pixel(x, y, STORED);
        }
    }

    let mut renderer = MetalRenderer::new(Context::default(), false);

    renderer.update_drawable_size(size(DevicePixels(WIDE as i32), DevicePixels(TALL as i32)));

    let sides = size(px(WIDE as f32), px(TALL as f32));
    let bounds = Bounds::new(point(px(0.), px(0.)), sides);
    let scaled: Bounds<ScaledPixels> = bounds.scale(1.0);
    let mut whole = Scene::default();

    whole.insert_primitive(PaintSurface {
        order: 0,
        bounds: scaled,
        content_mask: ContentMask { bounds }.scale(1.0),
        image_buffer: surface.buffer().clone(),
    });
    whole.insert_primitive(Backdrop {
        order: 1,
        pad: 0,
        blur: 28.0,
        opacity: 1.0,
        bounds: scaled,
        content_mask: ContentMask { bounds }.scale(1.0),
        corner_radii: Corners::default(),
    });

    let image = renderer.render_to_image(&whole).unwrap();

    assert_shown(image.get_pixel(WIDE / 2, TALL / 2).0, "a blurred backdrop");
    assert_shown(
        image.get_pixel(WIDE / 4, TALL / 4).0,
        "a blurred backdrop, a quarter in",
    );
    assert_shown(
        image.get_pixel(WIDE / 4, TALL - TALL / 8).0,
        "a blurred backdrop, low down",
    );
}

#[test]
fn the_socket_frame_paints_the_colour_the_surface_does() {
    use crate::dbusDisplay::pixels_to_rgba;

    /* what the console's surface holds: b,g,r,a a pixel, as qemu reads it back */
    let wire: Vec<u8> = STORED
        .iter()
        .copied()
        .cycle()
        .take(SIDES as usize * SIDES as usize * 4)
        .collect();
    let converted = pixels_to_rgba(SIDES, SIDES, SIDES * 4, 0x20020888, &wire);
    let socket = image::RgbaImage::from_raw(SIDES, SIDES, converted).unwrap();
    let drawn = drawable().render_to_image(&scene(&filled(), 0.0)).unwrap();

    assert_eq!(
        socket.get_pixel(SIDES / 2, SIDES / 2).0,
        middle(&drawn),
        "the frame the console sends over the socket is not the colour the window draws"
    );
    assert_eq!(
        filled().read().unwrap()[(SIDES as usize / 2 * SIDES as usize + SIDES as usize / 2) * 4
            ..(SIDES as usize / 2 * SIDES as usize + SIDES as usize / 2) * 4 + 4],
        SHOWN,
        "the buffer read back for the harness is not the colour the window draws"
    );
}
