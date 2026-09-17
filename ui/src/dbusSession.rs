use async_std::task;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc::{Receiver, Sender}, Mutex, OnceLock},
    time::{Duration, Instant},
};
use crate::bridge::Event;
use crate::dbusDisplay::{
    ConsoleProxy, DisplayListener, Listener, Scanout, UiInfoProxy, Update, VMProxy,
};
use crate::panel::refresh_rate;
use zbus::zvariant::Value;
use crate::geometry::{clamp_to_display, opening_size};
use crate::input::{send_button, Input, KeyboardProxy, MouseProxy, WHEEL_DOWN, WHEEL_UP};
use crate::qemu::{qemu_stopped, Qemu};
use crate::resize::SharedResize;
use crate::surfaceRing::SharedRing;

pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(200);

/* how soon the guest hears about a key, a click or a hand-over the window asked for */
pub(crate) const INPUT_POLL: Duration = Duration::from_millis(8);

pub(crate) static LAST_FRAME: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

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

    #[allow(clippy::too_many_arguments)]
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

pub(crate) async fn connect_display(
    events: Sender<Event>,
    resize: SharedResize,
    ring: SharedRing,
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
        .path(console_path.clone())
        .map_err(|error| format!("could not create the QEMU mouse path: {error}"))?
        .build()
        .await
        .map_err(|error| format!("could not open the QEMU mouse: {error}"))?;
    let ui_info = UiInfoProxy::builder(&connection)
        .path(console_path)
        .map_err(|error| format!("could not create the QEMU UI info path: {error}"))?
        .build()
        .await
        .map_err(|error| format!("could not open the QEMU UI info: {error}"))?;
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
                    ring: ring.clone(),
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
    let panel = refresh_rate();
    loop {
        if !ready && polled.elapsed() >= POLL_INTERVAL {
            polled = Instant::now();
            if guest_display_ready(&serial_log) {
                ready = true;
                let _ = events.send(Event::Ready);
                println!("guest display readiness service completed");
            }
        }
        /*
         * A frame landed, so the console is owed the next surface of the ring: it is told
         * once per frame rather than once, which is what keeps the surface it writes into
         * out from under the one the window is drawing.
         */
        let handover = ring.lock().unwrap().take_request();
        if let Some(name) = handover {
            if let Err(error) = console.set_surface(&name).await {
                println!("guest surface handover failed: {error}");
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
        let now = Instant::now();
        let (wanted, landed) = {
            let tracked = resize.lock().unwrap();

            (tracked.size(), tracked.landed(now))
        };
        if let Some(size) = wanted {
            let size = opening_size(size, display, ready);
            if landed && requested != Some(size) {
                let (width, height) = clamp_to_display((size.width, size.height), display);
                let mut info = HashMap::new();

                info.insert("width_mm", Value::U16(size.width_mm));
                info.insert("height_mm", Value::U16(size.height_mm));
                info.insert("xoff", Value::I32(0));
                info.insert("yoff", Value::I32(0));
                info.insert("width", Value::U32(width));
                info.insert("height", Value::U32(height));
                if panel != 0 {
                    info.insert("refresh_rate", Value::U32(panel));
                }
                ui_info
                    .apply(info)
                    .await
                    .map_err(|error| format!("could not resize the QEMU display: {error}"))?;
                println!("requested guest display resize: {width}x{height} at {panel} millihertz");
                requested = Some(size);
            }
        }
        if let Some(status) = qemu_stopped(&qemu) {
            return Err(format!("QEMU exited: {status}"));
        }
        task::sleep(INPUT_POLL).await;
    }
}

pub(crate) fn guest_display_ready(serial_log: &PathBuf) -> bool {
    std::fs::read_to_string(serial_log)
        .map(|log| log.contains("TRY_DISPLAY_READY=1"))
        .unwrap_or(false)
}
