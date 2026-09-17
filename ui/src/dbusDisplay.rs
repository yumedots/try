use std::sync::{mpsc::Sender, Mutex};
use crate::bridge::Event;

pub(crate) struct Surface {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Vec<u8>,
    pub(crate) format: u32,
}

pub(crate) struct Scanout {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) stride: u32,
    pub(crate) format: u32,
    pub(crate) data: Vec<u8>,
}

pub(crate) struct Update {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) stride: u32,
    pub(crate) format: u32,
    pub(crate) data: Vec<u8>,
}

pub(crate) struct Listener {
    pub(crate) events: Sender<Event>,
    pub(crate) surface: Option<Surface>,
}

pub(crate) struct DisplayListener {
    pub(crate) listener: Mutex<Listener>,
}

#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.VM",
    default_path = "/org/qemu/Display1/VM"
)]
pub trait VM {
    #[zbus(property, name = "ConsoleIDs")]
    fn console_ids(&self) -> zbus::Result<Vec<u32>>;
}

#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.Console",
    default_path = "/org/qemu/Display1/Console_0"
)]
pub trait Console {
    fn register_listener(&self, listener: zbus::zvariant::Fd<'_>) -> zbus::Result<()>;

    #[zbus(name = "SetUIInfo")]
    fn set_ui_info(
        &self,
        width_mm: u16,
        height_mm: u16,
        xoff: i32,
        yoff: i32,
        width: u32,
        height: u32,
    ) -> zbus::Result<()>;
}

impl Listener {
    pub(crate) fn replace_surface(&mut self, scanout: Scanout) {
        let pixels = pixels_to_rgba(
            scanout.width,
            scanout.height,
            scanout.stride,
            scanout.format,
            &scanout.data,
        );
        self.surface = Some(Surface {
            width: scanout.width,
            height: scanout.height,
            pixels,
            format: scanout.format,
        });
        self.send_frame();
    }

    pub(crate) fn update_surface(&mut self, update: Update) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        if surface.format != update.format || update.x < 0 || update.y < 0 {
            return;
        }
        let x = update.x as u32;
        let y = update.y as u32;
        let width = update.w.max(0) as u32;
        let height = update.h.max(0) as u32;
        if x.saturating_add(width) > surface.width || y.saturating_add(height) > surface.height {
            return;
        }
        let updated = pixels_to_rgba(width, height, update.stride, update.format, &update.data);
        for row in 0..height as usize {
            let source_start = row * width as usize * 4;
            let target_start = ((y as usize + row) * surface.width as usize + x as usize) * 4;
            surface.pixels[target_start..target_start + width as usize * 4]
                .copy_from_slice(&updated[source_start..source_start + width as usize * 4]);
        }
        self.send_frame();
    }

    pub(crate) fn send_frame(&self) {
        if let Some(surface) = &self.surface {
            let _ = self.events.send(Event::Frame {
                width: surface.width,
                height: surface.height,
                pixels: surface.pixels.clone(),
            });
        }
    }
}


pub(crate) fn pixels_to_rgba(width: u32, height: u32, stride: u32, format: u32, data: &[u8]) -> Vec<u8> {
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for y in 0..height as usize {
        let source_row = y * stride as usize;
        let target_row = y * width as usize * 4;
        for x in 0..width as usize {
            let source = source_row + x * 4;
            let target = target_row + x * 4;
            if source + 4 > data.len() {
                break;
            }
            let bytes = &data[source..source + 4];
            if format == 0x20088880 || format == 0x20088888 {
                pixels[target..target + 4].copy_from_slice(&[bytes[2], bytes[1], bytes[0], 255]);
            } else {
                pixels[target..target + 4].copy_from_slice(&[bytes[0], bytes[1], bytes[2], 255]);
            }
        }
    }
    pixels
}
