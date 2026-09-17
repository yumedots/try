use std::{env, path::Path, sync::{atomic::{AtomicUsize, Ordering}, Mutex, OnceLock}};
use crate::grab::{grab_step_path, write_ppm};

/*
 * TRY_DUMP=<path> writes every frame the windowed launcher receives next to that
 * path, numbered, with the size and how bright it is, so what the guest sends can
 * be held against what the window shows.
 */
pub(crate) fn dump_frame(width: u32, height: u32, pixels: &[u8]) {
    let Some(path) = env::var_os("TRY_DUMP") else {
        return;
    };
    let frame = DUMPED.fetch_add(1, Ordering::Relaxed);
    let bright = pixels.iter().step_by(4).filter(|byte| **byte > 40).count();
    println!(
        "tryframe: {frame} {width}x{height} bright {bright} of {}",
        pixels.len() / 4
    );
    let slot = DUMP_LAST.get_or_init(|| Mutex::new(None));
    let mut last = slot.lock().unwrap();
    let write = match *last {
        Some((seen, size)) => size != (width, height) || frame - seen >= 20,
        None => true,
    };
    if !write || frame > 6000 {
        return;
    }
    *last = Some((frame, (width, height)));
    write_ppm(
        &grab_step_path(Path::new(&path), frame),
        width,
        height,
        pixels,
    );
}

pub(crate) static DUMPED: AtomicUsize = AtomicUsize::new(0);

pub(crate) type LastDump = OnceLock<Mutex<Option<(usize, (u32, u32))>>>;

pub(crate) static DUMP_LAST: LastDump = OnceLock::new();
