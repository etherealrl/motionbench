use crate::capture::pcap::{Endpoint, Event, Record};
use crate::evidence::commit::{self, Marker};
use crate::evidence::reason::{Failure, Reason};
use crate::sync::LockExt;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[repr(C)]
#[derive(Default)]
struct Systemtime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLocalTime(out: *mut Systemtime);
}

fn local_time() -> Systemtime {
    let mut now = Systemtime::default();
    unsafe { GetLocalTime(&mut now) };
    now
}

// the moment a recording starts, on the wall clock and the performance counter
pub struct Anchor {
    pub system_us: u64,
    pub qpc: i64,
    pub qpc_frequency: i64,
    pub local: String,
}

impl Anchor {
    fn take() -> Self {
        let now = local_time();
        let anchor = crate::capture::clock::Anchor::take();
        Anchor {
            system_us: anchor.utc_us,
            qpc: anchor.midpoint(),
            qpc_frequency: crate::capture::clock::frequency(),
            local: format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                now.year, now.month, now.day, now.hour, now.minute, now.second
            ),
        }
    }
}

pub struct Device {
    pub root: String,
    pub bus: u16,
    pub address: u16,
    pub endpoint: u8,
    pub vid: u16,
    pub pid: u16,
    pub payload_len: usize,
    pub advertised: Option<Endpoint>,
    /// the box as it was when the recording started, when one was connected
    pub box_state: Option<serde_json::Value>,
}

/// one descriptor record and the bus and address it came from
type Cached = ((u16, u16), Option<Vec<u8>>);

/// the pcap global header and the injected descriptors go past once, at capture start, long
/// before anyone clicks record. remembering them here is what lets a recording begin mid stream
/// and still be a valid file that knows which device it holds
#[derive(Default)]
pub struct Tee {
    global: Option<[u8; 24]>,
    /// an entry a later device at the same address replaced is emptied rather than removed, since
    /// a recorder keeps its place in this list by index
    descriptors: Vec<Cached>,
    descriptor_bytes: usize,
    /// the vid:pid each address last said it was
    ids: HashMap<(u16, u16), (u16, u16)>,
    pub error: Option<Failure>,
}

fn descriptor_setup(packet: &[u8]) -> bool {
    if packet.len() < 28 || packet[22] != 2 || packet[27] != 0 {
        return false;
    }
    let header = u16::from_le_bytes([packet[0], packet[1]]) as usize;
    packet.get(header + 1) == Some(&6)
        && packet
            .get(header + 3)
            .is_some_and(|kind| [1, 2, 0x22].contains(kind))
}
impl Tee {
    pub fn initialize(&self, recorder: &mut Recorder) {
        if let Some(global) = self.global {
            recorder.ensure_header(&global);
            recorder.catch_up(&self.descriptors);
        }
    }
    pub fn offer(&mut self, recorder: Option<&mut Recorder>, record: &Record<'_>) {
        match record {
            Record::Header(global) => self.global = Some(**global),
            Record::Packet {
                header,
                packet,
                bus,
                device,
                event,
            } if matches!(
                event,
                Some(Event::Device { .. } | Event::Config { .. } | Event::Hid { .. })
            ) || descriptor_setup(packet) =>
            {
                let owner = (*bus, *device);
                // another device at this address, like a new mouse or a firmware's bootloader, so
                // what the one before it sent isn't this device's and would read as an address reuse
                if let Some(Event::Device { vid, pid, .. }) = event
                    && self
                        .ids
                        .insert(owner, (*vid, *pid))
                        .is_some_and(|before| before != (*vid, *pid))
                {
                    for (_, raw) in self.descriptors.iter_mut().filter(|(o, _)| *o == owner) {
                        self.descriptor_bytes -= raw.take().map_or(0, |r| r.len());
                    }
                }
                let mut raw = header.to_vec();
                raw.extend_from_slice(packet);
                if self.descriptors.len() >= 4096
                    || self.descriptor_bytes + raw.len() > 16 * 1024 * 1024
                {
                    self.error.get_or_insert(Failure::new(
                        Reason::DescriptorCacheFull,
                        "descriptor cache limit exceeded, restart capture",
                    ));
                } else {
                    self.descriptor_bytes += raw.len();
                    self.descriptors.push((owner, Some(raw)));
                }
            }
            _ => {}
        }
        let Some(recorder) = recorder else {
            return;
        };
        if let Some(global) = self.global {
            recorder.ensure_header(&global);
            recorder.catch_up(&self.descriptors);
        }
        recorder.offer(record);
    }
}

