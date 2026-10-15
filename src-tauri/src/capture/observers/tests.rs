use super::*;

/// the observers file the stop wrote beside `path`
fn summary(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path.with_extension("observers.json")).unwrap()).unwrap()
}
use std::time::{Duration, Instant};
fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "motion-bench-observer-{name}-{}-{}",
        std::process::id(),
        clock::qpc()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn batch(count: usize) -> Batch {
    let time = clock::qpc();
    Batch {
        device_change: None,
        boundary: "inside",
        source: "raw_input",
        sequence: 1,
        before_qpc: time,
        after_qpc: time,
        packets: (0..count)
            .map(|_| Packet {
                kernel: None,
                device: 0,
                flags: 0,
                buttons: 0,
                button_data: 0,
                dx: -1,
                dy: 2,
                extra: 0,
            })
            .collect(),
    }
}
/// the recorded mouse, which the test batches come from as device 0
const MOUSE: (u16, u16) = (0x373b, 0x11d9);
fn writer() -> (Worker, SyncSender<Batch>) {
    let control = Arc::new(Mutex::new(Control {
        status: Status {
            raw_ready: true,
            devices: BTreeMap::from([
                (
                    0,
                    r"\\?\HID#VID_373B&PID_11D9&MI_00#7&1a2b3c&0&0000#{378de44c}".into(),
                ),
                (
                    1,
                    r"\\?\HID#VID_046D&PID_C539&MI_01#8&4d5e6f&0&0000#{378de44c}".into(),
                ),
            ]),
            ..Default::default()
        },
        sink: None,
    }));
    let cached = Arc::new(Mutex::new(Status::default()));
    let losses = Arc::new(AtomicU64::new(0));
    let (tx, rx) = mpsc::sync_channel(64);
    let (c, l, k) = (control.clone(), losses.clone(), cached.clone());
    let thread = std::thread::spawn(move || write_batches(&c, &l, &k, rx));
    (
        Worker {
            control,
            cached,
            losses,
            stopping: Arc::new(AtomicBool::new(false)),
            threads: vec![thread],
        },
        tx,
    )
}
#[test]
fn bounded_writer_drains_eight_thousand_reports_without_frontend_snapshots() {
    let folder = temp("burst");
    let path = folder.join("capture.pcap");
    let (mut worker, tx) = writer();
    worker.journal().begin(&path, MOUSE).unwrap();
    for _ in 0..250 {
        tx.send(batch(32)).unwrap();
    }
    worker.control.locked().sink.as_mut().unwrap().ended_qpc = Some(clock::qpc());
    drop(tx);
    worker.stop();
    let summary = summary(&path);
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["raw_input"]["reports"], 8000);
    assert_eq!(summary["raw_input"]["batches"], 250);
    assert_eq!(summary["queue_losses"], 0);
    assert_eq!(summary["raw_input"]["os_loss_count_known"], false);
    assert_eq!(
        std::fs::read_to_string(path.with_extension("raw-input.jsonl"))
            .unwrap()
            .lines()
            .count(),
        250
    );
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn stalled_writer_counts_losses_and_snapshot_never_waits_for_disk_lock() {
    let folder = temp("loss");
    let path = folder.join("capture.pcap");
    let (mut worker, tx) = writer();
    worker.journal().begin(&path, MOUSE).unwrap();
    let hold = worker.control.locked();
    for _ in 0..100 {
        publish(&tx, batch(1), &worker.losses);
    }
    let began = Instant::now();
    let status = worker.snapshot();
    assert!(began.elapsed() < Duration::from_millis(100));
    assert!(status.queue_losses > 0 && status.error.is_some());
    assert!(worker.losses.load(Ordering::Relaxed) > 0);
    drop(hold);
    drop(tx);
    worker.stop();
    let summary = summary(&path);
    assert_eq!(summary["complete"], false);
    assert!(summary["queue_losses"].as_u64().unwrap() > 0);
    assert!(summary["error"].as_str().unwrap().contains("lost"));
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn failed_finalization_retains_explicit_incomplete_evidence() {
    let folder = temp("write-failure");
    let path = folder.join("capture.pcap");
    let (mut worker, tx) = writer();
    worker.journal().begin(&path, MOUSE).unwrap();
    let mut hold = worker.control.locked();
    hold.sink.as_mut().unwrap().raw =
        BufWriter::new(File::open(path.with_extension("raw-input.jsonl")).unwrap());
    hold.sink
        .as_mut()
        .unwrap()
        .raw
        .write_all(b"not writable")
        .unwrap();
    drop(hold);
    drop(tx);
    worker.stop();
    let summary = summary(&path);
    assert_eq!(summary["complete"], false);
    assert!(summary["error"].is_string());
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn boundary_batches_remain_in_evidence_without_entering_in_range_counts() {
    let folder = temp("boundaries");
    let path = folder.join("capture.pcap");
    let (mut worker, tx) = writer();
    worker.journal().begin(&path, MOUSE).unwrap();
    let mut c = worker.control.locked();
    let sink = c.sink.as_mut().unwrap();
    sink.started_qpc = 100;
    sink.ended_qpc = Some(200);
    for (before, after) in [(50, 80), (90, 110), (120, 130), (190, 210), (220, 240)] {
        let mut b = batch(1);
        b.before_qpc = before;
        b.after_qpc = after;
        tx.send(b).unwrap();
    }
    drop(c);
    drop(tx);
    worker.stop();
    let rows: Vec<serde_json::Value> =
        std::fs::read_to_string(path.with_extension("raw-input.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0]["boundary"], "before_start");
    assert_eq!(rows[1]["boundary"], "crosses_start");
    assert_eq!(rows[2]["boundary"], "inside");
    assert_eq!(rows[3]["boundary"], "crosses_stop");
    assert_eq!(rows[4]["boundary"], "after_stop");
    let summary = summary(&path);
    assert_eq!(summary["raw_input"]["reports"], 1);
    assert_eq!(summary["raw_input"]["boundary_batches"], 4);
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn device_reuse_is_a_recorded_discontinuity_instead_of_a_silent_remap() {
    let folder = temp("device-change");
    let path = folder.join("capture.pcap");
    let (mut worker, tx) = writer();
    worker.journal().begin(&path, MOUSE).unwrap();
    let mut b = batch(0);
    b.device_change = Some(DeviceChange {
        device: 0,
        action: "removal",
        previous_name: None,
        current_name: None,
    });
    tx.send(b).unwrap();
    drop(tx);
    worker.stop();
    let summary = summary(&path);
    assert_eq!(summary["complete"], false);
    assert_eq!(summary["raw_input"]["device_changes"], 1);
    assert!(summary["error"].as_str().unwrap().contains("topology"));
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn completion_metadata_failure_is_reported_to_capture_health() {
    let folder = temp("metadata-failure");
    let path = folder.join("capture.pcap");
    let (mut worker, tx) = writer();
    worker.journal().begin(&path, MOUSE).unwrap();
    std::fs::create_dir(path.with_extension("observers.json.tmp")).unwrap();
    drop(tx);
    worker.stop();
    assert!(
        worker
            .snapshot()
            .error
            .as_deref()
            .unwrap()
            .contains("finalization")
    );
    let summary = summary(&path);
    assert_eq!(summary["complete"], false);
    std::fs::remove_dir_all(folder).unwrap();
}
fn kernel_bytes() -> Vec<u8> {
    let mut bytes = vec![0u8; 32 + 2 * 72];
    bytes[..4].copy_from_slice(&1u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
    bytes[8..16].copy_from_slice(&1_000_000i64.to_le_bytes());
    for (i, r) in bytes[32..].chunks_exact_mut(72).enumerate() {
        for (p, n) in [(0, i as u64 + 100), (8, 7), (16, 1000), (24, 1010), (32, 1)] {
            r[p..p + 8].copy_from_slice(&n.to_le_bytes());
        }
        r[40..44].copy_from_slice(&(i as u32 + 3).to_le_bytes());
        r[44..48].copy_from_slice(&8u32.to_le_bytes());
        r[56..60].copy_from_slice(&(-15i32).to_le_bytes());
    }
    bytes
}
#[test]
fn kernel_parser_preserves_split_callback_indices_and_signed_counts() {
    let (_, b) = decode_kernel(&kernel_bytes(), 1_000_000).unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].packets.len(), 2);
    assert_eq!(b[0].packets[0].dx, -15);
    assert_eq!(b[0].packets[0].kernel.as_ref().unwrap().index, 3);
    assert_eq!(b[0].packets[0].kernel.as_ref().unwrap().count, 8);
}
#[test]
fn kernel_startup_history_does_not_hide_new_losses_or_crossing_callbacks() {
    let (_, mut old) = decode_kernel(&kernel_bytes(), 1_000_000).unwrap();
    let mut sequence = None;
    assert_eq!(kernel_continuity(&mut old, 1020, &mut sequence).unwrap(), 2);
    assert!(old.is_empty());
    assert!(sequence.is_none());
    let (_, mut fresh) = decode_kernel(&kernel_bytes(), 1_000_000).unwrap();
    fresh[0].before_qpc = 1020;
    fresh[0].after_qpc = 1030;
    for packet in &mut fresh[0].packets {
        packet.kernel.as_mut().unwrap().sequence += 500;
    }
    assert_eq!(
        kernel_continuity(&mut fresh, 1020, &mut sequence).unwrap(),
        0
    );
    assert_eq!(sequence, Some(601));
    assert!(kernel_continuity(&mut fresh, 1020, &mut sequence).is_err());
    let (_, mut crossing) = decode_kernel(&kernel_bytes(), 1_000_000).unwrap();
    assert_eq!(
        kernel_continuity(&mut crossing, 1005, &mut None).unwrap(),
        0
    );
    assert_eq!(crossing[0].packets.len(), 2);
}
#[test]
fn kernel_parser_rejects_truncation_wrong_clocks_and_invalid_batch_geometry() {
    let bytes = kernel_bytes();
    for end in 0..bytes.len() {
        assert!(decode_kernel(&bytes[..end], 1_000_000).is_err());
    }
    assert!(decode_kernel(&bytes, 2_000_000).is_err());
    assert!(decode_kernel(&bytes, 0).is_err());
    for (p, n) in [
        (0, 2u64),
        (32 + 24, 999),
        (32 + 40, 8),
        (32 + 72 + 16, 1001),
        (32 + 72 + 44, 9),
    ] {
        let mut invalid = bytes.clone();
        invalid[p..p + 8].copy_from_slice(&n.to_le_bytes());
        assert!(decode_kernel(&invalid, 1_000_000).is_err());
    }
}
#[cfg(windows)]
#[test]
#[ignore = "opens a read-only Windows Raw Input observer; run on an interactive desktop"]
fn raw_input_observer_starts_records_and_stops_on_windows() {
    let folder = temp("windows");
    let path = folder.join("capture.pcap");
    let mut worker = Worker::start();
    let began = Instant::now();
    while !worker.snapshot().raw_ready {
        assert!(worker.snapshot().error.is_none());
        assert!(began.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    worker.journal().begin(&path, MOUSE).unwrap();
    std::thread::sleep(Duration::from_secs(1));
    worker.stop();
    let summary = summary(&path);
    assert_eq!(summary["complete"], true);
    assert!(summary["clocks"]["samples"].as_u64().unwrap() >= 2);
    assert_eq!(summary["cross_layer_device_mapping_verified"], false);
    std::fs::remove_dir_all(folder).unwrap();
}

/// another mouse plugged in, coming and going or moving, stays out of the recording and doesn't
/// fail it
#[test]
fn only_the_recorded_mouse_goes_in_the_recording() {
    let folder = temp("other-mouse");
    let path = folder.join("capture.pcap");
    let (mut worker, tx) = writer();
    worker.journal().begin(&path, MOUSE).unwrap();
    let mut mixed = batch(2);
    mixed.packets[1].device = 1;
    tx.send(mixed).unwrap();
    let mut other = batch(3);
    for packet in &mut other.packets {
        packet.device = 1;
    }
    tx.send(other).unwrap();
    let mut gone = batch(0);
    gone.device_change = Some(DeviceChange {
        device: 1,
        action: "removal",
        previous_name: None,
        current_name: None,
    });
    tx.send(gone).unwrap();
    drop(tx);
    worker.stop();
    let rows: Vec<serde_json::Value> =
        std::fs::read_to_string(path.with_extension("raw-input.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["packets"].as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["packets"][0]["device"], 0);
    let summary = summary(&path);
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["raw_input"]["reports"], 1);
    let devices = summary["raw_input"]["devices"].as_object().unwrap();
    assert_eq!(devices.len(), 1);
    assert!(
        !devices
            .values()
            .any(|name| name.as_str().unwrap().contains("046D"))
    );
    std::fs::remove_dir_all(folder).unwrap();
}
