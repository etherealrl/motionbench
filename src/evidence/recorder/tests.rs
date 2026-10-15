#[test]
fn input_without_its_shift_line_cannot_be_paired() {
    let dir = std::env::temp_dir().join(format!("mpm-input-shift-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("input.csv");
    std::fs::write(
        &path,
        "# paired_shift_us=-5
box_us,dx,dy,dz
4123456789,3,-2,0
",
    )
    .unwrap();
    let (stream, shift) = super::read_input(&path).unwrap();
    assert_eq!(shift, -5);
    assert_eq!(stream.total, 1);

    std::fs::write(
        &path,
        "box_us,dx,dy,dz
1,0,0,0
",
    )
    .unwrap();
    assert!(super::read_input(&path).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

use super::*;

/// closes the recorder and publishes what it can write itself
fn finish(recorder: Recorder, paired_shift_us: Option<i64>) -> PathBuf {
    let closed = recorder.close(paired_shift_us);
    let manifest = closed.manifest.clone();
    closed.publish(&manifest).unwrap()
}
use crate::capture::pcap::{Report, read_pcap};

fn device(address: u16) -> Device {
    Device {
        root: "\\\\.\\USBPcap1".to_string(),
        bus: 1,
        address,
        endpoint: 0x81,
        vid: 0x373b,
        pid: 0x11d9,
        payload_len: 7,
        advertised: Some(Endpoint {
            interface: 0,
            address: 0x81,
            interval: 1,
            max_packet: 7,
            class: 3,
            subclass: 1,
            protocol: 2,
        }),
        box_state: None,
    }
}

/// a recording has to parse back through our own reader, or it is not evidence of anything
#[test]
fn a_recording_round_trips_and_drops_the_other_devices() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    );
    let raw = std::fs::read(fixture).unwrap();
    let folder = std::env::temp_dir().join("motion-bench-test");
    let _ = std::fs::remove_dir_all(&folder);

    let mut recorder = Recorder::begin(&folder, device(31)).unwrap();
    recorder.decoder = crate::testkit::traffic::mouse_decoder();
    let mut tee = Tee::default();
    read_pcap(&raw[..], |record| tee.offer(Some(&mut recorder), &record)).unwrap();
    let (records, reports) = (recorder.records, recorder.reports);
    let stem = recorder.stem().to_string();
    finish(recorder, None);

    assert_eq!(records, 400);
    assert_eq!(reports, 400);

    let written = std::fs::read(folder.join(format!("{stem}.pcap"))).unwrap();
    let mut back: Vec<Report> = Vec::new();
    read_pcap(&written[..], |record| {
        if let Record::Packet {
            event: Some(Event::Report(r)),
            ..
        } = record
        {
            back.push(r)
        }
    })
    .unwrap();

    assert_eq!(back.len(), 400);
    assert_eq!(back[0].device, 31);
    assert_eq!(back[0].len, 7);

    let csv = std::fs::read_to_string(folder.join(format!("{stem}.csv"))).unwrap();
    assert_eq!(csv.lines().count(), 401);
    assert!(csv.starts_with("t_us,dt_us,payload\n"));

    let motion = std::fs::read_to_string(folder.join(format!("{stem}.motion.csv"))).unwrap();
    let mut lines = motion.lines();
    assert_eq!(lines.next(), Some("# qpc_frequency=1000000"));
    assert!(lines.next().unwrap().starts_with("# from "));
    assert_eq!(lines.next(), Some("qpc,dx,dy"));
    assert!(lines.next().unwrap().starts_with("0,"));
    assert_eq!(motion.lines().count(), 403);

    let json = std::fs::read_to_string(folder.join(format!("{stem}.json"))).unwrap();
    assert!(json.contains("\"qpc_frequency\""));
    assert!(json.contains("\"reports\": 400"));
    assert!(json.contains("\"b_interval\": 1,"));
    assert!(json.contains("\"kind\": \"mouse\""));
    assert!(json.contains("\"box\": null,"));
    assert!(json.contains("\"shift_us\": null"));
    assert!(!folder.join(format!("{stem}.input.csv")).exists());
    assert!(json.contains("\"root\": \"\\\\\\\\.\\\\USBPcap1\""));

    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_different_address_writes_nothing_but_the_header() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    );
    let raw = std::fs::read(fixture).unwrap();
    let folder = std::env::temp_dir().join("motion-bench-test-empty");
    let _ = std::fs::remove_dir_all(&folder);

    let mut recorder = Recorder::begin(&folder, device(9)).unwrap();
    let mut tee = Tee::default();
    read_pcap(&raw[..], |record| tee.offer(Some(&mut recorder), &record)).unwrap();
    let stem = recorder.stem().to_string();
    assert_eq!(recorder.records, 0);
    assert!(recorder.trouble.is_none());
    finish(recorder, None);

    let written = std::fs::read(folder.join(format!("{stem}.pcap"))).unwrap();
    assert_eq!(written.len(), 24);

    let _ = std::fs::remove_dir_all(&folder);
}

/// recording starts when the button is clicked, which is long after the global header went
/// past. without it the file is records with no pcap header and nothing will open it
#[test]
fn a_recording_started_mid_stream_still_has_a_pcap_header() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    );
    let raw = std::fs::read(fixture).unwrap();
    let folder = std::env::temp_dir().join("motion-bench-test-late");
    let _ = std::fs::remove_dir_all(&folder);

    let mut tee = Tee::default();
    let mut recorder = None;
    let mut seen = 0;
    read_pcap(&raw[..], |record| {
        if seen == 100 {
            recorder = Some(Recorder::begin(&folder, device(31)).unwrap());
        }
        seen += 1;
        tee.offer(recorder.as_mut(), &record);
    })
    .unwrap();

    let recorder = recorder.unwrap();
    let stem = recorder.stem().to_string();
    assert_eq!(recorder.records, 301);
    finish(recorder, None);

    let written = std::fs::read(folder.join(format!("{stem}.pcap"))).unwrap();
    let mut back = 0;
    read_pcap(&written[..], |record| {
        if matches!(record, Record::Packet { .. }) {
            back += 1;
        }
    })
    .unwrap();

    assert_eq!(back, 301);
    let _ = std::fs::remove_dir_all(&folder);
}