// tests swap in a file that fails on cue, because nothing makes a real file fail halfway
// through a recording on windows
#[cfg(not(test))]
pub(crate) type Sink = File;
#[cfg(test)]
pub(crate) type Sink = crate::testkit::faults::FaultyFile;

/// what the recorder wrote since its files were last written to. it waits in memory so the
/// capture's lock is never held over a file
#[derive(Default)]
struct Pending {
    pcap: Vec<u8>,
    csv: Vec<u8>,
    motion: Vec<u8>,
    /// written as json once the lock is let go
    clocks: Vec<crate::capture::clock::Anchor>,
    arrivals: Vec<u8>,
    input: Vec<u8>,
}
impl Pending {
    fn is_empty(&self) -> bool {
        self.clocks.is_empty()
            && [
                &self.pcap,
                &self.csv,
                &self.motion,
                &self.arrivals,
                &self.input,
            ]
            .iter()
            .all(|bytes| bytes.is_empty())
    }
}

/// a recording's files. the box's two appear with the first row the box sends
struct Files {
    folder: PathBuf,
    stem: String,
    pcap: BufWriter<Sink>,
    csv: BufWriter<Sink>,
    motion: BufWriter<Sink>,
    clocks: BufWriter<Sink>,
    arrivals: Option<BufWriter<Sink>>,
    input: Option<BufWriter<Sink>>,
}
impl Files {
    /// every file gets its bytes even after one fails, the first failure is the answer
    fn write(&mut self, pending: &Pending) -> io::Result<()> {
        let path = |kind: &str| self.folder.join(format!("{}.{kind}", self.stem));
        let (arrivals, input) = (path("input-arrivals.csv"), path("input.csv"));
        [
            write_all(&mut self.pcap, &pending.pcap),
            write_all(&mut self.csv, &pending.csv),
            write_all(&mut self.motion, &pending.motion),
            pending.clocks.iter().try_for_each(|anchor| {
                serde_json::to_writer(&mut self.clocks, anchor)?;
                self.clocks.write_all(b"\n")
            }),
            created(&mut self.arrivals, arrivals, &pending.arrivals),
            created(&mut self.input, input, &pending.input),
        ]
        .into_iter()
        .collect()
    }

    fn flush_and_sync(&mut self, paired_shift_us: Option<i64>) -> io::Result<()> {
        let mut first = Ok(());
        for file in [
            Some(&mut self.pcap),
            Some(&mut self.csv),
            Some(&mut self.motion),
            Some(&mut self.clocks),
            self.arrivals.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            let synced = file.flush().and_then(|()| file.get_ref().sync_all());
            if first.is_ok() {
                first = synced;
            }
        }
        // the shift goes in last and only when everything else is on disk, so a failed
        // recording never leaves an input file that says the two clocks were related
        if let Some(input) = self.input.as_mut() {
            let shifted = match paired_shift_us {
                Some(shift) if first.is_ok() => writeln!(input, "# paired_shift_us={shift}"),
                _ => Ok(()),
            };
            let synced = shifted
                .and_then(|()| input.flush())
                .and_then(|()| input.get_ref().sync_all());
            if first.is_ok() {
                first = synced;
            }
        }
        first
    }
}

fn write_all(file: &mut BufWriter<Sink>, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    file.write_all(bytes)
}

fn created(file: &mut Option<BufWriter<Sink>>, path: PathBuf, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    if file.is_none() {
        *file = Some(BufWriter::new(Sink::create(path)?));
    }
    file.as_mut().expect("created just above").write_all(bytes)
}

