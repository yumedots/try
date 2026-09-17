use gpui::App;
use std::process::Command;
use crate::geometry::GUEST_MAX_DISPLAY;
use crate::paths::project_dir;

pub(crate) fn host_display_max(cx: &App) -> (u32, u32) {
    cx.displays()
        .first()
        .map(|display| {
            let bounds = display.bounds();
            (
                (bounds.size.width.to_f64() * 2.0).round() as u32,
                (bounds.size.height.to_f64() * 2.0).round() as u32,
            )
        })
        .filter(|(width, height)| *width > 0 && *height > 0)
        .unwrap_or(GUEST_MAX_DISPLAY)
}

pub(crate) fn host_cores() -> u32 {
    let (program, args) = if cfg!(target_os = "macos") {
        ("sysctl", vec!["-n", "hw.ncpu"])
    } else {
        ("nproc", vec![])
    };
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(8)
}

pub(crate) fn host_memory_gb() -> u32 {
    let bytes = if cfg!(target_os = "macos") {
        Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|text| text.trim().parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    let value = line
                        .strip_prefix("MemTotal:")?
                        .trim()
                        .trim_end_matches("kB")
                        .trim();
                    value.parse::<u64>().ok()
                })
            })
            .unwrap_or(0)
            * 1024
    };
    ((bytes / 1073741824) as u32).max(4)
}

pub(crate) fn run_host(args: &[&str]) -> Result<(), String> {
    let output = Command::new("xmake")
        .current_dir(project_dir())
        .args(args)
        .output()
        .map_err(|error| format!("could not run xmake {}: {error}", args.join(" ")))?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    Err(format!("xmake {} failed: {}", args.join(" "), text.trim()))
}
