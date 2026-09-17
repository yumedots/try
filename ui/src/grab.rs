use gpui::{px, size};
use image::RgbaImage;
use std::{
    env, path::{Path, PathBuf}, process::exit, time::{Duration, Instant},
};
use crate::bridge::{start_bridge, Bridge, Event};
use crate::geometry::{clamp_to_display, window_size, GUEST_MAX_DISPLAY};

pub(crate) const GRAB_SCALE: f32 = 2.0;

pub(crate) const GRAB_TIMEOUT: u64 = 240;

/*
 * The frame path needs no window to be checked, so TRY_GRAB=<png> runs the display
 * bridge on its own and writes the frame the guest sent into that file.  With
 * TRY_GRAB_SIZE=<width>x<height>, in the same points a window is that big, it asks for
 * that size first, waits for a frame to come back at the size the guest can adopt - the
 * request is clamped to the display the booted device advertises, as the window's is -
 * and fails if the guest never gets there.  TRY_GRAB_RESIZE=<width>x<height>,... asks
 * again, one size after the other, TRY_GRAB_SETTLE seconds apart, which is what dragging
 * a window does to a guest that is already up; a frame is written next to the path after
 * every stage, named <stem>.<n><ext>, so a frame leaking over from the one before cannot
 * pass for the one that was asked for.
 */
pub(crate) fn grab(path: PathBuf) {
    let bridge = match start_bridge() {
        Ok(bridge) => bridge,
        Err(error) => {
            eprintln!("grab: {error}");
            exit(1);
        }
    };
    let started = Instant::now();
    let first = grab_size("TRY_GRAB_SIZE");
    let resizes = grab_sizes("TRY_GRAB_RESIZE");
    let mut wanted = match first {
        Some((width, height)) => {
            let wanted = ask_size(&bridge, width, height);
            println!("grab: asking for {width}x{height} points, guest at {wanted:?}");
            Some(wanted)
        }
        None => None,
    };
    /*
     * When the frame is due.  A resize postpones it: the first frame is written only
     * after the guest has had TRY_GRAB_WAIT seconds to come up, the ask goes out, and
     * whichever frame has arrived TRY_GRAB_SETTLE seconds later is the answer.
     */
    let mut due = Instant::now() + Duration::from_secs(grab_wait());
    let mut step = 0;
    let deadline = Instant::now() + Duration::from_secs(GRAB_TIMEOUT);
    let mut last = None;
    while Instant::now() < deadline {
        if let Some((width, height)) = resizes.get(step).copied() {
            if Instant::now() >= due {
                wanted = Some(ask_size(&bridge, width, height));
                println!("grab: asking for {width}x{height} points");
                step += 1;
                due = Instant::now() + Duration::from_secs(grab_settle());
            }
        }
        match bridge.events.recv_timeout(Duration::from_millis(250)) {
            Ok(Event::Frame {
                width,
                height,
                pixels,
            }) => {
                println!("grab: frame {width}x{height} at {:?}", started.elapsed());
                last = Some((width, height, pixels));
            }
            Ok(Event::Error(error)) => {
                eprintln!("grab: {error}");
                exit(1);
            }
            Ok(Event::Ready) => {}
            Err(_) => {}
        }
        let caught_up = match wanted {
            Some(want) => last.as_ref().map(|(w, h, _)| (*w, *h)) == Some(want),
            None => last.is_some(),
        };
        if Instant::now() >= due && caught_up {
            if let Some(frame) = last.as_ref() {
                let named = grab_step_path(&path, step);
                write_frame(&named, frame);
                println!(
                    "grab: wrote {}x{} to {} at step {step}",
                    frame.0,
                    frame.1,
                    named.display()
                );
            }
            if step >= resizes.len() {
                break;
            }
            continue;
        }
    }
    let Some(frame) = last else {
        eprintln!("grab: no frame arrived");
        exit(1);
    };
    let (width, height, _) = frame;
    write_frame(&path, &frame);
    println!("grab: wrote {width}x{height} to {}", path.display());
    if let Some((ask_width, ask_height)) = wanted {
        assert!(
            (ask_width, ask_height) == (width, height),
            "asked for {ask_width}x{ask_height}, guest sent {width}x{height}"
        );
    }
}

pub(crate) fn write_frame(path: &Path, (width, height, pixels): &(u32, u32, Vec<u8>)) {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("ppm") => write_ppm(path, *width, *height, pixels),
        _ => {
            let image = RgbaImage::from_raw(*width, *height, pixels.clone())
                .expect("frame is not a whole image");
            image.save(path).expect("could not write the frame");
        }
    }
}

pub(crate) fn grab_step_path(path: &Path, step: usize) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("png");
    path.with_extension(format!("{step}.{extension}"))
}

/* a raw dump, so a frame can be compared against another one without a decoder */
pub(crate) fn write_ppm(path: &Path, width: u32, height: u32, pixels: &[u8]) {
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.reserve(pixels.len());
    for pixel in pixels.as_chunks::<4>().0 {
        ppm.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(path, ppm).expect("could not write the frame");
}

pub(crate) fn grab_size(name: &str) -> Option<(u32, u32)> {
    grab_sizes(name).into_iter().next()
}

pub(crate) fn grab_sizes(name: &str) -> Vec<(u32, u32)> {
    env::var(name)
        .map(|sizes| parse_sizes(&sizes))
        .unwrap_or_default()
}

pub(crate) fn parse_sizes(sizes: &str) -> Vec<(u32, u32)> {
    sizes
        .split(',')
        .filter_map(|size| {
            let (width, height) = size.split_once('x')?;
            Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
        })
        .collect()
}

pub(crate) fn grab_wait() -> u64 {
    env::var("TRY_GRAB_WAIT")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(0)
}

pub(crate) fn grab_settle() -> u64 {
    env::var("TRY_GRAB_SETTLE")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(20)
}

/* asks for a window of this many points and answers the size the guest can adopt */
pub(crate) fn ask_size(bridge: &Bridge, width: u32, height: u32) -> (u32, u32) {
    let requested = window_size(size(px(width as f32), px(height as f32)), GRAB_SCALE);
    let wanted = clamp_to_display((requested.width, requested.height), GUEST_MAX_DISPLAY);
    bridge.resized(requested, Instant::now());
    wanted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grab_harness_takes_a_list_of_sizes() {
        assert_eq!(parse_sizes("1200x800"), vec![(1200, 800)]);
        assert_eq!(
            parse_sizes("1200x800, 1000x700 ,900x600"),
            vec![(1200, 800), (1000, 700), (900, 600)]
        );
    }
}