/// the bytes taken under the capture's lock, in the order it gave them, until whoever holds the
/// files next writes them. its own lock is never held over a file, so the capture's lock never
/// waits on a disk
type Queue = Arc<Mutex<VecDeque<Pending>>>;

/// everything queued, written in order. the queue is let go before each write
fn drain(queue: &Queue, files: &mut Files) -> io::Result<()> {
    let mut first = Ok(());
    loop {
        let next = queue.locked().pop_front();
        let Some(pending) = next else {
            return first;
        };
        let written = files.write(&pending);
        if first.is_ok() {
            first = written;
        }
    }
}

/// a recorder's files and the queue its bytes wait in, written once the capture's lock is let go
#[must_use = "dropped writes are bytes the recording never gets"]
pub struct Writes {
    queue: Queue,
    files: Arc<Mutex<Files>>,
    failed: Arc<OnceLock<Failure>>,
}
impl Writes {
    /// `held`, the capture's lock, goes first, then the files are waited for. the bytes are in
    /// the queue already, in the order the lock gave them, and whoever holds the files writes all
    /// of it, a close too, so bytes taken later are written later and a close misses none
    pub fn after<T>(self, held: T) {
        drop(held);
        let mut files = self.files.locked();
        if let Err(error) = drain(&self.queue, &mut files) {
            let _ = self.failed.set(Failure::new(Reason::WriterFailed, error));
        }
    }
}

pub struct Recorder {
    started: std::time::Instant,
    clock_monitor: crate::capture::clock::Monitor,
    last_clock_qpc: i64,
    pub decoder: crate::capture::hid::Decoder,
    headed: bool,
    caught_up: usize,
    first_us: Option<u64>,
    arrivals_begun: bool,
    input_begun: bool,
    pending: Pending,
    queue: Queue,
    files: Arc<Mutex<Files>>,
    /// the first write to the files that failed, set outside the capture's lock
    failed: Arc<OnceLock<Failure>>,
    pub input_rows: u64,
    device: Device,
    anchor: Anchor,
    folder: PathBuf,
    stem: String,
    last_us: Option<u64>,
    pub records: u64,
    pub reports: u64,
    pub bytes: u64,
    pub trouble: Option<Failure>,
    marker: Marker,
}

/// a recorder whose files are flushed and synced, holding the metadata it can write itself. the
/// rest comes from whoever closed it, then `publish` writes it once
pub struct Closed {
    pub manifest: serde_json::Value,
    /// the first fault of the recording, or of closing its files
    pub failure: Option<Failure>,
    marker: Marker,
    folder: PathBuf,
    stem: String,
}

impl Recorder {
    pub fn begin(folder: &Path, device: Device) -> io::Result<Self> {
        let anchor = Anchor::take();
        let stem = format!(
            "{:04x}{:04x}-{}",
            device.vid,
            device.pid,
            anchor.local.replace(['-', ':'], "").replace(' ', "-")
        );
        let stem = format!("{stem}-{}", anchor.system_us);
        fs::create_dir_all(folder)?;
        let marker = Marker::create(
            folder.join(format!("{stem}.active.json")),
            &stem,
            anchor.system_us,
        )?;
        let startup = STARTUP.map(|kind| folder.join(format!("{stem}.{kind}")));
        let files = (|| -> io::Result<_> {
            let mut csv = BufWriter::new(Sink::create(&startup[0])?);
            writeln!(csv, "t_us,dt_us,payload")?;
            // the exact file abcurves' wire_to_csv writes, so its judges read a recording unchanged
            let mut motion = BufWriter::new(Sink::create(&startup[1])?);
            writeln!(motion, "# qpc_frequency=1000000")?;
            writeln!(
                motion,
                "# from {stem}.pcap, USB address {}, endpoint {:#x}",
                device.address, device.endpoint
            )?;
            writeln!(motion, "qpc,dx,dy")?;
            let clocks = BufWriter::new(Sink::create(&startup[2])?);
            let pcap = BufWriter::new(Sink::create(&startup[3])?);
            Ok((csv, motion, clocks, pcap))
        })();
        let (csv, motion, clocks, pcap) = match files {
            Ok(files) => files,
            Err(error) => {
                commit::discard(&startup);
                let _ = marker.remove();
                return Err(error);
            }
        };

        let files = Files {
            folder: folder.to_path_buf(),
            stem: stem.clone(),
            pcap,
            csv,
            motion,
            clocks,
            arrivals: None,
            input: None,
        };
        let mut recorder = Recorder {
            started: std::time::Instant::now(),
            clock_monitor: crate::capture::clock::Monitor::default(),
            last_clock_qpc: 0,
            decoder: crate::capture::hid::Decoder::Unavailable,
            headed: false,
            caught_up: 0,
            first_us: None,
            arrivals_begun: false,
            input_begun: false,
            pending: Pending::default(),
            queue: Queue::default(),
            files: Arc::new(Mutex::new(files)),
            failed: Arc::new(OnceLock::new()),
            input_rows: 0,
            device,
            anchor,
            folder: folder.to_path_buf(),
            stem,
            last_us: None,
            records: 0,
            reports: 0,
            bytes: 0,
            trouble: None,
            marker,
        };
        recorder.observe_clock(true);
        Ok(recorder)
    }