/// the descriptors go past at capture start, so a recording that begins later has to be handed
/// its own device's, or a replay of it has no idea what it is
#[test]
fn a_late_recording_still_knows_its_own_device() {
    let root = env!("CARGO_MANIFEST_DIR");
    let descriptors = std::fs::read(format!("{root}/tests/fixtures/descriptors.pcap")).unwrap();
    let reports = std::fs::read(format!("{root}/tests/fixtures/mouse-1000hz.pcap")).unwrap();
    let mut stream = descriptors.clone();
    stream.extend_from_slice(&reports[24..]);
    let folder = std::env::temp_dir().join("motion-bench-test-descriptors");
    let _ = std::fs::remove_dir_all(&folder);

    let mut tee = Tee::default();
    let mut recorder = None;
    read_pcap(&stream[..], |record| {
        // every descriptor has gone past by the time the first report shows up
        let first_report = recorder.is_none()
            && matches!(
                record,
                Record::Packet {
                    event: Some(Event::Report(_)),
                    ..
                }
            );
        if first_report {
            recorder = Some(Recorder::begin(&folder, device(31)).unwrap());
        }
        tee.offer(recorder.as_mut(), &record);
    })
    .unwrap();
    let recorder = recorder.unwrap();
    let stem = recorder.stem().to_string();
    finish(recorder, None);

    let written = std::fs::read(folder.join(format!("{stem}.pcap"))).unwrap();
    let mut got = Vec::new();
    read_pcap(&written[..], |record| {
        if let Record::Packet {
            event: Some(event), ..
        } = record
        {
            got.push(event)
        }
    })
    .unwrap();

    assert!(got.contains(&Event::Device {
        bus: 1,
        address: 31,
        vid: 0x373b,
        pid: 0x11d9
    }));
    assert!(
        got.iter()
            .any(|e| matches!(e, Event::Config { address: 31, .. }))
    );
    assert!(!got.iter().any(|e| matches!(
        e,
        Event::Device { address: 32, .. } | Event::Config { address: 32, .. }
    )));
    assert_eq!(
        got.iter().filter(|e| matches!(e, Event::Report(_))).count(),
        400
    );
    let _ = std::fs::remove_dir_all(&folder);
}

