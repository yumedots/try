use async_std::task;
use gpui::{
    div, img, prelude::*, px, rgb, size, AnyElement, App, Bounds, ClickEvent, Context, Div,
    FocusHandle, ImageSource, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    Pixels, Point, Render, RenderImage, ScrollDelta, ScrollWheelEvent, Size, Stateful, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;
use image::{Frame as ImageFrame, RgbaImage};

#[allow(non_snake_case)]
mod guestSurface;

use guestSurface::{guestSurface, GuestSurface};
use smallvec::SmallVec;
use std::{
    env,
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{exit, Child, Command},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

enum Event {
    Frame {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    Ready,
    Error(String),
}

enum Input {
    Key { code: u32, down: bool },
    Button { button: u32, down: bool },
    Move { x: u32, y: u32 },
    Wheel { up: bool },
}

const GUEST_MAX_DISPLAY: (u32, u32) = (2560, 1440);
const POLL_INTERVAL: Duration = Duration::from_millis(200);
const GRAB_SCALE: f32 = 2.0;
const GRAB_TIMEOUT: u64 = 240;

static DISPLAY_MAX: OnceLock<(u32, u32)> = OnceLock::new();

fn host_display_max(cx: &App) -> (u32, u32) {
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

type Qemu = Arc<Mutex<Option<Child>>>;

struct Bridge {
    events: Receiver<Event>,
    window_size: SharedSize,
    input: Sender<Input>,
    qemu: Qemu,
}

impl Bridge {
    fn set_window_size(&self, size: WindowSize) {
        *self.window_size.lock().unwrap() = Some(size);
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let mut qemu = self.qemu.lock().unwrap();
        let Some(child) = qemu.as_mut() else {
            return;
        };
        let _ = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
        for _ in 0..20 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

struct Frame {
    bridge: Option<Bridge>,
    pending: Option<Receiver<Result<Bridge, String>>>,
    settings: Settings,
    settings_open: bool,
    focus: FocusHandle,
    surface: (u32, u32),
    held: Modifiers,
    capslock: bool,
    status: Option<String>,
    image: Option<Arc<RenderImage>>,
    ready: bool,
    error: Option<String>,
    guest: Option<GuestSurface>,
    surface_demo: bool,
    wanted: Option<WindowSize>,
    shape_note: Option<String>,
}

struct Surface {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    format: u32,
}

struct Scanout {
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
    data: Vec<u8>,
}

struct Update {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    stride: u32,
    format: u32,
    data: Vec<u8>,
}

struct Listener {
    events: Sender<Event>,
    surface: Option<Surface>,
}

struct DisplayListener {
    listener: Mutex<Listener>,
}

#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.VM",
    default_path = "/org/qemu/Display1/VM"
)]
trait VM {
    #[zbus(property, name = "ConsoleIDs")]
    fn console_ids(&self) -> zbus::Result<Vec<u32>>;
}

#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.Console",
    default_path = "/org/qemu/Display1/Console_0"
)]
trait Console {
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

#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.Keyboard",
    default_path = "/org/qemu/Display1/Console_0"
)]
trait Keyboard {
    #[zbus(name = "Press")]
    fn press(&self, keycode: u32) -> zbus::Result<()>;

    #[zbus(name = "Release")]
    fn release(&self, keycode: u32) -> zbus::Result<()>;
}

#[zbus::proxy(
    default_service = "org.qemu",
    interface = "org.qemu.Display1.Mouse",
    default_path = "/org/qemu/Display1/Console_0"
)]
trait Mouse {
    #[zbus(name = "Press")]
    fn press(&self, button: u32) -> zbus::Result<()>;

    #[zbus(name = "Release")]
    fn release(&self, button: u32) -> zbus::Result<()>;

    #[zbus(name = "SetAbsPosition")]
    fn set_abs_position(&self, x: u32, y: u32) -> zbus::Result<()>;
}

const SHIFT: u32 = 0x2a;
const CONTROL: u32 = 0x1d;
const ALT: u32 = 0x38;
const SUPER: u32 = 0xdb;
const CAPS_LOCK: u32 = 0x3a;
const WHEEL_UP: u32 = 3;
const WHEEL_DOWN: u32 = 4;

