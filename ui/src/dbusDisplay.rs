use std::collections::HashMap;
use std::sync::{mpsc::Sender, Mutex};
use crate::bridge::Event;
use crate::surfaceRing::SharedRing;

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
    pub(crate) ring: SharedRing,
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

    fn set_surface(&self, surface: &str) -> zbus::Result<()>;
}

/*
 * The console's UI info, as a dictionary: `Apply` merges the keys it is given into what
 * the console already holds, which is the only way to say the panel's refresh rate -
 * `SetUIInfo` carries no such key and clears the one that is there.
 */
#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.UIInfo",
    default_path = "/org/qemu/Display1/Console_0"
)]
pub trait UiInfo {
    fn apply(
        &self,
        ui_info: HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<()>;
}

impl Listener {
    pub(crate) fn replace_surface(&mut self, scanout: Scanout) {
        if scanout.data.is_empty() {
            return self.handed_frame();
        }
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
        self
            .ring
            .lock()
            .unwrap()
            .software_frame(scanout.width, scanout.height);
        self.send_frame();
    }

    pub(crate) fn update_surface(&mut self, update: Update) {
        if update.data.is_empty() {
            return self.handed_frame();
        }
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        if surface.format != update.format || update.x < 0 || update.y < 0 {
            return;
        }
        let x = update.x as u32;
        let y = update.y as u32;
        let width = update.w.max(0) as u32;
        let height = update.h.max(0) as u32;
        let fits = x.saturating_add(width) <= surface.width
            && y.saturating_add(height) <= surface.height;
        /*
         * The whole of it from the corner at a size this frame is not: that is the console
         * at a new size, not a patch of this one - it happens when the console could not
         * write into a surface of ours, and without this the frame goes on being that size.
         */
        if !fits {
            if x == 0 && y == 0 {
                self.replace_surface(Scanout {
                    width,
                    height,
                    stride: update.stride,
                    format: update.format,
                    data: update.data,
                });
            }
            return;
        }
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let updated = pixels_to_rgba(width, height, update.stride, update.format, &update.data);
        for row in 0..height as usize {
            let source_start = row * width as usize * 4;
            let target_start = ((y as usize + row) * surface.width as usize + x as usize) * 4;
            surface.pixels[target_start..target_start + width as usize * 4]
                .copy_from_slice(&updated[source_start..source_start + width as usize * 4]);
        }
        self.send_frame();
    }

    /*
     * The pixels are not here: the console read them into a surface of ours, which is
     * already in the window's hands, so all this frame costs is the redraw.
     */
    fn handed_frame(&mut self) {
        self.ring.lock().unwrap().landed();
        let _ = self.events.send(Event::Surface);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};
    use crate::surfaceRing::Ring;

    const BGRX: u32 = 0x20088880;

    fn frame(width: u32, height: u32, value: u8) -> Vec<u8> {
        let mut data = vec![0; width as usize * height as usize * 4];
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.fill(value);
        }
        data
    }

    fn listener() -> (Listener, mpsc::Receiver<Event>) {
        let (events, frames) = mpsc::channel();
        (
            Listener {
                events,
                ring: Arc::new(Mutex::new(Ring::new())),
                surface: None,
            },
            frames,
        )
    }

    #[test]
    fn a_whole_frame_at_another_size_is_the_console_resizing() {
        let (mut listener, frames) = listener();
        listener.replace_surface(Scanout {
            width: 64,
            height: 48,
            stride: 64 * 4,
            format: BGRX,
            data: frame(64, 48, 0x20),
        });
        assert_eq!(listener.ring.lock().unwrap().size(), (64, 48));

        /* the shape the console sends when it could not write into a surface of ours */
        listener.update_surface(Update {
            x: 0,
            y: 0,
            w: 96,
            h: 64,
            stride: 96 * 4,
            format: BGRX,
            data: frame(96, 64, 0x40),
        });
        assert_eq!(
            listener.ring.lock().unwrap().size(),
            (96, 64),
            "the frame went on being the size the console had stopped using"
        );
        assert!(frames.try_iter().count() > 0);
    }

    #[test]
    fn a_patch_of_this_frame_leaves_it_alone() {
        let (mut listener, _frames) = listener();
        listener.replace_surface(Scanout {
            width: 64,
            height: 48,
            stride: 64 * 4,
            format: BGRX,
            data: frame(64, 48, 0x20),
        });

        /* a patch that does not fit is not the console resizing: it is a frame to drop */
        listener.update_surface(Update {
            x: 32,
            y: 16,
            w: 64,
            h: 48,
            stride: 64 * 4,
            format: BGRX,
            data: frame(64, 48, 0x40),
        });
        assert_eq!(listener.ring.lock().unwrap().size(), (64, 48));
    }
}
