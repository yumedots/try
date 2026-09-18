use std::{env, path::Path, sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex, OnceLock}, thread, time::{Duration, SystemTime, UNIX_EPOCH}};
use crate::grab::{grab_step_path, write_ppm};
use crate::surfaceRing::Ring;

/*
 * TRY_TRACE=1 prints the colour of the frame in the ring - what the console wrote into a
 * surface of ours, read back the way the window draws it - every 200 ms.  Held next to a
 * capture of the window it says which side of the wire turned red and blue over: the ring
 * alternating between two colour orders is the console's doing, the ring holding one while
 * the window shows two is the window's.
 */
pub(crate) fn spawn_surface_trace(ring: Arc<Mutex<Ring>>) {
    if env::var_os("TRY_TRACE").is_none() {
        return;
    }
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(200));
        let held = ring.lock().unwrap();
        let stride = held.stride();
        let Some((width, height, pixels)) = held.pixels() else {
            continue;
        };
        drop(held);
        let count = (pixels.len() / 4).max(1) as u64;
        let (mut red, mut green, mut blue) = (0u64, 0u64, 0u64);

        for pixel in pixels.as_chunks::<4>().0 {
            red += u64::from(pixel[0]);
            green += u64::from(pixel[1]);
            blue += u64::from(pixel[2]);
        }
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or(0);

        println!(
            "trysurface: {millis} {width}x{height} stride {stride} rgb {} {} {}",
            red / count,
            green / count,
            blue / count
        );
    });
}

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