fn guest_position(
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

fn frame_size(viewport: Size<Pixels>) -> (f32, f32) {
    (
        viewport.width.to_f64() as f32,
        viewport.height.to_f64() as f32,
    )
}

/*
 * What the guest sent against what the window asked for, so a screen that does not
 * fill the window can be told from one that is a resize behind.
 */
fn frame_shape(surface: (u32, u32), want: Option<WindowSize>) -> Option<String> {
    let size = want?;
    ((surface.0, surface.1) != (size.width, size.height)).then(|| {
        format!(
            "guest sent {}x{} for a {}x{} window",
            surface.0, surface.1, size.width, size.height
        )
    })
}

fn display_max(argv: &[String]) -> (u32, u32) {
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

fn clamp_to_display(size: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let scale = (max.0 as f64 / size.0 as f64)
        .min(max.1 as f64 / size.1 as f64)
        .min(1.0);
    (
        ((size.0 as f64 * scale) as u32).max(1),
        ((size.1 as f64 * scale) as u32).max(1),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowSize {
    width_mm: u16,
    height_mm: u16,
    width: u32,
    height: u32,
}

type SharedSize = Arc<Mutex<Option<WindowSize>>>;

/*
 * The guest reads the window twice over from the EDID we send: the pixel size asks for
 * that many guest pixels, and the physical size is how it learns how many of them are
 * one of the window's points.  Describing the window at 110 dpi puts a 1x, 1.5x and 2x
 * host on the guest's 1, 1.5 and 2 scales, and in whole centimetres because that is the
 * only precision a base EDID carries.  The pixel size is rounded down to whole logical
 * pixels so the guest never has to round a fractional one.
 */
const LOGICAL_DPI: f64 = 110.0;
const MM_PER_CM: u16 = 10;
const MAX_CM: f64 = 255.0;

fn window_size(viewport: Size<Pixels>, scale: f32) -> WindowSize {
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
fn opening_size(wanted: WindowSize, display: (u32, u32), ready: bool) -> WindowSize {
    if ready {
        return wanted;
    }
    WindowSize {
        width: display.0,
        height: display.1,
        ..wanted
    }
}

fn is_settings_toggle(keystroke: &Keystroke) -> bool {
    (keystroke.modifiers.platform || keystroke.modifiers.control) && keystroke.key == ","
}

fn keycode(key: &str) -> Option<u32> {
    Some(match key {
        "escape" => 0x01,
        "1" => 0x02,
        "2" => 0x03,
        "3" => 0x04,
        "4" => 0x05,
        "5" => 0x06,
        "6" => 0x07,
        "7" => 0x08,
        "8" => 0x09,
        "9" => 0x0a,
        "0" => 0x0b,
        "-" => 0x0c,
        "=" => 0x0d,
        "backspace" => 0x0e,
        "tab" => 0x0f,
        "q" => 0x10,
        "w" => 0x11,
        "e" => 0x12,
        "r" => 0x13,
        "t" => 0x14,
        "y" => 0x15,
        "u" => 0x16,
        "i" => 0x17,
        "o" => 0x18,
        "p" => 0x19,
        "[" => 0x1a,
        "]" => 0x1b,
        "enter" => 0x1c,
        "a" => 0x1e,
        "s" => 0x1f,
        "d" => 0x20,
        "f" => 0x21,
        "g" => 0x22,
        "h" => 0x23,
        "j" => 0x24,
        "k" => 0x25,
        "l" => 0x26,
        ";" => 0x27,
        "'" => 0x28,
        "`" => 0x29,
        "\\" => 0x2b,
        "z" => 0x2c,
        "x" => 0x2d,
        "c" => 0x2e,
        "v" => 0x2f,
        "b" => 0x30,
        "n" => 0x31,
        "m" => 0x32,
        "," => 0x33,
        "." => 0x34,
        "/" => 0x35,
        "space" => 0x39,
        "capslock" => CAPS_LOCK,
        "f1" => 0x3b,
        "f2" => 0x3c,
        "f3" => 0x3d,
        "f4" => 0x3e,
        "f5" => 0x3f,
        "f6" => 0x40,
        "f7" => 0x41,
        "f8" => 0x42,
        "f9" => 0x43,
        "f10" => 0x44,
        "f11" => 0x57,
        "f12" => 0x58,
        "home" => 0x80 | 0x47,
        "up" => 0x80 | 0x48,
        "pageup" => 0x80 | 0x49,
        "left" => 0x80 | 0x4b,
        "right" => 0x80 | 0x4d,
        "end" => 0x80 | 0x4f,
        "down" => 0x80 | 0x50,
        "pagedown" => 0x80 | 0x51,
        "insert" => 0x80 | 0x52,
        "delete" => 0x80 | 0x53,
        _ => return None,
    })
}

impl Listener {
    fn replace_surface(&mut self, scanout: Scanout) {
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

    fn update_surface(&mut self, update: Update) {
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

    fn send_frame(&self) {
        if let Some(surface) = &self.surface {
            let _ = self.events.send(Event::Frame {
                width: surface.width,
                height: surface.height,
                pixels: surface.pixels.clone(),
            });
        }
    }
}

#[zbus::interface(name = "org.qemu.Display1.Listener", spawn = false)]
impl DisplayListener {
    async fn scanout(
        &self,
        width: u32,
        height: u32,
        stride: u32,
        format: u32,
        data: serde_bytes::ByteBuf,
    ) {
        self.listener.lock().unwrap().replace_surface(Scanout {
            width,
            height,
            stride,
            format,
            data: data.into_vec(),
        });
    }

    async fn update(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        stride: u32,
        format: u32,
        data: serde_bytes::ByteBuf,
    ) {
        self.listener.lock().unwrap().update_surface(Update {
            x,
            y,
            w: width,
            h: height,
            stride,
            format,
            data: data.into_vec(),
        });
    }

    async fn disable(&self) {}

    async fn mouse_set(&self, _x: i32, _y: i32, _on: i32) {}

    async fn cursor_define(
        &self,
        _width: i32,
        _height: i32,
        _hot_x: i32,
        _hot_y: i32,
        _data: serde_bytes::ByteBuf,
    ) {
    }
}

fn pixels_to_rgba(width: u32, height: u32, stride: u32, format: u32, data: &[u8]) -> Vec<u8> {
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

#[derive(Clone)]
struct Settings {
    cores: u32,
    mem: String,
    audio: bool,
}

fn state_dir() -> PathBuf {
    env::var_os("TRY_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_dir().join("state"))
}

fn host_cores() -> u32 {
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

fn host_memory_gb() -> u32 {
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

const MEMORY: [&str; 7] = ["2G", "4G", "6G", "8G", "12G", "16G", "24G"];

fn memory_index(mem: &str) -> usize {
    MEMORY.iter().position(|value| *value == mem).unwrap_or(3)
}

fn read_settings() -> Settings {
    let mut settings = Settings {
        cores: host_cores().min(8),
        mem: if host_memory_gb() >= 16 {
            "8G".into()
        } else {
            "4G".into()
        },
        audio: cfg!(target_os = "macos"),
    };
    if let Ok(text) = std::fs::read_to_string(state_dir().join("settings")) {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "cores" => settings.cores = value.parse().unwrap_or(settings.cores),
                "mem" => settings.mem = value.to_string(),
                "audio" => settings.audio = value == "on",
                _ => {}
            }
        }
    }
    settings
}

fn write_settings(settings: &Settings) {
    let dir = state_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let text = format!(
        "cores={}\nmem={}\naudio={}\n",
        settings.cores,
        settings.mem,
        if settings.audio { "on" } else { "off" }
    );
    let _ = std::fs::write(dir.join("settings"), text);
}

fn project_dir() -> PathBuf {
    PathBuf::from(env::var("TRY_PROJECT_DIR").unwrap_or_else(|_| ".".into()))
}

fn serial_log(project: &Path) -> PathBuf {
    env::var_os("TRY_SERIAL_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.join("build/guest.log"))
}

fn qemu_command(project: &Path) -> Result<(String, Vec<String>, Vec<(String, String)>), String> {
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
     * one, which the fork's QEMU serves over D-Bus; TRY_GL asks for that instead of the
     * software virtio-gpu, for the runs that measure it against the default.
     */
    if env::var_os("TRY_GL").is_some() {
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

fn start_bridge() -> Result<Bridge, String> {
    let project = project_dir();
    let (program, argv, qemu_env) = qemu_command(&project)?;
    let display = display_max(&argv);
    let (events_tx, events_rx) = mpsc::channel();
    let (input_tx, input_rx) = mpsc::channel();
    let window_size: SharedSize = Arc::new(Mutex::new(None));
    let thread_window = window_size.clone();
    let qemu: Qemu = Arc::new(Mutex::new(None));
    let thread_qemu = qemu.clone();
    let log = serial_log(&project);

    thread::spawn(move || {
        let serial_log = log;
        if let Err(error) = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&serial_log)
        {
            let _ = events_tx.send(Event::Error(format!(
                "could not reset the guest log: {error}"
            )));
            return;
        }
        match Command::new(&program)
            .args(&argv)
            .envs(
                qemu_env
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            )
            .spawn()
        {
            Ok(child) => *thread_qemu.lock().unwrap() = Some(child),
            Err(error) => {
                let _ = events_tx.send(Event::Error(format!("could not start QEMU: {error}")));
                return;
            }
        }
        println!("started headless QEMU for GPUI display");
        let result = task::block_on(connect_display(
            events_tx.clone(),
            thread_window,
            input_rx,
            thread_qemu.clone(),
            serial_log,
            display,
        ));
        if let Err(error) = result {
            let _ = events_tx.send(Event::Error(error));
        }
    });

    Ok(Bridge {
        events: events_rx,
        window_size,
        input: input_tx,
        qemu,
    })
}

fn run_host(args: &[&str]) -> Result<(), String> {
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

fn spawn_bridge(reset: bool) -> Receiver<Result<Bridge, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = if reset {
            run_host(&["reset"])
                .and_then(|()| run_host(&["build", "disk"]))
                .and_then(|()| start_bridge())
        } else {
            start_bridge()
        };
        let _ = sender.send(result);
    });
    receiver
}

/*
 * Why the guest stopped, for the failure the frame shows: an exit status says a
 * shut down or a panic the guest did not recover from, and a signal says QEMU.
 */
fn qemu_stopped(qemu: &Qemu) -> Option<String> {
    let mut held = qemu.lock().unwrap();
    let child = held.as_mut()?;

    match child.try_wait() {
        Ok(Some(status)) => Some(status.to_string()),
        _ => None,
    }
}

async fn connect_display(
    events: Sender<Event>,
    window: SharedSize,
    input: Receiver<Input>,
    qemu: Qemu,
    serial_log: PathBuf,
    display: (u32, u32),
) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| format!("could not connect to the D-Bus session: {error}"))?;
    println!("connected to D-Bus session");
    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|error| format!("could not open the D-Bus session: {error}"))?;
    let qemu_name = zbus::names::BusName::try_from("org.qemu")
        .map_err(|error| format!("invalid QEMU D-Bus name: {error}"))?;
    loop {
        if let Some(status) = qemu_stopped(&qemu) {
            return Err(format!(
                "QEMU exited before its D-Bus display became available: {status}"
            ));
        }
        if !dbus
            .name_has_owner(qemu_name.clone())
            .await
            .unwrap_or(false)
        {
            task::sleep(Duration::from_millis(100)).await;
            continue;
        }
        break;
    }
    println!("found QEMU D-Bus service");
    let vm = VMProxy::new(&connection)
        .await
        .map_err(|error| format!("could not open the QEMU VM interface: {error}"))?;
    let console_id = vm
        .console_ids()
        .await
        .map_err(|error| format!("could not find the QEMU console: {error}"))?
        .into_iter()
        .next()
        .ok_or_else(|| "QEMU exported no display console".to_string())?;
    println!("found QEMU console {console_id}");
    let console_path = format!("/org/qemu/Display1/Console_{console_id}");
    let console = ConsoleProxy::builder(&connection)
        .path(console_path.clone())
        .map_err(|error| format!("could not create the QEMU console path: {error}"))?
        .build()
        .await
        .map_err(|error| format!("could not open the QEMU console: {error}"))?;
    let keyboard = KeyboardProxy::builder(&connection)
        .path(console_path.clone())
        .map_err(|error| format!("could not create the QEMU keyboard path: {error}"))?
        .build()
        .await
        .map_err(|error| format!("could not open the QEMU keyboard: {error}"))?;
    let mouse = MouseProxy::builder(&connection)
        .path(console_path)
        .map_err(|error| format!("could not create the QEMU mouse path: {error}"))?
        .build()
        .await
        .map_err(|error| format!("could not open the QEMU mouse: {error}"))?;
    let (qemu_stream, listener_stream) = std::os::unix::net::UnixStream::pair()
        .map_err(|error| format!("could not create the QEMU display socket: {error}"))?;
    console
        .register_listener((&qemu_stream).into())
        .await
        .map_err(|error| format!("could not register the QEMU framebuffer listener: {error}"))?;
    let _listener_connection = zbus::connection::Builder::async_io_unix_stream(listener_stream)
        .p2p()
        .serve_at(
            "/org/qemu/Display1/Listener",
            DisplayListener {
                listener: Mutex::new(Listener {
                    events: events.clone(),
                    surface: None,
                }),
            },
        )
        .map_err(|error| format!("could not create the QEMU display listener: {error}"))?
        .build()
        .await
        .map_err(|error| format!("could not start the QEMU display listener: {error}"))?;
    println!("registered QEMU framebuffer listener");
    println!("connected to QEMU D-Bus console {console_id}");
    let mut ready = false;
    let mut requested = None;
    let mut polled = Instant::now();
    loop {
        if !ready && polled.elapsed() >= POLL_INTERVAL {
            polled = Instant::now();
            if guest_display_ready(&serial_log) {
                ready = true;
                let _ = events.send(Event::Ready);
                println!("guest display readiness service completed");
            }
        }
        let mut pointer = None;
        let mut wheel = None;
        for event in input.try_iter() {
            match event {
                Input::Key { code, down } => {
                    let sent = if down {
                        keyboard.press(code).await
                    } else {
                        keyboard.release(code).await
                    };
                    if let Err(error) = sent {
                        println!("guest keyboard input failed: {error}");
                    }
                }
                Input::Button { button, down } => {
                    if let Some((x, y)) = pointer.take() {
                        mouse.set_abs_position(x, y).await.ok();
                    }
                    if let Err(error) = send_button(&mouse, button, down).await {
                        println!("guest mouse input failed: {error}");
                    }
                }
                Input::Move { x, y } => pointer = Some((x, y)),
                Input::Wheel { up } => wheel = Some(up),
            }
        }
        if let Some((x, y)) = pointer {
            mouse.set_abs_position(x, y).await.ok();
        }
        if let Some(up) = wheel {
            send_button(&mouse, if up { WHEEL_UP } else { WHEEL_DOWN }, true)
                .await
                .ok();
            send_button(&mouse, if up { WHEEL_UP } else { WHEEL_DOWN }, false)
                .await
                .ok();
        }
        /*
         * The window is the source of truth, so follow it whenever it changes: an
         * earlier version of this only acted on window-resize *events*, which never
         * arrive for the size the window opened at, leaving the guest at its boot mode
         * (the whole screen) letterboxed inside a smaller window.
         *
         * The guest keeps the shape of the window but not more pixels than the QEMU
         * device advertises (its `xres`/`yres`): a preferred mode past that is dropped
         * (virtio_gpu_conn_mode_valid) and the guest silently falls back to the largest
         * mode in the EDID list, which is what leaves the window showing a differently
         * shaped screen.  Asking for the window scaled into that box keeps the layout
         * matching the window, at the cost of the host scaling the image a little.
         */
        let wanted = *window.lock().unwrap();
        if let Some(size) = wanted {
            let size = opening_size(size, display, ready);
            let (width, height) = clamp_to_display((size.width, size.height), display);
            if requested != Some(size) {
                console
                    .set_ui_info(size.width_mm, size.height_mm, 0, 0, width, height)
                    .await
                    .map_err(|error| format!("could not resize the QEMU display: {error}"))?;
                println!("requested guest display resize: {width}x{height}");
                requested = Some(size);
            }
        }
        if let Some(status) = qemu_stopped(&qemu) {
            return Err(format!("QEMU exited: {status}"));
        }
        task::sleep(Duration::from_millis(16)).await;
    }
}

async fn send_button(mouse: &MouseProxy<'_>, button: u32, down: bool) -> zbus::Result<()> {
    if down {
        mouse.press(button).await
    } else {
        mouse.release(button).await
    }
}

fn guest_display_ready(serial_log: &PathBuf) -> bool {
    std::fs::read_to_string(serial_log)
        .map(|log| log.contains("TRY_DISPLAY_READY=1"))
        .unwrap_or(false)
}

impl Frame {
    fn new(cx: &mut Context<Self>) -> Self {
        let settings = read_settings();
        let (bridge, error) = match start_bridge() {
            Ok(bridge) => (Some(bridge), None),
            Err(error) => (None, Some(error)),
        };
        cx.observe_keystrokes(|frame, event, _, cx| {
            let keystroke = &event.keystroke;
            if is_settings_toggle(keystroke) {
                frame.settings_open = !frame.settings_open;
                cx.notify();
                return;
            }
            if frame.settings_open && keystroke.key == "escape" {
                frame.settings_open = false;
                cx.notify();
            }
        })
        .detach();
        cx.spawn(async move |this, cx| loop {
            this.update(cx, |frame, cx| {
                frame.poll();
                cx.notify();
            })
            .ok();
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
        })
        .detach();
        Self {
            bridge,
            pending: None,
            settings,
            settings_open: false,
            focus: cx.focus_handle(),
            surface: (0, 0),
            held: Modifiers::default(),
            capslock: false,
            status: None,
            image: None,
            ready: false,
            error,
            guest: None,
            surface_demo: env::var_os("TRY_SURFACE").is_some(),
            wanted: None,
            shape_note: None,
        }
    }

    fn send(&self, input: Input) {
        if let Some(bridge) = &self.bridge {
            let _ = bridge.input.send(input);
        }
    }

    fn sync_modifiers(&mut self, modifiers: Modifiers) {
        for (before, after, code) in [
            (self.held.shift, modifiers.shift, SHIFT),
            (self.held.control, modifiers.control, CONTROL),
            (self.held.alt, modifiers.alt, ALT),
            (self.held.platform, modifiers.platform, SUPER),
        ] {
            if after != before {
                self.send(Input::Key { code, down: after });
            }
        }
        self.held = modifiers;
    }

    fn key(&mut self, keystroke: &Keystroke, down: bool) {
        if self.settings_open || (down && is_settings_toggle(keystroke)) {
            return;
        }
        if down {
            self.sync_modifiers(keystroke.modifiers);
        }
        if let Some(code) = keycode(&keystroke.key) {
            self.send(Input::Key { code, down });
        }
    }

    fn modifiers_changed(&mut self, event: &ModifiersChangedEvent) {
        self.sync_modifiers(event.modifiers);
        if event.capslock.on != self.capslock {
            self.capslock = event.capslock.on;
            self.send(Input::Key {
                code: CAPS_LOCK,
                down: true,
            });
            self.send(Input::Key {
                code: CAPS_LOCK,
                down: false,
            });
        }
    }

    fn pointer(&mut self, position: Point<Pixels>, viewport: Size<Pixels>) {
        if self.settings_open {
            return;
        }
        if let Some((x, y)) = self.guest_position(position, viewport) {
            self.send(Input::Move { x, y });
        }
    }

    fn button(
        &mut self,
        button: MouseButton,
        down: bool,
        position: Point<Pixels>,
        viewport: Size<Pixels>,
    ) {
        if self.settings_open {
            return;
        }
        let button = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            _ => return,
        };
        if down {
            if let Some((x, y)) = self.guest_position(position, viewport) {
                self.send(Input::Move { x, y });
            }
        }
        self.send(Input::Button { button, down });
    }

    fn wheel(&mut self, delta: ScrollDelta) {
        if self.settings_open {
            return;
        }
        let up = match delta {
            ScrollDelta::Lines(point) => point.y,
            ScrollDelta::Pixels(point) => point.y.to_f64() as f32,
        };
        if up != 0.0 {
            self.send(Input::Wheel { up: up > 0.0 });
        }
    }

    fn guest_position(
        &self,
        position: Point<Pixels>,
        viewport: Size<Pixels>,
    ) -> Option<(u32, u32)> {
        guest_position(self.surface, viewport, position)
    }

    fn poll(&mut self) {
        let events: Vec<Event> = self
            .bridge
            .as_ref()
            .map(|bridge| bridge.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Frame {
                    width,
                    height,
                    pixels,
                } => {
                    self.surface = (width, height);
                    self.image = RgbaImage::from_raw(width, height, pixels).map(|buffer| {
                        Arc::new(RenderImage::new(SmallVec::from_elem(
                            ImageFrame::new(buffer),
                            1,
                        )))
                    });
                    self.error = None;
                    let note = frame_shape(self.surface, self.wanted);
                    if note != self.shape_note {
                        if let Some(note) = &note {
                            println!("{note}");
                        }
                        self.shape_note = note;
                    }
                }
                Event::Ready => self.ready = true,
                Event::Error(error) => self.error = Some(error),
            }
        }
        if let Some(result) = self
            .pending
            .as_ref()
            .and_then(|pending| pending.try_recv().ok())
        {
            self.pending = None;
            self.status = None;
            match result {
                Ok(bridge) => self.bridge = Some(bridge),
                Err(error) => {
                    self.ready = false;
                    self.error = Some(error);
                }
            }
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>, reset: bool) {
        write_settings(&self.settings);
        self.restart(cx, reset);
    }

    fn restart(&mut self, cx: &mut Context<Self>, reset: bool) {
        self.bridge = None;
        self.pending = Some(spawn_bridge(reset));
        self.image = None;
        self.ready = false;
        self.error = None;
        self.status = Some(
            if reset {
                "rebuilding the guest disk"
            } else {
                "restarting the guest"
            }
            .to_string(),
        );
        cx.notify();
    }

    fn demo(&mut self, viewport: Size<Pixels>, scale: f32) -> AnyElement {
        let size = (
            (viewport.width.to_f64() * scale as f64).round().max(1.0) as u32,
            (viewport.height.to_f64() * scale as f64).round().max(1.0) as u32,
        );
        if self.guest.as_ref().map(GuestSurface::size) != Some(size) {
            self.guest = GuestSurface::new(size.0, size.1).ok();
            if let Some(guest) = &self.guest {
                guest.fill();
                println!(
                    "try: pixel buffer {}x{} iosurface {}",
                    size.0,
                    size.1,
                    guest.io_surface_id()
                );
            }
        }
        let Some(guest) = &self.guest else {
            return div()
                .flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(rgb(0xff7777))
                .child("no pixel buffer")
                .into_any_element();
        };
        guestSurface(guest)
            .w(viewport.width)
            .h(viewport.height)
            .into_any_element()
    }

    fn settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cores = self.settings.cores;
        let memory = self.settings.mem.clone();
        let audio = self.settings.audio;
        let status = self.status.clone().unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .p_8()
            .bg(rgb(0x121212))
            .text_color(rgb(0xdddddd))
            .child(div().text_lg().child("try settings"))
            .child(button("cores-down", "-").on_click(cx.listener(
                |frame, _: &ClickEvent, _, cx| {
                    frame.settings.cores = frame.settings.cores.saturating_sub(1).max(1);
                    frame.apply(cx, false);
                },
            )))
            .child(div().flex_none().child(format!("{cores} cores")))
            .child(
                button("cores-up", "+").on_click(cx.listener(|frame, _: &ClickEvent, _, cx| {
                    frame.settings.cores = (frame.settings.cores + 1).min(host_cores());
                    frame.apply(cx, false);
                })),
            )
            .child(
                button("mem-down", "-").on_click(cx.listener(|frame, _: &ClickEvent, _, cx| {
                    let index = memory_index(&frame.settings.mem).saturating_sub(1);
                    frame.settings.mem = MEMORY[index].to_string();
                    frame.apply(cx, false);
                })),
            )
            .child(div().flex_none().child(format!("{memory} ram")))
            .child(
                button("mem-up", "+").on_click(cx.listener(|frame, _: &ClickEvent, _, cx| {
                    let index = (memory_index(&frame.settings.mem) + 1).min(MEMORY.len() - 1);
                    frame.settings.mem = MEMORY[index].to_string();
                    frame.apply(cx, false);
                })),
            )
            .child(
                button("audio", if audio { "audio on" } else { "audio off" }).on_click(
                    cx.listener(|frame, _: &ClickEvent, _, cx| {
                        frame.settings.audio = !frame.settings.audio;
                        frame.apply(cx, false);
                    }),
                ),
            )
            .child(
                button("reset", "reset guest")
                    .on_click(cx.listener(|frame, _: &ClickEvent, _, cx| frame.apply(cx, true))),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x888888))
                    .child(if status.is_empty() {
                        "saved, applied on restart".to_string()
                    } else {
                        status
                    }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x666666))
                    .child("cmd-, to close"),
            )
    }
}

fn button(id: &'static str, label: impl Into<String>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_2()
        .py_1()
        .rounded_md()
        .bg(rgb(0x333333))
        .text_color(rgb(0xeeeeee))
        .cursor_pointer()
        .child(label.into())
}

impl Render for Frame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let scale = window.scale_factor();
        /*
         * The window's size leaves for the guest on every frame, whatever it is showing:
         * the guest's boot mode is the whole screen, so holding it back until the first
         * frame arrives would letterbox the guest inside a smaller window.
         */
        let wanted = window_size(viewport, scale);
        self.wanted = Some(wanted);
        if let Some(bridge) = &self.bridge {
            bridge.set_window_size(wanted);
        }
        let content = if self.settings_open {
            self.settings(cx).into_any_element()
        } else if self.surface_demo {
            self.demo(viewport, scale)
        } else {
            match (&self.image, &self.error, self.ready) {
                (Some(image), _, true) => {
                    /*
                     * The frame covers the window: drawn at the window's size with its
                     * aspect kept and the overflow cropped evenly off both sides, so the
                     * guest fills the window without ever being squeezed, and a guest that
                     * is the size of the window is drawn one guest pixel per device pixel
                     * with nothing cropped at all.  `frame_shape` reports the case where a
                     * resize never arrived, which is a guest that did not adopt it.
                     */
                    let (width, height) = frame_size(viewport);
                    img(ImageSource::Render(image.clone()))
                        .object_fit(ObjectFit::Cover)
                        .w(px(width))
                        .h(px(height))
                        .into_any_element()
                }
                (_, Some(error), _) => div()
                    .flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0xff7777))
                    .child(error.clone())
                    .into_any_element(),
                _ => div()
                    .flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0x111111))
                    .text_color(rgb(0xffffff))
                    .child("waiting for Linux graphical session")
                    .into_any_element(),
            }
        };
        let mut root = div()
            .id("frame")
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |frame, event: &KeyDownEvent, _, _| {
                frame.key(&event.keystroke, true)
            }))
            .on_key_up(cx.listener(move |frame, event: &KeyUpEvent, _, _| {
                frame.key(&event.keystroke, false)
            }))
            .on_modifiers_changed(
                cx.listener(move |frame, event: &ModifiersChangedEvent, _, _| {
                    frame.modifiers_changed(event)
                }),
            )
            .on_mouse_move(cx.listener(move |frame, event: &MouseMoveEvent, _, _| {
                frame.pointer(event.position, viewport)
            }))
            .on_scroll_wheel(
                cx.listener(move |frame, event: &ScrollWheelEvent, _, _| frame.wheel(event.delta)),
            );
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            root = root
                .on_mouse_down(
                    button,
                    cx.listener(move |frame, event: &MouseDownEvent, _, _| {
                        frame.button(button, true, event.position, viewport)
                    }),
                )
                .on_mouse_up(
                    button,
                    cx.listener(move |frame, event: &MouseUpEvent, _, _| {
                        frame.button(button, false, event.position, viewport)
                    }),
                );
        }
        root.child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_file_matches_the_xmake_reader() {
        let dir = env::temp_dir().join("try-ui-settings-test");
        let _ = std::fs::remove_dir_all(&dir);
        env::set_var("TRY_STATE_DIR", &dir);
        let settings = Settings {
            cores: 6,
            mem: "12G".into(),
            audio: false,
        };
        write_settings(&settings);
        assert_eq!(
            std::fs::read_to_string(dir.join("settings")).unwrap(),
            "cores=6\nmem=12G\naudio=off\n"
        );
        let read = read_settings();
        assert_eq!(read.cores, 6);
        assert_eq!(read.mem, "12G");
        assert!(!read.audio);
        assert_eq!(MEMORY[memory_index(&read.mem)], "12G");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_frame_is_one_guest_pixel_per_device_pixel_and_fills_the_window() {
        let surface = (2560, 1440);
        let viewport = gpui::size(gpui::px(1280.0), gpui::px(720.0));
        assert_eq!(frame_size(viewport), (1280.0, 720.0));
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
        assert_eq!(frame_size(wider), (1000.0, 720.0));
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
        assert_eq!(
            frame_shape(capped, Some(window_size(viewport, 2.0))).is_some(),
            true
        );
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

    #[test]
    fn keycodes_are_xt_set1_with_the_extended_bit() {
        assert_eq!(keycode("h"), Some(0x23));
        assert_eq!(keycode("a"), Some(0x1e));
        assert_eq!(keycode("escape"), Some(0x01));
        assert_eq!(keycode("space"), Some(0x39));
        assert_eq!(keycode("up"), Some(0x80 | 0x48));
        assert_eq!(keycode("delete"), Some(0x80 | 0x53));
        assert_eq!(keycode("nonsense"), None);
    }
}

/*
 * The frame path needs no window to be checked, so TRY_GRAB=<png> runs the display
 * bridge on its own and writes the frame the guest sent into that file.  With
 * TRY_GRAB_SIZE=<width>x<height>, in the same points a window is that big, it asks for
 * that size first, waits for a frame to come back at the size the guest can adopt - the
 * request is clamped to the display the booted device advertises, as the window's is -
 * and fails if the guest never gets there.  TRY_GRAB_RESIZE=<width>x<height> asks
 * *again* TRY_GRAB_WAIT seconds in, which is what dragging a window does to a guest that
 * is already up, and the frame is written TRY_GRAB_SETTLE seconds after the last ask.  A
 * run therefore says both that frames arrive and what size the guest took, without a
 * screen to look at.
 */
fn grab(path: PathBuf) {
    let bridge = match start_bridge() {
        Ok(bridge) => bridge,
        Err(error) => {
            eprintln!("grab: {error}");
            exit(1);
        }
    };
    let first = grab_size("TRY_GRAB_SIZE");
    let resize = grab_size("TRY_GRAB_RESIZE");
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
    let mut resized = false;
    let deadline = Instant::now() + Duration::from_secs(GRAB_TIMEOUT);
    let mut last = None;
    while Instant::now() < deadline {
        if let Some((width, height)) = resize.filter(|_| !resized) {
            if Instant::now() >= due {
                wanted = Some(ask_size(&bridge, width, height));
                println!("grab: asking for {width}x{height} points");
                resized = true;
                due = Instant::now() + Duration::from_secs(grab_settle());
            }
        }
        match bridge.events.recv_timeout(Duration::from_millis(250)) {
            Ok(Event::Frame {
                width,
                height,
                pixels,
            }) => {
                println!("grab: frame {width}x{height}");
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
        if Instant::now() >= due && caught_up && (resize.is_none() || resized) {
            break;
        }
    }
    let Some((width, height, pixels)) = last else {
        eprintln!("grab: no frame arrived");
        exit(1);
    };
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("ppm") => write_ppm(&path, width, height, &pixels),
        _ => {
            let image =
                RgbaImage::from_raw(width, height, pixels).expect("frame is not a whole image");
            image.save(&path).expect("could not write the frame");
        }
    }
    println!("grab: wrote {width}x{height} to {}", path.display());
    if let Some((ask_width, ask_height)) = wanted {
        assert!(
            (ask_width, ask_height) == (width, height),
            "asked for {ask_width}x{ask_height}, guest sent {width}x{height}"
        );
    }
}

/* a raw dump, so a frame can be compared against another one without a decoder */
fn write_ppm(path: &Path, width: u32, height: u32, pixels: &[u8]) {
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.reserve(pixels.len());
    for pixel in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(path, ppm).expect("could not write the frame");
}

fn grab_size(name: &str) -> Option<(u32, u32)> {
    let requested = env::var(name).ok()?;
    let (width, height) = requested.split_once('x')?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

fn grab_wait() -> u64 {
    env::var("TRY_GRAB_WAIT")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(0)
}

fn grab_settle() -> u64 {
    env::var("TRY_GRAB_SETTLE")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(20)
}

/* asks for a window of this many points and answers the size the guest can adopt */
fn ask_size(bridge: &Bridge, width: u32, height: u32) -> (u32, u32) {
    let requested = window_size(size(px(width as f32), px(height as f32)), GRAB_SCALE);
    let wanted = clamp_to_display((requested.width, requested.height), GUEST_MAX_DISPLAY);
    *bridge.window_size.lock().unwrap() = Some(requested);
    wanted
}

fn main() {
    if let Some(path) = env::var_os("TRY_GRAB") {
        grab(PathBuf::from(path));
        return;
    }
    application().run(|cx: &mut App| {
        let _ = DISPLAY_MAX.set(host_display_max(cx));
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let bounds = Bounds::centered(None, size(gpui::px(1280.0), gpui::px(720.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let frame = cx.new(|cx| Frame::new(cx));
                let focus = frame.read(cx).focus.clone();
                focus.focus(window, cx);
                frame
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
