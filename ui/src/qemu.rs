use std::{
    env, path::Path, process::{Child, Command}, sync::{Arc, Mutex},
};
use crate::geometry::{DISPLAY_MAX, GUEST_MAX_DISPLAY};

pub(crate) type Qemu = Arc<Mutex<Option<Child>>>;

pub(crate) type QemuCommand = (String, Vec<String>, Vec<(String, String)>);

pub(crate) fn qemu_command(project: &Path) -> Result<QemuCommand, String> {
    /*
     * The QEMU device only accepts a guest resolution inside the size it advertises
     * with `xres`/`yres`; past that the guest drops the mode and falls back to a
     * standard 1920x1440, so the guest screen stops following the window.  Give it
     * the host screen: that is also the guest's boot mode, and any window that fits
     * the screen is then adopted exactly.
     */
    let (width, height) = *DISPLAY_MAX.get().unwrap_or(&GUEST_MAX_DISPLAY);
    let mut args = vec![
        "vm".to_string(),
        "--bridge".to_string(),
        "--print-args".to_string(),
        format!("--width={width}"),
        format!("--height={height}"),
    ];
    /*
     * The guest renders through the host GPU only when the display it exports is a GL
     * one, which the fork's QEMU serves over D-Bus; TRY_NO_GL asks for the software
     * virtio-gpu instead, for the runs that measure it against the default.
     */
    if env::var_os("TRY_NO_GL").is_none() {
        args.push("--gl".to_string());
    }
    let output = Command::new("xmake")
        .current_dir(project)
        .args(&args)
        .output()
        .map_err(|error| format!("could not run xmake: {error}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut program = None;
    let mut argv = Vec::new();
    let mut env = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("program=") {
            program = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("arg=") {
            argv.push(value.to_string());
        } else if let Some((key, value)) = line
            .strip_prefix("env=")
            .and_then(|line| line.split_once('='))
        {
            env.push((key.to_string(), value.to_string()));
        }
    }
    let Some(mut program) = program else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(format!(
            "could not read the QEMU command from xmake ({}): {detail}",
            output.status
        ));
    };
    if let Ok(path) = env::var("TRY_QEMU") {
        program = path;
    }
    Ok((program, argv, env))
}

/*
 * Why the guest stopped, for the failure the frame shows: an exit status says a
 * shut down or a panic the guest did not recover from, and a signal says QEMU.
 */
pub(crate) fn qemu_stopped(qemu: &Qemu) -> Option<String> {
    let mut held = qemu.lock().unwrap();
    let child = held.as_mut()?;

    match child.try_wait() {
        Ok(Some(status)) => Some(status.to_string()),
        _ => None,
    }
}