/// what the box saw goes beside the pcap and comes back as a stream with its shift, so a
/// replay pairs without the box present
#[test]
fn the_input_side_round_trips_with_its_shift() {
    let folder = std::env::temp_dir().join("motion-bench-test-input");
    let _ = std::fs::remove_dir_all(&folder);

    let mut recorder = Recorder::begin(&folder, device(31)).unwrap();
    recorder.input(1_000, 3, -2, 0);
    recorder.input(2_000, 0, 0, 1);
    let stem = recorder.stem().to_string();
    finish(recorder, Some(-12_345));

    let (stream, shift) = read_input(&folder.join(format!("{stem}.input.csv"))).unwrap();
    let view = stream.view(0, u64::MAX);

    assert_eq!(shift, -12_345);
    assert_eq!(view.reports(), 2);
    assert_eq!(
        view.motion().collect::<Vec<_>>(),
        vec![(1_000, 3, -2), (2_000, 0, 0)]
    );
    assert_eq!(view.still(), 0);
    let json = std::fs::read_to_string(folder.join(format!("{stem}.json"))).unwrap();
    assert!(json.contains("\"input_rows\": 2,"));
    assert!(json.contains("\"shift_us\": -12345"));

    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn the_anchor_reads_a_running_counter() {
    let first = Anchor::take();
    let second = Anchor::take();

    assert!(first.qpc_frequency > 0);
    assert_eq!(first.qpc_frequency, second.qpc_frequency);
    assert!(second.qpc >= first.qpc);
    assert!(first.system_us > 1_700_000_000_000_000);
}
#[test]
fn malformed_and_regressing_input_are_not_silently_skipped() {
    let folder = std::env::temp_dir().join(format!("mpm-input-errors-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("input.csv");
    for text in [
        "# paired_shift_us=0\n100,1,2,0\n90,1,2,0\n",
        "# paired_shift_us=0\nbad,1,2,0\n",
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(super::read_input(&path).is_err());
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn a_write_that_fails_while_closing_fails_the_recording() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    );
    let raw = std::fs::read(fixture).unwrap();
    let folder = crate::testkit::traffic::TempDir::new("close-write");
    let mut recorder = {
        // the flush after it goes through, so only the write itself can say
        let _fault =
            crate::testkit::faults::inject("csv", crate::testkit::faults::Fault::FailOnceAt(100));
        Recorder::begin(folder.path(), device(31)).unwrap()
    };
    recorder.decoder = crate::testkit::traffic::mouse_decoder();
    let mut tee = Tee::default();
    // nothing is written before the close, it all waits in memory
    read_pcap(&raw[..], |record| tee.offer(Some(&mut recorder), &record)).unwrap();
    let closed = recorder.close(None);
    let failure = closed.failure.clone().unwrap();
    assert_eq!(
        failure.reason,
        crate::evidence::reason::Reason::WriterFailed
    );
}

#[test]
fn a_clock_fault_first_seen_at_finish_is_saved_as_a_clock_fault() {
    let folder = std::env::temp_dir().join("motion-bench-test-finish-clock");
    let _ = std::fs::remove_dir_all(&folder);
    let mut recorder = Recorder::begin(&folder, device(31)).unwrap();
    let stem = recorder.stem().to_string();
    // an anchor from the future makes the one taken while finishing look like the clock went back
    let mut future = crate::capture::clock::Anchor::take();
    future.qpc_before += 1 << 40;
    future.qpc_after += 1 << 40;
    let _ = recorder
        .clock_monitor
        .observe(&future, crate::capture::clock::frequency());
    let closed = recorder.close(None);
    let manifest = closed.manifest.clone();
    let failure = closed.publish(&manifest).unwrap_err();
    assert_eq!(failure.reason, crate::evidence::reason::Reason::ClockFailed);
    // the failed recording still publishes its metadata and takes the marker off
    assert!(folder.join(format!("{stem}.json")).is_file());
    assert!(!folder.join(format!("{stem}.active.json")).exists());
    std::fs::remove_dir_all(folder).unwrap();
}

/// a mouse unplugged while capture runs, and another one (or the same one in its bootloader) at
/// its old address, must not leave the first one's descriptors in the next recording, or the
/// recording reads as an address reuse and every comparison refuses it
#[test]
fn a_different_device_at_the_same_address_replaces_the_descriptors_before_it() {
    let root = env!("CARGO_MANIFEST_DIR");
    let descriptors = std::fs::read(format!("{root}/tests/fixtures/descriptors.pcap")).unwrap();
    let reports = std::fs::read(format!("{root}/tests/fixtures/mouse-1000hz.pcap")).unwrap();
    // the same descriptors with another product id stand in for the device that was there first
    let mut before = descriptors.clone();
    let id = [0x3b, 0x37, 0xd9, 0x11];
    // the fixture holds address 32's device descriptor first, then 31's
    let at = (0..before.len() - 4)
        .filter(|&i| before[i..i + 4] == id)
        .nth(1)
        .unwrap();
    before[at + 2..at + 4].copy_from_slice(&[0x34, 0x12]);
    let mut stream = before;
    stream.extend_from_slice(&descriptors[24..]);
    stream.extend_from_slice(&reports[24..]);
    let folder = std::env::temp_dir().join("motion-bench-test-replaced");
    let _ = std::fs::remove_dir_all(&folder);

    let mut tee = Tee::default();
    let mut recorder = None;
    read_pcap(&stream[..], |record| {
        let first_report = recorder.is_none()
            && matches!(
                record,
                Record::Packet {
                    event: Some(Event::Report(_)),
                    ..
                }
            );
        if first_report {
            recorder = Some(Recorder::begin(&folder, device(31)).unwrap());
        }
        tee.offer(recorder.as_mut(), &record);
    })
    .unwrap();
    let recorder = recorder.unwrap();
    let stem = recorder.stem().to_string();
    finish(recorder, None);

    let written = std::fs::read(folder.join(format!("{stem}.pcap"))).unwrap();
    let mut ids = Vec::new();
    read_pcap(&written[..], |record| {
        if let Record::Packet {
            event: Some(Event::Device {
                address: 31, pid, ..
            }),
            ..
        } = record
        {
            ids.push(pid)
        }
    })
    .unwrap();
    assert!(!ids.is_empty());
    assert!(ids.iter().all(|&pid| pid == 0x11d9), "{ids:x?}");
    let _ = std::fs::remove_dir_all(&folder);
}
