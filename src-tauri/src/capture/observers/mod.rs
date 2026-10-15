use motion_bench::capture::clock;
use motion_bench::sync::LockExt;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::JoinHandle,
};

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub raw_ready: bool,
    pub kernel_ready: bool,
    pub kernel_seen: bool,
    pub recording: bool,
    pub reports: u64,
    pub batches: u64,
    pub max_batch: usize,
    pub queue_losses: u64,
    pub kernel_losses: u64,
    pub kernel_prior_losses: u64,
    pub kernel_history_reports: u64,
    pub os_loss_count_known: bool,
    pub device_changes: u64,
    pub error: Option<String>,
    pub kernel_status: String,
    pub devices: BTreeMap<u64, String>,
    pub kernel_devices: BTreeMap<u64, String>,
}
#[derive(Clone, Serialize)]
pub struct Packet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel: Option<KernelPacket>,
    pub device: u64,
    pub flags: u16,
    pub buttons: u16,
    pub button_data: u16,
    pub dx: i32,
    pub dy: i32,
    pub extra: u32,
}
#[derive(Clone, Serialize)]
pub struct KernelPacket {
    pub sequence: u64,
    pub batch: u64,
    pub index: u32,
    pub count: u32,
    pub unit: u16,
}
#[derive(Serialize)]
pub struct DeviceChange {
    pub device: u64,
    pub action: &'static str,
    pub previous_name: Option<String>,
    pub current_name: Option<String>,
}
#[derive(Serialize)]
pub struct Batch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_change: Option<DeviceChange>,
    pub boundary: &'static str,
    pub source: &'static str,
    pub sequence: u64,
    pub before_qpc: i64,
    pub after_qpc: i64,
    pub packets: Vec<Packet>,
}
struct Sink {
    path: PathBuf,
    /// the recorded mouse as Windows names it, `VID_xxxx&PID_xxxx`
    only: String,
    raw: BufWriter<File>,
    kernel: BufWriter<File>,
    anchors: BufWriter<File>,
    monitor: clock::Monitor,
    initial: Status,
    started_qpc: i64,
    ended_qpc: Option<i64>,
    anchor: clock::Anchor,
    reports: u64,
    batches: u64,
    max_batch: usize,
    boundary_batches: [u64; 2],
    kernel_reports: u64,
    kernel_batches: u64,
}
struct Control {
    status: Status,
    sink: Option<Sink>,
}
impl Control {
    fn finalize(&mut self) -> Result<(), String> {
        self.status.recording = false;
        let Some(mut sink) = self.sink.take() else {
            return Ok(());
        };
        let anchor = clock::Anchor::take();
        if let Err(error) = sink.monitor.observe(&anchor, clock::frequency()) {
            self.status.error = Some(error);
        }
        let written = (|| -> std::io::Result<()> {
            serde_json::to_writer(&mut sink.anchors, &anchor)?;
            sink.anchors.write_all(b"\n")?;
            for file in [&mut sink.raw, &mut sink.kernel, &mut sink.anchors] {
                file.flush()?;
                file.get_ref().sync_all()?;
            }
            Ok(())
        })();
        let error = self
            .status
            .error
            .clone()
            .or_else(|| written.err().map(|e| e.to_string()));
        let summary = serde_json::json!({
            "schema":"motion-bench.observers.v1", "complete":error.is_none(), "error":error,
            "qpc_frequency":clock::frequency(), "started_qpc":sink.started_qpc,
            "ended_qpc":sink.ended_qpc.unwrap_or_else(clock::qpc), "anchor":sink.anchor, "clocks":sink.monitor.summary,
            "queue_losses":self.status.queue_losses.saturating_sub(sink.initial.queue_losses),
            "raw_input":{"observation":"user_space_receipt_batch", "os_loss_count_known":false,
                "reports":sink.reports, "batches":sink.batches,
                "max_batch":sink.max_batch,
                "device_changes":self.status.device_changes.saturating_sub(sink.initial.device_changes),
                "boundary_batches":sink.boundary_batches[0],
                "devices":self.status.devices.iter().filter(|(_, name)| recorded(name, &sink.only)).collect::<BTreeMap<_, _>>()},
            "kernel":{"available":self.status.kernel_seen,"status":self.status.kernel_status,
                "reports":sink.kernel_reports,"batches":sink.kernel_batches,"boundary_batches":sink.boundary_batches[1],
                "losses_before_observer":self.status.kernel_prior_losses,
                "startup_discarded_reports":self.status.kernel_history_reports,
                "losses":self.status.kernel_losses.saturating_sub(sink.initial.kernel_losses), "devices":self.status.kernel_devices},
            "cross_layer_device_mapping_verified":false,
        });
        let temp = sink.path.with_extension("json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(&summary).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temp, sink.path).map_err(|e| e.to_string())?;
        if let Some(error) = error {
            self.status.error = Some(error.clone());
            return Err(error);
        }
        Ok(())
    }
}
fn write_batches(
    shared: &Mutex<Control>,
    lost: &AtomicU64,
    cached: &Mutex<Status>,
    rx: mpsc::Receiver<Batch>,
) {
    let mut last_anchor = 0;
    loop {
        let batch = match rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(batch) => Some(batch),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut c = shared.locked();
        note_losses(&mut c.status, lost);
        let result = (|| -> Result<(), String> {
            if let Some(mut batch) = batch {
                if let Some(change) = &mut batch.device_change {
                    c.status.device_changes += 1;
                    change.previous_name = c.status.devices.remove(&change.device);
                    if change.action == "arrival" {
                        let name = device_name(change.device);
                        c.status.devices.insert(change.device, name.clone());
                        change.current_name = Some(name);
                    }
                }
                if batch.source == "raw_input" && !batch.packets.is_empty() {
                    c.status.reports += batch.packets.len() as u64;
                    c.status.batches += 1;
                    c.status.max_batch = c.status.max_batch.max(batch.packets.len());
                    for packet in &batch.packets {
                        if c.status.devices.len() >= 256
                            && !c.status.devices.contains_key(&packet.device)
                        {
                            return Err(
                                "too many Raw Input device identities, restart capture".into()
                            );
                        }
                        c.status
                            .devices
                            .entry(packet.device)
                            .or_insert_with(|| device_name(packet.device));
                    }
                }
                // a recording only keeps the mouse it records, other mice plugged in stay out of it
                let mut theirs = false;
                if let Some(sink) = &c.sink
                    && batch.source == "raw_input"
                {
                    let had = !batch.packets.is_empty() || batch.device_change.is_some();
                    let devices = &c.status.devices;
                    batch.packets.retain(|p| {
                        devices
                            .get(&p.device)
                            .is_some_and(|name| recorded(name, &sink.only))
                    });
                    if batch.device_change.as_ref().is_some_and(|change| {
                        ![&change.previous_name, &change.current_name]
                            .into_iter()
                            .flatten()
                            .any(|name| recorded(name, &sink.only))
                    }) {
                        batch.device_change = None;
                    }
                    theirs = had && batch.packets.is_empty() && batch.device_change.is_none();
                }
                if let Some(sink) = &mut c.sink
                    && !theirs
                {
                    batch.boundary = if batch.after_qpc < sink.started_qpc {
                        "before_start"
                    } else if batch.before_qpc < sink.started_qpc {
                        "crosses_start"
                    } else if sink.ended_qpc.is_some_and(|end| batch.before_qpc > end) {
                        "after_stop"
                    } else if sink.ended_qpc.is_some_and(|end| batch.after_qpc > end) {
                        "crosses_stop"
                    } else {
                        "inside"
                    };
                    if batch.boundary != "inside" {
                        sink.boundary_batches[usize::from(batch.source != "raw_input")] += 1;
                    } else if batch.source == "raw_input" && !batch.packets.is_empty() {
                        sink.reports += batch.packets.len() as u64;
                        sink.batches += 1;
                        sink.max_batch = sink.max_batch.max(batch.packets.len());
                    } else if batch.source == "mouse_class" {
                        sink.kernel_reports += batch.packets.len() as u64;
                        sink.kernel_batches += 1;
                    }
                    let file = if batch.source == "raw_input" {
                        &mut sink.raw
                    } else {
                        &mut sink.kernel
                    };
                    serde_json::to_writer(&mut *file, &batch).map_err(|e| e.to_string())?;
                    file.write_all(b"\n").map_err(|e| e.to_string())?;
                    if batch.device_change.is_some() && batch.boundary == "inside" {
                        return Err("mouse device topology changed during recording, reselect the source and start a new capture".into());
                    }
                }
            }
            if let Some(sink) = &mut c.sink
                && clock::qpc() - last_anchor >= clock::frequency()
            {
                let anchor = clock::Anchor::take();
                last_anchor = anchor.midpoint();
                serde_json::to_writer(&mut sink.anchors, &anchor).map_err(|e| e.to_string())?;
                sink.anchors.write_all(b"\n").map_err(|e| e.to_string())?;
                sink.monitor.observe(&anchor, clock::frequency())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            c.status.error = Some(error);
        }
        *cached.locked() = c.status.clone();
    }
    let mut c = shared.locked();
    note_losses(&mut c.status, lost);
    *cached.locked() = c.status.clone();
}

/// batches the writer's queue dropped, after which the evidence cannot be whole
fn note_losses(status: &mut Status, lost: &AtomicU64) {
    status.queue_losses = lost.load(Ordering::Relaxed);
    if status.queue_losses > 0 {
        status.error = Some("observer writer queue lost batches, evidence is incomplete".into());
    }
}
/// what a recording needs of the observers, held apart from the worker so their files are made
/// without the app's lock
#[derive(Clone)]
pub struct Journal {
    control: Arc<Mutex<Control>>,
}
impl Journal {
    /// `device` is the recorded mouse's vid and pid
    pub fn begin(&self, pcap: &Path, device: (u16, u16)) -> Result<(), String> {
        let mut c = self.control.locked();
        if !c.status.raw_ready || c.status.error.is_some() || c.sink.is_some() {
            return Err("input observer is not ready, restart capture before recording".into());
        }
        let open = |suffix: &str| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(pcap.with_extension(suffix))
                .map(BufWriter::new)
                .map_err(|e| e.to_string())
        };
        let anchor = clock::Anchor::take();
        let mut monitor = clock::Monitor::default();
        monitor.observe(&anchor, clock::frequency())?;
        let mut anchors = open("observer-clocks.jsonl")?;
        serde_json::to_writer(&mut anchors, &anchor).map_err(|e| e.to_string())?;
        anchors.write_all(b"\n").map_err(|e| e.to_string())?;
        let path = pcap.with_extension("observers.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&serde_json::json!({"schema":"motion-bench.observers.v1","complete":false,"qpc_frequency":clock::frequency(),"anchor":anchor})).unwrap()).map_err(|e| e.to_string())?;
        c.sink = Some(Sink {
            path,
            only: format!("VID_{:04X}&PID_{:04X}", device.0, device.1),
            raw: open("raw-input.jsonl")?,
            kernel: open("kernel-input.jsonl")?,
            anchors,
            monitor,
            initial: c.status.clone(),
            started_qpc: anchor.qpc_after,
            ended_qpc: None,
            anchor,
            reports: 0,
            batches: 0,
            max_batch: 0,
            boundary_batches: [0; 2],
            kernel_reports: 0,
            kernel_batches: 0,
        });
        c.status.recording = true;
        Ok(())
    }
}

pub struct Worker {
    control: Arc<Mutex<Control>>,
    cached: Arc<Mutex<Status>>,
    stopping: Arc<AtomicBool>,
    losses: Arc<AtomicU64>,
    threads: Vec<JoinHandle<()>>,
}
impl Worker {
    pub fn start() -> Self {
        let control = Arc::new(Mutex::new(Control {
            status: Status::default(),
            sink: None,
        }));
        let cached = Arc::new(Mutex::new(Status::default()));
        let stopping = Arc::new(AtomicBool::new(false));
        let losses = Arc::new(AtomicU64::new(0));
        let (tx, rx) = mpsc::sync_channel::<Batch>(512);
        let (shared, lost) = (control.clone(), losses.clone());
        let cache = cached.clone();
        let writer = std::thread::spawn(move || write_batches(&shared, &lost, &cache, rx));
        let (shared, stop, lost, sender) = (
            control.clone(),
            stopping.clone(),
            losses.clone(),
            tx.clone(),
        );
        let raw = std::thread::spawn(move || {
            if let Err(error) = raw_input(&sender, &stop, &lost, &shared) {
                shared.locked().status.error = Some(error);
            }
            shared.locked().status.raw_ready = false;
        });
        let (shared, stop, lost) = (control.clone(), stopping.clone(), losses.clone());
        let kernel = std::thread::spawn(move || kernel_input(&tx, &stop, &lost, &shared));
        Self {
            cached,
            control,
            stopping,
            losses,
            threads: vec![raw, kernel, writer],
        }
    }
    /// a worker with no threads that reports `status`, for tests of what depends on it
    #[cfg(test)]
    pub(crate) fn with_status(status: Status) -> Self {
        Self {
            control: Arc::new(Mutex::new(Control { status, sink: None })),
            cached: Arc::new(Mutex::new(Status::default())),
            stopping: Arc::new(AtomicBool::new(false)),
            losses: Arc::new(AtomicU64::new(0)),
            threads: Vec::new(),
        }
    }
    pub fn snapshot(&self) -> Status {
        let control = match self.control.try_lock() {
            Ok(c) => Some(c),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        let mut status = if let Some(c) = control {
            let status = c.status.clone();
            *self.cached.locked() = status.clone();
            status
        } else {
            self.cached.locked().clone()
        };
        status.queue_losses = self.losses.load(Ordering::Relaxed);
        if status.queue_losses > 0 {
            status.error =
                Some("observer writer queue lost batches, evidence is incomplete".into());
        }
        status
    }
    pub fn journal(&self) -> Journal {
        Journal {
            control: self.control.clone(),
        }
    }
    pub fn stop(&mut self) {
        if let Some(sink) = &mut self.control.locked().sink {
            sink.ended_qpc.get_or_insert_with(clock::qpc);
        }
        self.stopping.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                self.control.locked().status.error =
                    Some("input observer worker panicked, evidence is incomplete".into());
            }
        }
        let mut c = self.control.locked();
        c.status.queue_losses = self.losses.load(Ordering::Relaxed);
        if let Err(error) = c.finalize() {
            c.status
                .error
                .get_or_insert(format!("observer finalization failed: {error}"));
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
fn device_name(device: u64) -> String {
    use windows::Win32::{
        Foundation::HANDLE,
        UI::Input::{GetRawInputDeviceInfoW, RIDI_DEVICENAME},
    };
    let mut text = vec![0u16; 1024];
    let mut len = text.len() as u32;
    let result = unsafe {
        GetRawInputDeviceInfoW(
            Some(HANDLE(device as usize as *mut _)),
            RIDI_DEVICENAME,
            Some(text.as_mut_ptr().cast()),
            &mut len,
        )
    };
    if result == u32::MAX {
        return format!("unresolved raw device {device:#x}");
    }
    String::from_utf16_lossy(&text[..(len as usize).min(text.len())])
        .trim_end_matches('\0')
        .into()
}
#[cfg(not(windows))]
fn device_name(device: u64) -> String {
    format!("{device:#x}")
}

/// whether a Raw Input device name is the recorded mouse
fn recorded(name: &str, only: &str) -> bool {
    name.to_ascii_uppercase().contains(only)
}

fn publish(tx: &SyncSender<Batch>, batch: Batch, losses: &AtomicU64) {
    if (!batch.packets.is_empty() || batch.device_change.is_some()) && tx.try_send(batch).is_err() {
        losses.fetch_add(1, Ordering::Relaxed);
    }
}
#[cfg(windows)]
fn raw_input(
    tx: &SyncSender<Batch>,
    stop: &AtomicBool,
    losses: &AtomicU64,
    state: &Mutex<Control>,
) -> Result<(), String> {
    use windows::{
        Win32::UI::{Input::*, WindowsAndMessaging::*},
        core::w,
    };
    let fail = |e: windows::core::Error| e.to_string();
    unsafe {
        let window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Motion Bench input observer"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
        .map_err(fail)?;
        let result = (|| -> Result<(), String> {
            RegisterRawInputDevices(
                &[RAWINPUTDEVICE {
                    usUsagePage: 1,
                    usUsage: 2,
                    dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
                    hwndTarget: window,
                }],
                size_of::<RAWINPUTDEVICE>() as u32,
            )
            .map_err(fail)?;
            state.locked().status.raw_ready = true;
            let mut buffer = vec![0u64; 8192];
            let mut sequence = 0;
            while !stop.load(Ordering::Relaxed) {
                MsgWaitForMultipleObjectsEx(None, 50, QS_RAWINPUT, MWMO_INPUTAVAILABLE);
                let mut msg = MSG::default();
                while PeekMessageW(
                    &mut msg,
                    Some(window),
                    WM_INPUT_DEVICE_CHANGE,
                    WM_INPUT,
                    PM_REMOVE,
                )
                .as_bool()
                {
                    let before = clock::qpc();
                    if msg.message == WM_INPUT_DEVICE_CHANGE {
                        let change = DeviceChange {
                            device: msg.lParam.0 as usize as u64,
                            action: if msg.wParam.0 == 1 {
                                "arrival"
                            } else {
                                "removal"
                            },
                            previous_name: None,
                            current_name: None,
                        };
                        sequence += 1;
                        publish(
                            tx,
                            Batch {
                                device_change: Some(change),
                                boundary: "inside",
                                source: "raw_input",
                                sequence,
                                before_qpc: before,
                                after_qpc: clock::qpc(),
                                packets: Vec::new(),
                            },
                            losses,
                        );
                        continue;
                    }
                    let mut bytes = (buffer.len() * 8) as u32;
                    if GetRawInputData(
                        HRAWINPUT(msg.lParam.0 as *mut _),
                        RID_INPUT,
                        Some(buffer.as_mut_ptr().cast()),
                        &mut bytes,
                        size_of::<RAWINPUTHEADER>() as u32,
                    ) == u32::MAX
                    {
                        DefWindowProcW(window, msg.message, msg.wParam, msg.lParam);
                        return Err(
                            "GetRawInputData failed; observer evidence is incomplete".into()
                        );
                    }
                    let mut packets = Vec::with_capacity(256);
                    let parsed = push_raw(&mut packets, &*buffer.as_ptr().cast::<RAWINPUT>());
                    DefWindowProcW(window, msg.message, msg.wParam, msg.lParam);
                    parsed?;
                    loop {
                        bytes = (buffer.len() * 8) as u32;
                        let count = GetRawInputBuffer(
                            Some(buffer.as_mut_ptr().cast()),
                            &mut bytes,
                            size_of::<RAWINPUTHEADER>() as u32,
                        );
                        if count == u32::MAX {
                            return Err(
                                "GetRawInputBuffer failed; observer evidence is incomplete".into(),
                            );
                        }
                        if count == 0 {
                            break;
                        }
                        let mut offset = 0usize;
                        for _ in 0..count {
                            if offset + size_of::<RAWINPUT>() > buffer.len() * 8 {
                                return Err("truncated Raw Input batch".into());
                            }
                            let raw = &*buffer.as_ptr().cast::<u8>().add(offset).cast::<RAWINPUT>();
                            let size = raw.header.dwSize as usize;
                            if size < size_of::<RAWINPUTHEADER>()
                                || offset + size > buffer.len() * 8
                            {
                                return Err("invalid Raw Input batch length".into());
                            }
                            push_raw(&mut packets, raw)?;
                            offset += (size + 7) & !7;
                        }
                        if packets.len() >= 4096 {
                            break;
                        }
                    }
                    sequence += 1;
                    publish(
                        tx,
                        Batch {
                            device_change: None,
                            boundary: "inside",
                            source: "raw_input",
                            sequence,
                            before_qpc: before,
                            after_qpc: clock::qpc(),
                            packets,
                        },
                        losses,
                    );
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                }
            }
            Ok(())
        })();
        let unregister = RegisterRawInputDevices(
            &[RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 2,
                dwFlags: RIDEV_REMOVE,
                hwndTarget: Default::default(),
            }],
            size_of::<RAWINPUTDEVICE>() as u32,
        )
        .map_err(fail);
        let _ = DestroyWindow(window);
        result.and(unregister)
    }
}
#[cfg(windows)]
unsafe fn push_raw(
    packets: &mut Vec<Packet>,
    raw: &windows::Win32::UI::Input::RAWINPUT,
) -> Result<(), String> {
    use windows::Win32::UI::Input::{RAWINPUTHEADER, RAWMOUSE, RIM_TYPEMOUSE};
    if raw.header.dwType != RIM_TYPEMOUSE.0 {
        return Ok(());
    }
    if (raw.header.dwSize as usize) < size_of::<RAWINPUTHEADER>() + size_of::<RAWMOUSE>() {
        return Err("truncated raw mouse packet".into());
    }
    let mouse = unsafe { raw.data.mouse };
    let buttons = unsafe { mouse.Anonymous.Anonymous };
    packets.push(Packet {
        kernel: None,
        device: raw.header.hDevice.0 as usize as u64,
        flags: mouse.usFlags.0,
        buttons: buttons.usButtonFlags,
        button_data: buttons.usButtonData,
        dx: mouse.lLastX,
        dy: mouse.lLastY,
        extra: mouse.ulExtraInformation,
    });
    Ok(())
}
#[cfg(not(windows))]
fn raw_input(
    _: &SyncSender<Batch>,
    _: &AtomicBool,
    _: &AtomicU64,
    _: &Mutex<Control>,
) -> Result<(), String> {
    Err("Raw Input requires Windows".into())
}

#[cfg(windows)]
fn kernel_input(
    tx: &SyncSender<Batch>,
    stop: &AtomicBool,
    losses: &AtomicU64,
    state: &Mutex<Control>,
) {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            handle: *mut std::ffi::c_void,
            code: u32,
            input: *const std::ffi::c_void,
            input_size: u32,
            output: *mut std::ffi::c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }
    let file = match OpenOptions::new()
        .read(true)
        .open(r"\\.\MotionBenchObserver")
    {
        Ok(file) => file,
        Err(error) => {
            let action = if error.kind() == std::io::ErrorKind::PermissionDenied {
                "the observer port needs an elevated Motion Bench process"
            } else {
                "install and qualify the signed observer package for the selected mouse"
            };
            state.locked().status.kernel_status =
                format!("kernel observer unavailable: {error}. {action}");
            return;
        }
    };
    let read = |code: u32, buffer: &mut [u8]| -> Result<usize, String> {
        let mut bytes = 0;
        let result = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                std::ptr::null(),
                0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut bytes,
                std::ptr::null_mut(),
            )
        };
        if result == 0 {
            return Err(format!(
                "kernel observer read: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(bytes as usize)
    };
    let result = (|| -> Result<(), String> {
        let mut buffer = vec![0u8; 32 + 256 * 72];
        let mut last_devices = 0;
        let mut sequence = None;
        let mut previous_losses = None;
        let mut drain_reads = 0;
        let observed_from = clock::qpc();
        loop {
            let bytes = read(0x00226000, &mut buffer)?;
            let (dropped, mut batches) = decode_kernel(&buffer[..bytes], clock::frequency())?;
            let empty = batches.is_empty();
            let history = kernel_continuity(&mut batches, observed_from, &mut sequence)?;
            {
                let mut c = state.locked();
                c.status.kernel_ready = true;
                c.status.kernel_seen = true;
                c.status.kernel_status =
                    "Mouse-class callbacks, native QPC, accepted packets".into();
                c.status.kernel_losses = dropped;
                c.status.kernel_history_reports += history;
                if previous_losses.is_none() {
                    c.status.kernel_prior_losses = dropped;
                }
                if previous_losses.is_some_and(|previous| dropped > previous) {
                    c.status.error =
                        Some("kernel observer ring lost records, evidence is incomplete".into());
                }
                previous_losses = Some(dropped);
            }
            for batch in batches {
                publish(tx, batch, losses);
            }
            if clock::qpc() - last_devices >= clock::frequency() {
                let mut devices = vec![0u8; 32 + 32 * 520];
                let bytes = read(0x00226004, &mut devices)?;
                if bytes < 32 || (bytes - 32) % 520 != 0 {
                    return Err("invalid kernel device metadata".into());
                }
                let mut c = state.locked();
                for row in devices[32..bytes].chunks_exact(520) {
                    let id = u64::from_le_bytes(row[..8].try_into().unwrap());
                    let name: Vec<u16> = row[8..]
                        .chunks_exact(2)
                        .map(|v| u16::from_le_bytes([v[0], v[1]]))
                        .take_while(|v| *v != 0)
                        .collect();
                    c.status
                        .kernel_devices
                        .insert(id, String::from_utf16_lossy(&name));
                }
                last_devices = clock::qpc();
            }
            if stop.load(Ordering::Relaxed) {
                if empty {
                    break;
                }
                drain_reads += 1;
                if drain_reads >= 129 {
                    return Err(
                        "kernel observer did not drain at shutdown, evidence is incomplete".into(),
                    );
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        Ok(())
    })();
    let mut c = state.locked();
    c.status.kernel_ready = false;
    if let Err(error) = result {
        c.status.error = Some(error.clone());
        c.status.kernel_status = error;
    }
}
fn kernel_continuity(
    batches: &mut Vec<Batch>,
    observed_from: i64,
    sequence: &mut Option<u64>,
) -> Result<u64, String> {
    let mut history = 0;
    // the driver can accumulate a full ring while no reader exists; that is outside our coverage
    batches.retain(|batch| {
        if batch.after_qpc < observed_from {
            history += batch.packets.len() as u64;
            false
        } else {
            true
        }
    });
    for batch in batches {
        for packet in &batch.packets {
            let next = packet.kernel.as_ref().unwrap().sequence;
            if sequence.is_some_and(|previous| previous.checked_add(1) != Some(next)) {
                return Err("kernel observer sequence gap".into());
            }
            *sequence = Some(next);
        }
    }
    Ok(history)
}
fn decode_kernel(bytes: &[u8], frequency: i64) -> Result<(u64, Vec<Batch>), String> {
    if bytes.len() < 32 {
        return Err("truncated kernel observer header".into());
    }
    let u32_at = |p: usize| u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
    let u64_at = |p: usize| u64::from_le_bytes(bytes[p..p + 8].try_into().unwrap());
    let count = u32_at(4) as usize;
    if frequency <= 0
        || u32_at(0) != 1
        || u64_at(8) as i64 != frequency
        || count > 256
        || bytes.len() != 32 + count * 72
    {
        return Err("kernel observer version, frequency, or record length mismatch".into());
    }
    let mut batches: Vec<Batch> = Vec::new();
    for record in bytes[32..].chunks_exact(72) {
        let u64_at = |p: usize| u64::from_le_bytes(record[p..p + 8].try_into().unwrap());
        let u32_at = |p: usize| u32::from_le_bytes(record[p..p + 4].try_into().unwrap());
        let u16_at = |p: usize| u16::from_le_bytes(record[p..p + 2].try_into().unwrap());
        let before = u64_at(16) as i64;
        let after = u64_at(24) as i64;
        if before <= 0
            || after < before
            || u64_at(0) == 0
            || u64_at(8) == 0
            || u64_at(32) == 0
            || u32_at(44) == 0
            || u32_at(40) >= u32_at(44)
        {
            return Err("invalid kernel callback bounds".into());
        }
        if batches.last().is_none_or(|b| b.sequence != u64_at(8)) {
            batches.push(Batch {
                device_change: None,
                boundary: "inside",
                source: "mouse_class",
                sequence: u64_at(8),
                before_qpc: before,
                after_qpc: after,
                packets: Vec::new(),
            });
        }
        let batch = batches.last_mut().unwrap();
        if batch.before_qpc != before || batch.after_qpc != after {
            return Err("kernel callback timestamps changed within a batch".into());
        }
        if batch.packets.last().is_some_and(|p| {
            let k = p.kernel.as_ref().unwrap();
            k.count != u32_at(44)
                || k.index.checked_add(1) != Some(u32_at(40))
                || p.device != u64_at(32)
        }) {
            return Err("kernel callback packet identity changed within a batch".into());
        }
        batch.packets.push(Packet {
            kernel: Some(KernelPacket {
                sequence: u64_at(0),
                batch: u64_at(8),
                index: u32_at(40),
                count: u32_at(44),
                unit: u16_at(54),
            }),
            device: u64_at(32),
            flags: u16_at(48),
            buttons: u16_at(50),
            button_data: u16_at(52),
            dx: u32_at(56) as i32,
            dy: u32_at(60) as i32,
            extra: u32_at(64),
        });
    }
    Ok((u64_at(16), batches))
}
#[cfg(not(windows))]
fn kernel_input(_: &SyncSender<Batch>, _: &AtomicBool, _: &AtomicU64, state: &Mutex<Control>) {
    state.locked().status.kernel_status = "the kernel observer needs Windows".into();
}

#[cfg(test)]
mod tests;