    /// a recorder that was made but never took evidence, because the live state refused it.
    /// its startup files and marker go, they hold nothing
    pub fn abandon(self) {
        let startup = STARTUP.map(|kind| self.path(kind));
        let marker = self.marker;
        drop(self.files);
        commit::discard(&startup);
        let _ = marker.remove();
    }

    pub fn path(&self, kind: &str) -> PathBuf {
        self.folder.join(format!("{}.{kind}", self.stem))
    }

    pub fn started_us(&self) -> u64 {
        self.anchor.system_us
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    /// bus, address and endpoint of the device it records
    pub fn device_key(&self) -> (u16, u16, u8) {
        (self.device.bus, self.device.address, self.device.endpoint)
    }
    /// vid and pid of the device it records
    pub fn vid_pid(&self) -> (u16, u16) {
        (self.device.vid, self.device.pid)
    }
    pub fn stem(&self) -> &str {
        &self.stem
    }
    /// the first fault of the recording, its own or a write to its files
    pub fn failure(&self) -> Option<&Failure> {
        self.trouble.as_ref().or(self.failed.get())
    }
    /// what is waiting goes into the queue, and the writes that take it out once the capture's
    /// lock is let go, see `Writes::after`
    pub fn writes(&mut self) -> Option<Writes> {
        if self.pending.is_empty() {
            return None;
        }
        self.queue
            .locked()
            .push_back(std::mem::take(&mut self.pending));
        Some(Writes {
            queue: self.queue.clone(),
            files: self.files.clone(),
            failed: self.failed.clone(),
        })
    }
    pub fn observe_clock(&mut self, force: bool) {
        let now = crate::capture::clock::qpc();
        if !force && now - self.last_clock_qpc < crate::capture::clock::frequency() {
            return;
        }
        self.last_clock_qpc = now;
        let anchor = crate::capture::clock::Anchor::take();
        self.pending.clocks.push(anchor.clone());
        // the anchor is kept even when it is wrong, the code says the clock failed, not the file
        if let Err(error) = self
            .clock_monitor
            .observe(&anchor, crate::capture::clock::frequency())
        {
            self.trouble
                .get_or_insert(Failure::new(Reason::ClockFailed, error));
        }
    }
    pub fn receipt(&mut self, motion: &crate::hardware::medius::Motion) {
        let result = (|| -> io::Result<()> {
            if !self.arrivals_begun {
                writeln!(self.pending.arrivals, "box_us,received_qpc,received_utc_us")?;
                self.arrivals_begun = true;
            }
            writeln!(
                self.pending.arrivals,
                "{},{},{}",
                motion.box_us, motion.arrived_qpc, motion.arrived_us
            )
        })();
        if let Err(error) = result {
            self.trouble
                .get_or_insert(Failure::new(Reason::WriterFailed, error));
        }
    }

    fn ensure_header(&mut self, global: &[u8; 24]) {
        if self.headed {
            return;
        }
        self.headed = true;
        if let Err(e) = self.pending.pcap.write_all(global) {
            self.trouble
                .get_or_insert(Failure::new(Reason::WriterFailed, e));
        }
    }

    /// descriptor records the tee has seen and this recorder has not yet written, for its own
    /// device only. covers both orders: installed late, or installed before they arrived
    fn catch_up(&mut self, descriptors: &[Cached]) {
        for (owner, raw) in &descriptors[self.caught_up.min(descriptors.len())..] {
            let Some(raw) = raw else {
                continue;
            };
            if *owner != (self.device.bus, self.device.address) {
                continue;
            }
            if let Err(e) = self.pending.pcap.write_all(raw) {
                self.trouble
                    .get_or_insert(Failure::new(Reason::WriterFailed, e));
            }
            self.records += 1;
            self.bytes += raw.len() as u64;
        }
        self.caught_up = descriptors.len();
    }

    fn offer(&mut self, record: &Record<'_>) {
        if let Err(e) = self.write(record) {
            self.trouble
                .get_or_insert(Failure::new(Reason::WriterFailed, e));
        }
    }

    fn write(&mut self, record: &Record<'_>) -> io::Result<()> {
        match record {
            // written once by ensure_header, which knows it even when we joined mid stream
            Record::Header(_) => {}
            // descriptors arrive through catch_up, so a recording that started late gets them too
            Record::Packet {
                event: Some(Event::Device { .. } | Event::Config { .. } | Event::Hid { .. }),
                ..
            } => {}
            Record::Packet {
                header,
                packet,
                bus,
                device,
                event,
            } => {
                if descriptor_setup(packet) {
                    return Ok(());
                }
                if (*bus, *device) != (self.device.bus, self.device.address) {
                    return Ok(());
                }
                if packet.get(22) != Some(&2) && packet.get(21) != Some(&self.device.endpoint) {
                    return Ok(());
                }
                self.pending.pcap.write_all(*header)?;
                self.pending.pcap.write_all(packet)?;
                self.records += 1;
                self.bytes += (header.len() + packet.len()) as u64;

                if let Some(Event::Report(report)) = event {
                    if report.endpoint != self.device.endpoint {
                        return Ok(());
                    }
                    // a report stamped before the latest one keeps its raw bytes in the pcap and its
                    // row in the csv, but gets no interval and no motion row, derived time only
                    // moves forward. the session fails the recording for it
                    let backward = self.last_us.is_some_and(|last| report.t_us < last);
                    let dt = self.last_us.and_then(|last| report.t_us.checked_sub(last));
                    self.last_us = self.last_us.max(Some(report.t_us));
                    self.reports += 1;

                    let csv = &mut self.pending.csv;
                    write!(csv, "{},", report.t_us)?;
                    if let Some(dt) = dt {
                        write!(csv, "{dt}")?;
                    }
                    write!(csv, ",")?;
                    for byte in report.bytes() {
                        write!(csv, "{byte:02x}")?;
                    }
                    writeln!(csv)?;

                    if let Some((dx, dy)) = self.decoder.decode(report.bytes())
                        && !backward
                    {
                        // `first` came from a report that was not behind, and this one is not
                        // behind the latest either, so the subtraction cannot go below zero
                        let first = *self.first_us.get_or_insert(report.t_us);
                        writeln!(self.pending.motion, "{},{dx},{dy}", report.t_us - first)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// the box's view of the same movement, on its own clock. the file appears with the first
    /// report the box sends, so a recording made without the watch on leaves no such file
    pub fn input(&mut self, box_us: u64, dx: i16, dy: i16, dz: i16) {
        let result = (|| -> io::Result<()> {
            let file = &mut self.pending.input;
            if !self.input_begun {
                writeln!(
                    file,
                    "# every report the box's host chip took off usb3 while this pcap was recorded"
                )?;
                writeln!(
                    file,
                    "# box_us is that chip's clock. box_us = wire t_us + paired_shift_us, written at the end"
                )?;
                writeln!(file, "box_us,dx,dy,dz")?;
                self.input_begun = true;
            }
            writeln!(file, "{box_us},{dx},{dy},{dz}")?;
            self.input_rows += 1;
            Ok(())
        })();
        if let Err(e) = result {
            self.trouble
                .get_or_insert(Failure::new(Reason::WriterFailed, e));
        }
    }

    /// ends the recording's files: the last clock check, then every file flushed and synced.
    /// the shift puts the box's clock on the wire's, it is written only when nothing failed,
    /// and after a fault the files are still flushed as far as they go
    pub fn close(mut self, paired_shift_us: Option<i64>) -> Closed {
        self.observe_clock(true);
        let files = self.files.clone();
        let mut files = files.locked();
        // what other writers queued is older than what this recorder still holds
        let queued = drain(&self.queue, &mut files);
        let written = files.write(&std::mem::take(&mut self.pending));
        let written = queued
            .and(written)
            .map_err(|error| Failure::new(Reason::WriterFailed, error));
        let failure = match self.failure().cloned().or(written.err()) {
            Some(trouble) => {
                let _ = files.flush_and_sync(None);
                Some(trouble)
            }
            None => files
                .flush_and_sync(paired_shift_us)
                .err()
                .map(|error| Failure::new(Reason::WriterFailed, error)),
        };
        drop(files);
        let shift = paired_shift_us.filter(|_| failure.is_none());
        Closed {
            manifest: self.manifest(shift),
            failure,
            marker: self.marker,
            folder: self.folder,
            stem: self.stem,
        }
    }

    /// what makes the other files readable a month later, by anything
    fn manifest(&self, paired_shift_us: Option<i64>) -> serde_json::Value {
        let device = &self.device;
        let mut selection = serde_json::json!({
            "note": "records for this device only, written verbatim. other devices on the root hub were dropped and never reached disk.",
            "bus": device.bus,
            "address": device.address,
            "endpoint": device.endpoint,
            // zero is what an unnamed device gets, written as unknown rather than as an id
            "vid": (device.vid != 0).then(|| format!("{:#06x}", device.vid)),
            "pid": (device.pid != 0).then(|| format!("{:#06x}", device.pid)),
            "payload_bytes": device.payload_len,
            "b_interval": null,
        });
        if let Some(e) = device.advertised {
            let (full, high) = e.period_us();
            selection["kind"] = e.kind().into();
            selection["interface_class"] = serde_json::json!([e.class, e.subclass, e.protocol]);
            selection["b_interval"] = e.interval.into();
            selection["advertised_period_us"] =
                serde_json::json!({ "full_speed": full, "high_speed": high });
            selection["max_packet"] = e.max_packet.into();
        }
        serde_json::json!({
            "tool": format!("motion-bench {}", env!("CARGO_PKG_VERSION")),
            "root": device.root,
            "selection": selection,
            "timing": {
                "schema": "motion-bench.timing.v1",
                "pcap_clock": "usbpcap_driver_utc_microseconds",
                "host_clock": "qpc",
                "host_qpc_frequency": crate::capture::clock::frequency(),
                "anchors": format!("{}.clocks.jsonl", self.stem),
                "anchor_summary": self.clock_monitor.summary,
                "mapping_discontinuity_threshold_us": 1000,
                "medius_receipt_observation": "anybox Stamp.received_qpc, taken as anybox dequeues the event from the medius SDK; utc mapped from it",
                "motion_csv_clock": "USBPcap relative microseconds, in the column ABCurves' wire_to_csv calls qpc",
                "hardware_bus_timestamps": false,
                "raw_input_timestamps": "receipt batches, not hardware timestamps",
            },
            "clock": {
                "note": "pcap stamps are driver UTC microseconds. native QPC brackets precise UTC for joining host observations; see timing.anchor_summary and the clock journal for uncertainty.",
                "started_local": self.anchor.local,
                "system_us": self.anchor.system_us,
                "qpc": self.anchor.qpc,
                "qpc_frequency": self.anchor.qpc_frequency,
            },
            "box": device.box_state,
            "paired": {
                "note": "the input csv holds what the box saw on usb3 during this recording, on the host chip's clock. box_us = wire t_us + shift_us. null when the watch was off or a clock was never seen.",
                "input_rows": self.input_rows,
                "shift_us": paired_shift_us,
            },
            "counts": {
                "pcap_records": self.records,
                "reports": self.reports,
                "pcap_bytes": self.bytes,
            },
        })
    }
}

/// the files a recording opens before it takes any evidence
const STARTUP: [&str; 4] = ["csv", "motion.csv", "clocks.jsonl", "pcap"];

impl Closed {
    pub fn pcap(&self) -> PathBuf {
        self.folder.join(format!("{}.pcap", self.stem))
    }

    /// publishes the finished metadata, then takes the marker off. when publishing fails the
    /// marker stays to say the recording never finished, and the first fault is the answer
    pub fn publish(self, manifest: &serde_json::Value) -> Result<PathBuf, Failure> {
        let pcap = self.pcap();
        if let Err(error) = commit::publish(&pcap.with_extension("json"), manifest) {
            let failure = self.failure.unwrap_or_else(|| {
                Failure::new(
                    Reason::MetadataFailed,
                    format!("recording metadata finalization failed: {error}"),
                )
            });
            // the running marker alone cannot say how it failed, so the failure gets a marker
            // of its own, never beside a sidecar already there
            if error.kind() != io::ErrorKind::AlreadyExists {
                let _ = fs::write(
                    pcap.with_extension("incomplete.json"),
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "reason": failure.reason, "error": failure.message,
                    }))
                    .unwrap(),
                );
            }
            return Err(failure);
        }
        // ponytail: a marker that will not come off is left, published metadata outranks it
        let _ = self.marker.remove();
        self.failure.map_or(Ok(pcap), Err)
    }
}

/// the input csv beside a recording, back into a stream on the box's clock, with the shift that
/// puts it on the wire's. a file without the shift line cannot be paired and is refused
pub fn read_input(path: &Path) -> io::Result<(crate::signal::stats::Stream, i64)> {
    let mut stream = crate::signal::stats::Stream::new(u64::MAX);
    let (shift, _, _) = visit_input(path, |t, dx, dy, dz| {
        stream.push_motion(t, dx, dy, dz);
        Ok(())
    })?;
    Ok((stream, shift))
}
pub fn visit_input(
    path: &Path,
    mut visit: impl FnMut(u64, i16, i16, i16) -> io::Result<()>,
) -> io::Result<(i64, Option<u64>, Option<u64>)> {
    use std::io::BufRead;
    let file = std::io::BufReader::new(File::open(path)?);
    let mut first = None;
    let mut last = None;
    let mut shift = None;
    for line in file.lines() {
        let line = line?;
        if let Some(value) = line.strip_prefix("# paired_shift_us=") {
            shift = value.trim().parse::<i64>().ok();
            continue;
        }
        if line.starts_with('#') || line.starts_with("box_us") {
            continue;
        }
        let mut cells = line.split(',');
        let (Some(t), Some(dx), Some(dy), Some(dz)) =
            (cells.next(), cells.next(), cells.next(), cells.next())
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated input row",
            ));
        };
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "malformed input row");
        let t = t.parse::<u64>().map_err(|_| invalid())?;
        let dx = dx.parse().map_err(|_| invalid())?;
        let dy = dy.parse().map_err(|_| invalid())?;
        let dz = dz.parse().map_err(|_| invalid())?;
        if last.is_some_and(|last| t < last) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "input clock regressed",
            ));
        }
        first.get_or_insert(t);
        last = Some(t);
        visit(t, dx, dy, dz)?;
    }
    let shift = shift.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "no paired_shift_us line, the two clocks were never related",
        )
    })?;
    Ok((shift, first, last))
}

#[cfg(test)]
mod tests;
