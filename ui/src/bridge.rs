use async_std::task;
use std::{
    fs::OpenOptions, process::Command, sync::{mpsc::{self, Receiver, Sender}, Arc, Mutex},
    thread, time::{Duration, Instant},
};
use crate::dbusSession::connect_display;
use crate::geometry::{display_max, WindowSize};
use crate::host::run_host;
use crate::input::Input;
use crate::mouseButtons;
use crate::paths::{project_dir, serial_log};
use crate::qemu::{qemu_command, Qemu};
use crate::resize::{Resize, SharedResize};

pub(crate) enum Event {
    Frame {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    Ready,
    Error(String),
}

pub(crate) struct Bridge {
    pub(crate) events: Receiver<Event>,
    pub(crate) resize: SharedResize,
    pub(crate) input: Sender<Input>,
    pub(crate) qemu: Qemu,
}

impl Bridge {
    /* the window is this size now, and this is the radius of the blur it wears for it */
    pub(crate) fn resized(&self, size: WindowSize, now: Instant) -> Option<f32> {
        let mut resize = self.resize.lock().unwrap();

        resize.changed(size, now, mouseButtons::held());
        resize.blur(now)
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

pub(crate) fn start_bridge() -> Result<Bridge, String> {
    let project = project_dir();
    let (program, argv, qemu_env) = qemu_command(&project)?;
    let display = display_max(&argv);
    let (events_tx, events_rx) = mpsc::channel();
    let (input_tx, input_rx) = mpsc::channel();
    let resize: SharedResize = Arc::new(Mutex::new(Resize::new()));
    let thread_resize = resize.clone();
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
            thread_resize,
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
        resize,
        input: input_tx,
        qemu,
    })
}

pub(crate) fn spawn_bridge(reset: bool) -> Receiver<Result<Bridge, String>> {
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
