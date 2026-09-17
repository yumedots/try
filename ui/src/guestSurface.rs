use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_video::pixel_buffer::{
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey,
    kCVPixelFormatType_32BGRA, CVPixelBuffer,
};
use gpui::{ObjectFit, Surface};

pub const PATTERN_CELL: u32 = 32;
pub const PATTERN_MARKER: u32 = 48;
const DARK: u8 = 0x28;
const LIGHT: u8 = 0x50;
const MARKER: [u8; 4] = [0x30, 0x30, 0xff, 0xff];

pub struct GuestSurface {
    buffer: CVPixelBuffer,
    width: u32,
    height: u32,
}

impl GuestSurface {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let empty: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[]);
        let attributes = CFDictionary::from_CFType_pairs(&[
            (
                unsafe { CFString::wrap_under_get_rule(kCVPixelBufferIOSurfacePropertiesKey) },
                empty.as_CFType(),
            ),
            (
                unsafe { CFString::wrap_under_get_rule(kCVPixelBufferMetalCompatibilityKey) },
                CFBoolean::true_value().as_CFType(),
            ),
        ]);
        let buffer = CVPixelBuffer::new(
            kCVPixelFormatType_32BGRA,
            width as usize,
            height as usize,
            Some(&attributes),
        )
        .map_err(|status| format!("no {width}x{height} pixel buffer: {status}"))?;
        Ok(Self {
            buffer,
            width,
            height,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn buffer(&self) -> &CVPixelBuffer {
        &self.buffer
    }

    #[allow(deprecated)]
    pub fn io_surface_id(&self) -> u32 {
        self.buffer
            .get_io_surface()
            .map(|surface| surface.get_id())
            .unwrap_or(0)
    }

    pub fn fill(&self) {
        let mut rows = [Vec::new(), Vec::new()];
        for (parity, row) in rows.iter_mut().enumerate() {
            for x in 0..self.width {
                let pixel = if x < PATTERN_MARKER {
                    MARKER
                } else if (x / PATTERN_CELL + parity as u32).is_multiple_of(2) {
                    [DARK, DARK, DARK, 0xff]
                } else {
                    [LIGHT, LIGHT, LIGHT, 0xff]
                };
                row.extend_from_slice(&pixel);
            }
        }
        self.write(|pixels, stride, width, height| {
            let bytes = width as usize * 4;
            for y in 0..height {
                let start = y as usize * stride;
                pixels[start..start + bytes]
                    .copy_from_slice(&rows[(y / PATTERN_CELL % 2) as usize]);
            }
        });
    }

    #[cfg(test)]
    pub fn pixel(&self, x: u32, y: u32, pixel: [u8; 4]) {
        self.write(|pixels, stride, width, height| {
            if x >= width || y >= height {
                return;
            }
            let start = y as usize * stride + x as usize * 4;
            pixels[start..start + 4].copy_from_slice(&pixel);
        });
    }

    fn write(&self, body: impl FnOnce(&mut [u8], usize, u32, u32)) {
        if self.width == 0 || self.height == 0 || self.buffer.lock_base_address(0) != 0 {
            return;
        }
        let stride = self.buffer.get_bytes_per_row();
        let base = unsafe { self.buffer.get_base_address() as *mut u8 };
        let pixels = unsafe { std::slice::from_raw_parts_mut(base, stride * self.height as usize) };
        body(pixels, stride, self.width, self.height);
        self.buffer.unlock_base_address(0);
    }
}

#[allow(non_snake_case)]
pub fn guestSurface(surface: &GuestSurface) -> Surface {
    gpui::surface(surface.buffer().clone()).object_fit(ObjectFit::Fill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        point, px, size, Bounds, ContentMask, DevicePixels, PaintSurface, PlatformHeadlessRenderer,
        Scene,
    };
    use gpui_apple::metal_renderer::MetalHeadlessRenderer;
    use image::Rgba;

    fn render(renderer: &mut MetalHeadlessRenderer, surface: &GuestSurface) -> image::RgbaImage {
        let mut scene = Scene::default();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(64.), px(64.)));
        scene.insert_primitive(PaintSurface {
            order: 0,
            bounds: bounds.scale(1.0),
            content_mask: ContentMask { bounds }.scale(1.0),
            image_buffer: surface.buffer().clone(),
        });
        renderer
            .render_scene_to_image(&scene, size(DevicePixels(64), DevicePixels(64)))
            .unwrap()
    }

    #[test]
    fn the_content_survives_a_blurred_layer() {
        use gpui::{point, px, size, Bounds, ContentMask, Filter, ScaledPixels};

        let surface = GuestSurface::new(64, 64).unwrap();
        surface.fill();
        let mut renderer = MetalHeadlessRenderer::new();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(64.), px(64.)));
        let scaled: Bounds<ScaledPixels> = bounds.scale(1.0);

        let mut blurred = Scene::default();
        blurred.push_filter(
            scaled.dilate(ScaledPixels(28.0 * 3.0)),
            scaled.center(),
            scaled,
            Filter {
                blur: 28.0,
                ..Default::default()
            },
        );
        blurred.insert_primitive(PaintSurface {
            order: 1,
            bounds: scaled,
            content_mask: ContentMask { bounds }.scale(1.0),
            image_buffer: surface.buffer().clone(),
        });
        blurred.pop_filter();

        let image = renderer
            .render_scene_to_image(&blurred, size(DevicePixels(64), DevicePixels(64)))
            .unwrap();
        let (mut opaque, mut sum) = (0u32, 0u32);
        for pixel in image.pixels() {
            if pixel.0[3] == 0xff {
                opaque += 1;
            }
            sum += u32::from(pixel.0[0]);
        }
        let mean = sum / (image.width() * image.height());
        println!("blurred frame: {opaque} opaque pixels, mean {mean}");
        assert!(
            opaque == image.width() * image.height(),
            "a blurred layer dropped the content: {opaque} of {} pixels are opaque",
            image.width() * image.height()
        );
        assert!(mean > 0x20, "a blurred layer came back empty: mean {mean}");
    }

    #[test]
    fn the_window_draws_the_callers_buffer() {
        let surface = GuestSurface::new(64, 64).unwrap();
        surface.fill();
        let mut renderer = MetalHeadlessRenderer::new();
        let image = render(&mut renderer, &surface);
        assert_eq!(image.get_pixel(10, 10), &Rgba([0xff, 0x30, 0x30, 0xff]));
        assert_eq!(image.get_pixel(50, 8), &Rgba([LIGHT, LIGHT, LIGHT, 0xff]));
        assert_eq!(image.get_pixel(50, 40), &Rgba([DARK, DARK, DARK, 0xff]));

        surface.pixel(50, 40, [0x00, 0xff, 0x00, 0xff]);
        let image = render(&mut renderer, &surface);
        assert_eq!(image.get_pixel(50, 40), &Rgba([0x00, 0xff, 0x00, 0xff]));
        assert_eq!(image.get_pixel(50, 8), &Rgba([LIGHT, LIGHT, LIGHT, 0xff]));
    }
}
