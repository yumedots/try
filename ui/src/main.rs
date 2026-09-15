use async_std::task;
use gpui::{
    div, img, prelude::*, rgb, size, App, Bounds, Context, ImageSource, Render, RenderImage,
    Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;
use image::{Frame as ImageFrame, RgbaImage};
use smallvec::SmallVec;
use std::{
    env,
    path::PathBuf,
    process::{Child, Command},
    fs::OpenOptions,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

enum Event {
    Frame { width: u32, height: u32, pixels: Vec<u8> },
    Ready,
    Error(String),
}

const GUEST_MAX_DISPLAY: (u32, u32) = (2560, 1440);
const RESIZE_SETTLE: Duration = Duration::from_millis(150);
const RESIZE_TIMEOUT: Duration = Duration::from_millis(700);
const RESIZE_POLL: Duration = Duration::from_millis(200);

type Qemu = Arc<Mutex<Option<Child>>>;

struct Frame {
    events: Receiver<Event>,
    resize: Sender<(u32, u32)>,
    qemu: Qemu,
    image: Option<Arc<RenderImage>>,
    ready: bool,
    error: Option<String>,
}

impl Drop for Frame {
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

fn qemu_path() -> String {
    env::var("TRY_QEMU").unwrap_or_else(|_| "qemu-system-aarch64".into())
}

fn qemu_args(project: &PathBuf) -> Vec<String> {
    let root = env::var_os("TRY_GUEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.join(".cache/root"));
    let disk = env::var_os("TRY_DISK")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.join("build/rootfs.qcow2"));
    let serial_log = env::var_os("TRY_SERIAL_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.join("build/guest.log"));
    vec![
        "-machine".into(),
        "virt,accel=hvf,highmem=on".into(),
        "-cpu".into(),
        "host".into(),
        "-smp".into(),
        "8".into(),
        "-m".into(),
        "8G".into(),
        "-kernel".into(),
        root.join("boot/Image").display().to_string(),
        "-initrd".into(),
        root.join("boot/initramfs-linux.img").display().to_string(),
        "-append".into(),
        "root=/dev/vda rw console=tty0 console=ttyAMA0".into(),
        "-drive".into(),
        format!(
            "if=virtio,format=qcow2,file={}",
            disk.display()
        ),
        "-device".into(),
        format!(
            "virtio-gpu-pci,xres={},yres={}",
            GUEST_MAX_DISPLAY.0, GUEST_MAX_DISPLAY.1
        ),
        "-device".into(),
        "virtio-rng-pci".into(),
        "-device".into(),
        "qemu-xhci".into(),
        "-device".into(),
        "usb-kbd".into(),
        "-device".into(),
        "usb-tablet".into(),
        "-netdev".into(),
        "user,id=net0,hostfwd=tcp::2222-:22".into(),
        "-device".into(),
        "virtio-net-pci,netdev=net0".into(),
        "-serial".into(),
        format!("file:{}", serial_log.display()),
        "-no-reboot".into(),
        "-display".into(),
        "dbus,gl=off".into(),
    ]
}

fn start_bridge() -> (Receiver<Event>, Sender<(u32, u32)>, Qemu) {
    let (events_tx, events_rx) = mpsc::channel();
    let (resize_tx, resize_rx) = mpsc::channel();
    let qemu: Qemu = Arc::new(Mutex::new(None));
    let thread_qemu = qemu.clone();
    let project = PathBuf::from(env::var("TRY_PROJECT_DIR").unwrap_or_else(|_| ".".into()));

    thread::spawn(move || {
        let serial_log = env::var_os("TRY_SERIAL_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|| project.join("build/guest.log"));
        if let Err(error) = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&serial_log)
        {
            let _ = events_tx.send(Event::Error(format!("could not reset the guest log: {error}")));
            return;
        }
        match Command::new(qemu_path()).args(qemu_args(&project)).spawn() {
            Ok(child) => *thread_qemu.lock().unwrap() = Some(child),
            Err(error) => {
                let _ = events_tx.send(Event::Error(format!("could not start QEMU: {error}")));
                return;
            }
        }
        println!("started headless QEMU for GPUI display");
        let result = task::block_on(connect_display(
            events_tx.clone(),
            resize_rx,
            thread_qemu.clone(),
            serial_log,
        ));
        if let Err(error) = result {
            let _ = events_tx.send(Event::Error(error));
        }
    });

    (events_rx, resize_tx, qemu)
}

fn qemu_exited(qemu: &Qemu) -> bool {
    qemu.lock()
        .unwrap()
        .as_mut()
        .map(|child| !matches!(child.try_wait(), Ok(None)))
        .unwrap_or(false)
}

async fn connect_display(
    events: Sender<Event>,
    resize: Receiver<(u32, u32)>,
    qemu: Qemu,
    serial_log: PathBuf,
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
        if qemu_exited(&qemu) {
            return Err("QEMU exited before its D-Bus display became available".into());
        }
        if !dbus.name_has_owner(qemu_name.clone()).await.unwrap_or(false) {
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
        .path(console_path)
        .map_err(|error| format!("could not create the QEMU console path: {error}"))?
        .build()
        .await
        .map_err(|error| format!("could not open the QEMU console: {error}"))?;
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
    let mut latest = None;
    let mut settled = Instant::now();
    let mut guest_mode = None;
    let mut compositor = false;
    let mut polled = Instant::now();
    let mut pending = None;
    let mut pending_since = Instant::now();
    loop {
        if !ready && guest_display_ready(&serial_log) {
            ready = true;
            let _ = events.send(Event::Ready);
            println!("guest display readiness service completed");
        }
        if ready && polled.elapsed() >= RESIZE_POLL {
            polled = Instant::now();
            if let Some(mode) = guest_display_mode(&serial_log) {
                guest_mode = Some(mode);
            }
            if let Some(session) = guest_display_session(&serial_log) {
                let running = session == "wayland";
                if running != compositor {
                    println!("guest session is {session}");
                }
                compositor = running;
            }
        }
        if ready && pending.is_some() {
            if let Some(asked) = pending {
                if guest_mode == Some(asked) {
                    pending = None;
                } else if pending_since.elapsed() >= RESIZE_TIMEOUT {
                    if let Some(mode) = guest_mode {
                        console
                            .set_ui_info(0, 0, 0, 0, mode.0, mode.1)
                            .await
                            .map_err(|error| format!("could not resize the QEMU display: {error}"))?;
                        println!("guest stayed at {}x{}, letting it scale", mode.0, mode.1);
                        requested = Some(mode);
                        latest = None;
                    }
                    pending = None;
                }
            }
        }
        for size in resize.try_iter() {
            latest = Some(size);
            settled = Instant::now();
        }
        let target = if !ready || !compositor {
            guest_mode.or(Some(GUEST_MAX_DISPLAY))
        } else if settled.elapsed() >= RESIZE_SETTLE {
            latest
        } else {
            requested
        };
        if target != requested {
            if let Some((width, height)) = target {
                console
                    .set_ui_info(0, 0, 0, 0, width, height)
                    .await
                    .map_err(|error| format!("could not resize the QEMU display: {error}"))?;
                println!("requested guest display resize: {width}x{height}");
                pending = if ready && compositor { target } else { None };
                pending_since = Instant::now();
                requested = target;
            }
        }
        if qemu_exited(&qemu) {
            return Err("QEMU exited".into());
        }
        task::sleep(Duration::from_millis(16)).await;
    }
}

fn guest_display_session(serial_log: &PathBuf) -> Option<String> {
    let log = std::fs::read_to_string(serial_log).ok()?;
    let tail = log.rsplit_once("tryDisplay: session ")?.1;
    Some(tail.lines().next()?.trim().to_string())
}

fn guest_display_mode(serial_log: &PathBuf) -> Option<(u32, u32)> {
    let log = std::fs::read_to_string(serial_log).ok()?;
    let tail = log.rsplit_once("tryDisplay: mode ")?.1;
    let (width, height) = tail.lines().next()?.split_once('x')?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

fn guest_display_ready(serial_log: &PathBuf) -> bool {
    std::fs::read_to_string(serial_log)
        .map(|log| log.contains("TRY_DISPLAY_READY=1"))
        .unwrap_or(false)
}

impl Frame {
    fn new(cx: &mut Context<Self>) -> Self {
        let (events, resize, qemu) = start_bridge();
        cx.spawn(async move |this, cx| {
            loop {
                this.update(cx, |frame, cx| {
                    for event in frame.events.try_iter() {
                        match event {
                            Event::Frame {
                                width,
                                height,
                                pixels,
                            } => {
                                frame.image = RgbaImage::from_raw(width, height, pixels).map(
                                    |buffer| {
                                        Arc::new(RenderImage::new(SmallVec::from_elem(
                                            ImageFrame::new(buffer),
                                            1,
                                        )))
                                    },
                                );
                                frame.error = None;
                            }
                            Event::Ready => frame.ready = true,
                            Event::Error(error) => frame.error = Some(error),
                        }
                    }
                    cx.notify();
                })
                .ok();
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
        })
        .detach();
        Self {
            events,
            resize,
            qemu,
            image: None,
            ready: false,
            error: None,
        }
    }
}

impl Render for Frame {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let content = match (&self.image, &self.error, self.ready) {
            (Some(image), _, true) => img(ImageSource::Render(image.clone()))
                .size_full()
                .into_any_element(),
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
        };
        div().size_full().bg(rgb(0x000000)).child(content)
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let bounds = Bounds::centered(None, size(gpui::px(960.0), gpui::px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let frame = Frame::new(cx);
                    cx.observe_window_bounds(window, |frame, window, _| {
                        let viewport = window.viewport_size();
                        let _ = frame.resize.send((
                            viewport.width.to_f64() as u32,
                            viewport.height.to_f64() as u32,
                        ));
                    })
                    .detach();
                    frame
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
