use super::*;
use crate::evidence::reader::recording::open_recording;
use crate::evidence::reason::Reason;
use crate::testkit::{
    faults::Fault,
    traffic::{
        CONTROL, Descriptors, Interface, Packet, TempDir, Traffic, fixture, mouse_payload,
        record_time, ticks,
    },
};

fn mouse_train(rate_hz: u64, count: u64) -> Traffic {
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(ticks(rate_hz, 1_000_000, count).map(|t| {
        let step = (t / 1000 % 5) as i16 - 2;
        (
            t,
            Packet::report(1, 2, 0x81, &mouse_payload(step, -step, 7)),
        )
    }));
    traffic
}

#[test]
fn eight_khz_ingestion_does_not_depend_on_presentation() {
    let mut data = Data::new(true);
    pcap::read_pcap(mouse_train(8000, 80_000).pcap().bytes(), |record| {
        data.ingest(&record)
    })
    .unwrap();
    assert_eq!(data.health.reports, 80_000);
    assert!(
        data.bus
            .streams
            .values()
            .next()
            .unwrap()
            .view(0, u64::MAX)
            .reports()
            <= 64_001
    );
    assert_eq!(data.selected, None);
}
fn report(data: &mut Data, t: u64, endpoint: u8) {
    data.ingest(&Record::Packet {
        header: &[0; 16],
        packet: &[],
        bus: 1,
        device: 2,
        event: Some(Event::Report(pcap::Report {
            t_us: t,
            bus: 1,
            device: 2,
            endpoint,
            len: 5,
            data: vec![0; 5],
        })),
    });
}
#[test]
fn windows_layout_survives_automatic_reselection_and_round_trips() {
    let mut data = Data::new(true);
    report(&mut data, 100, 0x81);
    let descriptor = [
        0x05, 1, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7f, 0x75, 8, 0x95, 2, 0x81, 6,
    ];
    let key = Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    data.remember_windows_layout(key, Layout::parse(&descriptor).unwrap());
    data.select("1:2:129", None).unwrap();
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    data.select("1:2:129", None).unwrap();
    let decoder = data.decoders.get(&key).unwrap();
    assert!(matches!(decoder, Decoder::WindowsDescriptor(_)));
    let saved = serde_json::to_string(decoder).unwrap();
    let restored: Decoder = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.decode(&[255, 2]), Some((-1, 2)));
}
#[test]
fn selection_survives_busier_composite_endpoint_but_not_disconnect() {
    let mut data = Data::new(true);
    report(&mut data, 100, 0x81);
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    for t in 100..200 {
        report(&mut data, t, 0x82);
    }
    assert_eq!(data.selected.unwrap().endpoint, 0x81);
    data.ingest(&Record::Packet {
        header: &[0; 16],
        packet: &[],
        bus: 1,
        device: 2,
        event: Some(Event::Disconnected { bus: 1, address: 2 }),
    });
    assert!(data.selected.is_none());
    assert!(data.delivery.is_empty());
    // capture goes on, the device has to be selected again
    assert!(data.health.error.is_none());
    assert!(data.health.notice.is_some());
    assert!(data.plan("test", None).is_err());
}

#[test]
fn a_selected_device_that_reappears_fails_with_its_own_code() {
    let device = |vid| Record::Packet {
        header: &[0; 16],
        packet: &[],
        bus: 1,
        device: 2,
        event: Some(Event::Device {
            bus: 1,
            address: 2,
            vid,
            pid: 0x11d9,
        }),
    };
    // live capture treats any new descriptor at the selected address as re-enumeration, a
    // replay only fails when the address now names another device
    for (live, vid, reason) in [
        (true, 0x373b, "device_reenumerated"),
        (false, 0x1234, "address_reused"),
    ] {
        let folder = TempDir::new("reappears");
        let mut data = Data::new(live);
        data.ingest(&device(0x373b));
        report(&mut data, 100, 0x81);
        data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
            .unwrap();
        data.trust_selection();
        let stem = data.begin("test", folder.path(), None).unwrap();
        data.ingest(&device(vid));
        // the first failure is the one kept, a later one does not replace its code
        data.fail(Reason::WriterFailed, "later");
        assert!(data.selected.is_none());
        assert!(data.health.error.is_none(), "{reason}");
        assert!(data.finish().is_err());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(folder.path().join(format!("{stem}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["reason"], reason);
    }
}
#[test]
fn a_regression_of_another_device_is_left_out_and_fails_nothing() {
    let mut data = Data::new(false);
    report(&mut data, 200, 0x81);
    report(&mut data, 100, 0x81);
    let key = Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    assert_eq!(data.bus.streams[&key].regressions, 1);
    // the count beside the selected mouse is the selected mouse's
    assert_eq!(data.health.timestamp_regressions, 0);
    assert_eq!(data.health.reports, 1);
    assert!(data.health.error.is_none());
    assert!(data.health.notice.is_none());
    // a saved capture reads with another device chosen, and not with this one
    data.ingest(&Record::Packet {
        header: &[0; 16],
        packet: &[],
        bus: 1,
        device: 3,
        event: Some(Event::Report(pcap::Report {
            t_us: 300,
            bus: 1,
            device: 3,
            endpoint: 0x81,
            len: 5,
            data: vec![0; 5],
        })),
    });
    data.select("1:3:129", None).unwrap();
    assert!(data.read_error().is_none());
    data.select("1:2:129", None).unwrap();
    assert!(data.read_error().is_some());
}
#[test]
fn choosing_a_device_again_checks_it_again() {
    let mut data = Data::new(true);
    report(&mut data, 100, 0x81);
    let mut t = 200;
    let mut send = |data: &mut Data, count: u32| {
        for _ in 0..count {
            report(data, t, 0x81);
            t += 1000;
        }
    };
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    send(&mut data, VALIDATION_REPORTS);
    assert!(data.readiness().is_ok());
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    assert!(data.readiness().is_err());
    send(&mut data, VALIDATION_REPORTS - 1);
    assert!(data.readiness().is_err());
    send(&mut data, 1);
    assert!(data.readiness().is_ok());
}
#[test]
fn a_regression_of_the_selected_device_makes_it_check_again() {
    let mut data = Data::new(true);
    report(&mut data, 100, 0x81);
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    let mut t = 200;
    let mut send = |data: &mut Data, count: u32| {
        for _ in 0..count {
            report(data, t, 0x81);
            t += 1000;
        }
    };
    send(&mut data, VALIDATION_REPORTS - 1);
    assert!(data.plan("test", None).is_err());
    send(&mut data, 1);
    assert!(data.plan("test", None).is_ok());
    report(&mut data, 150, 0x81);
    assert_eq!(data.validated, 0);
    assert!(data.health.error.is_none());
    assert!(data.health.notice.is_some());
    assert!(data.read_error().is_some());
    assert!(data.plan("test", None).is_err());
    send(&mut data, VALIDATION_REPORTS);
    assert!(data.plan("test", None).is_ok());
    // the device passed its checks again, so what was wrong with it is no longer news
    assert!(data.health.notice.is_none());
}
#[test]
fn eight_khz_recording_finalizes_without_frontend_consumption() {
    let folder = TempDir::new("eight-khz");
    let mut data = Data::new(true);
    let mut started = false;
    pcap::read_pcap(mouse_train(8000, 80_000).pcap().bytes(), |record| {
        data.ingest(&record);
        if !started && data.health.reports == 1 {
            let id = data.devices()[0].id.clone();
            data.select(&id, Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            data.begin("test", folder.path(), None).unwrap();
            started = true;
        }
    })
    .unwrap();
    let path = data.finish().unwrap();
    let replay = open_recording(&path, None).unwrap();
    assert_eq!(replay.data().health.reports, 79_999);
    assert_eq!(
        replay.status,
        crate::evidence::reader::status::Status::Complete
    );
}

#[test]
fn a_box_name_with_control_characters_survives_the_recording_sidecar() {
    let name = "box\nname \"quoted\" \\ \u{1}, {x}";
    let info = crate::hardware::medius::Snapshot {
        name: name.into(),
        clone: format!("373B:11D9 {name}"),
        family: "Medius".into(),
        ..Default::default()
    }
    .sidecar();
    let folder = TempDir::new("odd-name");
    let mut data = Data::new(true);
    let mut started = false;
    pcap::read_pcap(mouse_train(1000, 100).pcap().bytes(), |record| {
        data.ingest(&record);
        if !started && data.health.reports == 1 {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            data.begin("test", folder.path(), Some(info.clone()))
                .unwrap();
            started = true;
        }
    })
    .unwrap();
    let path = data.finish().unwrap();
    let sidecar: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
    assert_eq!(sidecar["box"]["name"], name);
    assert_eq!(
        sidecar["box"]["medius"]["clone"],
        format!("373B:11D9 {name}")
    );
}

fn reports_in(path: &Path) -> Vec<pcap::Report> {
    let mut reports = Vec::new();
    pcap::read_pcap(
        BufReader::new(std::fs::File::open(path).unwrap()),
        |record| {
            if let Record::Packet {
                event: Some(Event::Report(report)),
                ..
            } = record
            {
                reports.push(report);
            }
        },
    )
    .unwrap();
    reports
}

fn consecutive_recordings_keep_monitoring_and_exclude_the_gap(source: &[u8]) {
    let folder = TempDir::new("consecutive");
    let mut data = Data::new(true);
    let mut paths = Vec::new();
    let mut original = Vec::new();
    pcap::read_pcap(source, |record| {
        data.ingest(&record);
        if let Record::Packet {
            event: Some(Event::Report(report)),
            ..
        } = record
        {
            original.push(report.clone());
            match original.len() {
                1 => {
                    data.select(
                        &data.devices()[0].id,
                        Some(crate::testkit::traffic::mouse_decoder()),
                    )
                    .unwrap();
                    data.trust_selection();
                    data.begin("test", folder.path(), None).unwrap();
                }
                100 | 400 => {
                    paths.push(data.finish().unwrap());
                    assert!(!data.health.ended);
                    assert!(data.recorder.is_none());
                    assert!(data.selected.is_some());
                }
                // the selection stays checked across recordings
                200 => {
                    data.begin("test", folder.path(), None).unwrap();
                }
                _ => {}
            }
        }
    })
    .unwrap();
    assert_eq!(paths.len(), 2);
    assert_ne!(paths[0], paths[1]);
    for (path, expected) in paths.iter().zip([&original[1..100], &original[200..400]]) {
        assert_eq!(reports_in(path), expected);
        assert_eq!(
            open_recording(path, None).unwrap().data().health.reports as usize,
            expected.len()
        );
    }
}

#[test]
fn consecutive_recordings_keep_monitoring_and_exclude_the_gap_between_runs() {
    consecutive_recordings_keep_monitoring_and_exclude_the_gap(&fixture("mouse-1000hz.pcap"));
    for rate_hz in [1000, 8000] {
        consecutive_recordings_keep_monitoring_and_exclude_the_gap(
            mouse_train(rate_hz, 1000).pcap().bytes(),
        );
    }
}

const START_US: u64 = 1_000_000;

/// a composite mouse at 1:2 with a second hid interface. with `foreign` set, a mouse and a
/// keyboard on the same bus and a mouse at the same address on bus 2, all on the selected
/// endpoint number, plus control traffic on every device. the offsets keep foreign packets
/// off the selected mouse's timestamps
fn bench_traffic(rate_hz: u64, foreign: bool) -> Traffic {
    let mut traffic = Traffic::default();
    let composite = Descriptors {
        vid: 0x373b,
        pid: 0x11d9,
        interfaces: vec![
            Interface::hid(1, 2, &[(0x81, 1, 8)]),
            Interface::hid(0, 0, &[(0x82, 4, 64)]),
        ],
    };
    traffic.describe(0, 1, 2, &composite);
    traffic.hid_descriptor(0, 1, 2, 0, 0x10);
    let get_status = [0x80, 0, 0, 0, 0, 0, 2, 0];
    let control = |bus: u16, device: u16, offset: u64| {
        ticks(10, START_US + offset, 10).flat_map(move |t| {
            [
                (t, Packet::setup(bus, device, get_status)),
                (t, Packet::control_in(bus, device, &[0, 0])),
            ]
        })
    };
    traffic
        .extend(ticks(rate_hz, START_US, rate_hz).map(|t| {
            let step = (t / 1000 % 7) as i16 - 3;
            (
                t,
                Packet::report(1, 2, 0x81, &mouse_payload(step, step / 2, 8)),
            )
        }))
        .extend(
            ticks(250, START_US + 13, 250).map(|t| (t, Packet::report(1, 2, 0x82, &[0x5a; 64]))),
        )
        .extend(control(1, 2, 7));
    if foreign {
        traffic.describe(0, 1, 3, &Descriptors::mouse(0x046d, 0xc08b));
        traffic.describe(0, 1, 4, &Descriptors::keyboard(0x046d, 0xc31c));
        traffic.describe(0, 2, 2, &Descriptors::mouse(0x1532, 0x0084));
        traffic
            .extend(
                ticks(rate_hz, START_US + 37, rate_hz)
                    .map(|t| (t, Packet::report(1, 3, 0x81, &mouse_payload(9, -9, 8)))),
            )
            .extend(
                ticks(125, START_US + 61, 125)
                    .map(|t| (t, Packet::report(1, 4, 0x81, &[0, 0, 4, 0, 0, 0, 0, 0]))),
            )
            .extend(
                ticks(rate_hz, START_US + 53, rate_hz)
                    .map(|t| (t, Packet::report(2, 2, 0x81, &mouse_payload(-5, 5, 8)))),
            )
            .extend(control(1, 3, 3))
            .extend(control(1, 4, 5))
            .extend(control(2, 2, 9));
    }
    traffic.sorted()
}

/// selects 1:2:0x81 and records from the first packet at or after `begin_us` up to the first
/// packet at or after `end_us`
fn record_selected(
    traffic: &Traffic,
    folder: &Path,
    begin_us: u64,
    end_us: u64,
) -> (Data, Vec<u8>) {
    let mut data = Data::new(true);
    let mut path = None;
    pcap::read_pcap(traffic.pcap().bytes(), |record| {
        data.ingest(&record);
        let Record::Packet { header, .. } = &record else {
            return;
        };
        let t = record_time(header);
        if data.recorder.is_none() && path.is_none() && t >= begin_us {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            data.begin("test", folder, None).unwrap();
        } else if data.recorder.is_some() && t >= end_us {
            path = Some(data.finish().unwrap());
        }
    })
    .unwrap();
    let bytes = std::fs::read(path.unwrap()).unwrap();
    (data, bytes)
}

#[test]
fn foreign_traffic_never_reaches_the_selected_recording_or_its_live_stream() {
    for rate_hz in [1000, 8000] {
        let folder = TempDir::new("isolation");
        let (begin_us, end_us) = (START_US + 200_000, START_US + 800_000);
        let traffic = bench_traffic(rate_hz, true);
        let (data, recorded) = record_selected(&traffic, folder.path(), begin_us, end_us);

        // the recording holds the selected device's descriptors from capture start, then its
        // control transfers and the selected endpoint's reports while recording. the packet
        // that triggers begin is ingested before the recorder exists, the one that triggers
        // finish is the last one it gets
        let mut expected = crate::testkit::traffic::Pcap::default();
        for (t, packet) in &traffic.0 {
            let selected = (packet.bus, packet.device) == (1, 2);
            let recording = (begin_us + 1..=end_us).contains(t);
            let kept =
                *t == 0 || recording && (packet.transfer == CONTROL || packet.endpoint == 0x81);
            if selected && kept {
                expected.record(*t, &packet.bytes());
            }
        }
        assert_eq!(recorded, expected.bytes(), "{rate_hz} Hz");
        assert!(data.health.error.is_none());

        let (alone, recorded_alone) = record_selected(
            &bench_traffic(rate_hz, false),
            folder.path(),
            begin_us,
            end_us,
        );
        assert_eq!(recorded, recorded_alone, "{rate_hz} Hz");
        let key = Key {
            bus: 1,
            device: 2,
            endpoint: 0x81,
        };
        let live = |data: &Data| {
            let stream = &data.bus.streams[&key];
            let view = stream.view(0, u64::MAX);
            (
                stream.total,
                view.motion().collect::<Vec<_>>(),
                view.gaps().collect::<Vec<_>>(),
            )
        };
        assert_eq!(live(&data), live(&alone), "{rate_hz} Hz");
        // selecting resets the live stream after the report at begin_us, so it holds the
        // rest of the one second train
        let after_begin = rate_hz - (begin_us - START_US) * rate_hz / 1_000_000 - 1;
        assert_eq!(live(&data).0, after_begin);
    }
}

/// records `traffic` from its first selected report to the end, calling `during` after
/// every packet that arrives while recording
fn record_train(
    traffic: &Traffic,
    folder: &Path,
    mut during: impl FnMut(&mut Data),
) -> (Result<PathBuf, String>, String) {
    let mut data = Data::new(true);
    let mut stem = None;
    pcap::read_pcap(traffic.pcap().bytes(), |record| {
        data.ingest(&record);
        if data.recorder.is_some() {
            during(&mut data);
        } else if stem.is_none() && data.health.reports > 0 {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            stem = Some(data.begin("test", folder, None).unwrap());
        }
    })
    .unwrap();
    (data.finish(), stem.unwrap())
}

struct Run {
    folder: TempDir,
    stem: String,
    finish: Result<PathBuf, String>,
}

impl Run {
    fn new(traffic: &Traffic, during: impl FnMut(&mut Data)) -> Self {
        let folder = TempDir::new("faults");
        let (finish, stem) = record_train(traffic, folder.path(), during);
        Run {
            folder,
            stem,
            finish,
        }
    }

    fn file(&self, kind: &str) -> Vec<u8> {
        std::fs::read(self.path(kind)).unwrap()
    }

    fn path(&self, kind: &str) -> PathBuf {
        self.folder.path().join(format!("{}.{kind}", self.stem))
    }

    /// a failed recording publishes its metadata, saying it failed and why, and keeps its
    /// evidence. nothing is left to say it is still running
    fn assert_failed_with_evidence(&self, reason: &str) {
        assert!(self.finish.is_err(), "{:?}", self.finish);
        let manifest: serde_json::Value = serde_json::from_slice(&self.file("json")).unwrap();
        assert_eq!(manifest["outcome"], "failed");
        assert_eq!(manifest["health"]["reason"], reason, "{manifest}");
        assert!(manifest["health"]["error"].is_string());
        assert_eq!(manifest["paired"]["shift_us"], serde_json::Value::Null);
        assert!(!self.path("incomplete.json").exists());
        assert!(!self.path("active.json").exists());
        assert!(self.path("pcap").is_file());
        // and the reader takes it as failed, with the same code
        let opened =
            crate::evidence::reader::recording::open_recording(&self.path("pcap"), None).unwrap();
        let crate::evidence::reader::status::Status::Failed {
            reason: Some(read), ..
        } = opened.status
        else {
            panic!("{:?}", opened.status)
        };
        assert_eq!(serde_json::to_value(read).unwrap(), reason);
    }
}

#[test]
fn a_writer_fault_keeps_everything_written_before_it() {
    let traffic = mouse_train(1000, 1000);
    let reference = Run::new(&traffic, |_| {});
    assert!(reference.finish.is_ok());
    let full_csv = reference.file("csv");
    let full_pcap = reference.file("pcap");

    let limit = full_csv.len() as u64 / 2;
    let run = {
        let _fault = crate::testkit::faults::inject("csv", Fault::WriteAfter(limit));
        Run::new(&traffic, |_| {})
    };
    run.assert_failed_with_evidence("writer_failed");
    assert_eq!(run.file("csv"), full_csv[..limit as usize]);
    let pcap = run.file("pcap");
    assert!(full_pcap.starts_with(&pcap));
    let complete_rows = run.file("csv").iter().filter(|b| **b == b'\n').count() - 1;
    assert!(reports_in(&run.path("pcap")).len() >= complete_rows);

    // a write that fails once and then works again must still fail the recording, even
    // though the file can be flushed whole at the end
    let run = {
        let _fault = crate::testkit::faults::inject("csv", Fault::FailOnceAt(limit));
        Run::new(&traffic, |_| {})
    };
    run.assert_failed_with_evidence("writer_failed");

    let limit = full_pcap.len() as u64 / 2;
    let run = {
        let _fault = crate::testkit::faults::inject("pcap", Fault::WriteAfter(limit));
        Run::new(&traffic, |_| {})
    };
    run.assert_failed_with_evidence("writer_failed");
    assert_eq!(run.file("pcap"), full_pcap[..limit as usize]);
}

#[test]
fn a_fault_while_finishing_keeps_the_whole_capture() {
    let traffic = mouse_train(1000, 1000);
    let full_pcap = Run::new(&traffic, |_| {}).file("pcap");
    for (kind, fault) in [
        ("motion.csv", Fault::Flush),
        ("pcap", Fault::Flush),
        ("csv", Fault::Flush),
        ("clocks.jsonl", Fault::Flush),
        ("clocks.jsonl", Fault::Sync),
        ("pcap", Fault::Sync),
        ("csv", Fault::Sync),
        ("motion.csv", Fault::Sync),
    ] {
        let run = {
            let _fault = crate::testkit::faults::inject(kind, fault);
            Run::new(&traffic, |_| {})
        };
        run.assert_failed_with_evidence("writer_failed");
        assert_eq!(run.file("pcap"), full_pcap, "{kind} {fault:?}");
    }
}

#[test]
fn a_failed_recording_leaves_capture_ready_for_the_next_one() {
    let folder = TempDir::new("finish-code");
    let mut fault = Some(crate::testkit::faults::inject("motion.csv", Fault::Flush));
    let mut data = Data::new(true);
    let mut stems = Vec::new();
    let mut finished = Vec::new();
    let mut notice = None;
    pcap::read_pcap(mouse_train(1000, 400).pcap().bytes(), |record| {
        data.ingest(&record);
        if stems.is_empty() && data.health.reports > 0 {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            stems.push(data.begin("test", folder.path(), None).unwrap());
        } else if finished.is_empty() && data.health.reports == 200 {
            finished.push(data.finish());
            notice = data.health.notice.clone();
            fault.take();
            stems.push(data.begin("test", folder.path(), None).unwrap());
        }
    })
    .unwrap();
    finished.push(data.finish());
    let manifest = |stem: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(folder.path().join(format!("{stem}.json"))).unwrap())
            .unwrap()
    };
    assert!(finished[0].is_err());
    assert_eq!(manifest(&stems[0])["health"]["reason"], "writer_failed");
    assert_eq!(manifest(&stems[0])["outcome"], "failed");
    assert!(notice.unwrap().starts_with("the recording failed"));
    assert!(data.health.error.is_none());
    assert!(finished[1].is_ok(), "{:?}", finished[1]);
    assert_eq!(manifest(&stems[1])["outcome"], "complete");
    // its counters are its own, not capture's since it started
    assert_eq!(manifest(&stems[1])["health"]["reports"], 200);
    assert_eq!(
        manifest(&stems[1])["health"]["error"],
        serde_json::Value::Null
    );
}

#[test]
fn a_file_that_cannot_be_created_refuses_the_recording() {
    for kind in ["csv", "motion.csv", "clocks.jsonl", "pcap"] {
        let folder = TempDir::new("create");
        let mut data = Data::new(true);
        pcap::read_pcap(mouse_train(1000, 2).pcap().bytes(), |record| {
            data.ingest(&record)
        })
        .unwrap();
        data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
            .unwrap();
        let _fault = crate::testkit::faults::inject(kind, Fault::Create);
        data.trust_selection();
        assert!(data.begin("test", folder.path(), None).is_err(), "{kind}");
        assert!(data.recorder.is_none(), "{kind}");
        // the files made before it and the marker go, they belong to this attempt only
        let left: Vec<_> = std::fs::read_dir(folder.path()).unwrap().collect();
        assert!(left.is_empty(), "{kind}: {left:?}");
    }
}

#[test]
fn a_lost_or_regressing_source_keeps_the_reports_before_it() {
    let mut unplugged = mouse_train(1000, 500);
    unplugged.add(1_500_000, Packet::disconnect(1, 2, 0x81));
    let mut regressing = mouse_train(1000, 500);
    regressing.add(
        1_100_000,
        Packet::report(1, 2, 0x81, &mouse_payload(1, 1, 7)),
    );
    for (traffic, reason) in [
        (unplugged, "device_lost"),
        (regressing, "timestamp_regressed"),
    ] {
        let run = Run::new(&traffic, |_| {});
        run.assert_failed_with_evidence(reason);
        // the first report starts the recording, the next 499 are in it
        assert!(reports_in(&run.path("pcap")).len() >= 499);
    }
}

#[test]
fn a_failing_participant_keeps_the_capture() {
    let traffic = mouse_train(1000, 500);
    let motion = |box_us| crate::hardware::medius::Motion {
        arrived_us: wall_us(),
        arrived_qpc: crate::capture::clock::qpc(),
        box_us,
        dx: 1,
        dy: 0,
        dz: 0,
    };
    let mut sent = 0;
    let clock_reset = Run::new(&traffic, |data| {
        sent += 1;
        if sent == 100 || sent == 200 {
            data.input(&motion(1_000_000 - sent * 1000));
        }
    });
    let mut sent = 0;
    let unwritable = {
        let _fault = crate::testkit::faults::inject("input.csv", Fault::Create);
        Run::new(&traffic, |data| {
            sent += 1;
            if sent == 100 {
                data.input(&motion(1_000_000));
            }
        })
    };
    let mut sent = 0;
    let unwritable_receipts = {
        let _fault = crate::testkit::faults::inject("input-arrivals.csv", Fault::Create);
        Run::new(&traffic, |data| {
            sent += 1;
            if sent == 100 {
                data.input(&motion(1_000_000));
            }
        })
    };
    // the receipt clock steps back while the box clock moves on, so only the alignment fails
    let mut sent = 0;
    let receipts_regressed = Run::new(&traffic, |data| {
        sent += 1;
        if sent == 100 || sent == 200 {
            let mut late = motion(1_000_000 + sent * 1000);
            late.arrived_qpc -= sent as i64 * 1_000_000;
            data.input(&late);
        }
    });
    // a participant the desktop runs, failing in its own words, still writes the same code
    let participant = Run::new(&traffic, |data| {
        data.fail(
            Reason::ParticipantFailed,
            "screen recording could not start: anything",
        );
    });
    for (run, reason) in [
        (unwritable, "writer_failed"),
        (unwritable_receipts, "writer_failed"),
        (participant, "participant_failed"),
    ] {
        run.assert_failed_with_evidence(reason);
        assert_eq!(reports_in(&run.path("pcap")).len(), 499);
    }
    // the box's clock going back or its receipts no longer lining up only cost the pairing
    for run in [&clock_reset, &receipts_regressed] {
        assert!(run.finish.is_ok(), "{:?}", run.finish);
        let manifest: serde_json::Value = serde_json::from_slice(&run.file("json")).unwrap();
        assert_eq!(manifest["outcome"], "degraded", "{manifest}");
        assert_eq!(manifest["health"]["pairing_valid"], false);
        assert_eq!(manifest["health"]["error"], serde_json::Value::Null);
        assert_eq!(manifest["paired"]["shift_us"], serde_json::Value::Null);
        assert_eq!(manifest["participants"][0]["id"], "medius_input");
        assert_eq!(manifest["participants"][0]["outcome"], "failed");
        assert_eq!(reports_in(&run.path("pcap")).len(), 499);
    }
    // nothing after the reset is written, it would sit on another clock
    let input = String::from_utf8(clock_reset.file("input.csv")).unwrap();
    let rows = input.lines().filter(|l| !l.starts_with('#')).skip(1);
    assert_eq!(rows.collect::<Vec<_>>(), ["900000,1,0,0"]);
}

fn live(pcap: &crate::testkit::traffic::Pcap) -> Data {
    let mut data = Data::new(true);
    pcap::read_pcap(pcap.bytes(), |record| data.ingest(&record)).unwrap();
    data
}

#[test]
fn a_full_descriptor_cache_fails_capture_explicitly() {
    let setup = Packet::setup(1, 2, [0x80, 6, 0, 1, 0, 0, 18, 0]).bytes();
    let setups = |count| {
        let mut pcap = crate::testkit::traffic::Pcap::default();
        for _ in 0..count {
            pcap.record(0, &setup);
        }
        live(&pcap)
    };
    assert!(setups(4096).health.error.is_none());
    let full = setups(4097).health;
    assert!(full.error.unwrap().contains("descriptor cache"));
    assert_eq!(full.reason, Some(Reason::DescriptorCacheFull));

    // each pair caches a 52 byte request and a 60,044 byte answer, so 279 pairs stay under
    // 16 MiB and 280 do not
    let answers = |pairs: u64| {
        let mut pcap = crate::testkit::traffic::Pcap::default();
        for irp in 0..pairs {
            let request = Packet {
                irp,
                ..Packet::setup(1, 2, [0x81, 6, 0, 0x22, 0, 0, 0x60, 0xea])
            };
            let answer = Packet {
                irp,
                ..Packet::control_in(1, 2, &[0; 60_000])
            };
            pcap.record(0, &request.bytes()).record(0, &answer.bytes());
        }
        live(&pcap)
    };
    assert!(answers(279).health.error.is_none());
    let full = answers(280).health.error.unwrap();
    assert!(full.contains("descriptor cache"), "{full}");
}

#[test]
fn reports_longer_than_64_bytes_are_recorded_whole() {
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    let payloads: Vec<Vec<u8>> = (0..300u64)
        .map(|i| (0..200u64).map(|b| (i + b) as u8).collect())
        .collect();
    traffic.extend(
        ticks(1000, START_US, 300)
            .zip(&payloads)
            .map(|(t, payload)| (t, Packet::report(1, 2, 0x81, payload))),
    );
    let run = Run::new(&traffic, |_| {});
    assert!(run.finish.is_ok(), "{:?}", run.finish);
    let recorded: Vec<Vec<u8>> = reports_in(&run.path("pcap"))
        .into_iter()
        .map(|report| report.bytes().to_vec())
        .collect();
    assert!(recorded.len() >= 299);
    assert_eq!(recorded, payloads[payloads.len() - recorded.len()..]);
}

#[test]
fn descriptors_that_arrive_late_still_decode_the_reports_before_them() {
    let mut traffic = Traffic::default();
    traffic.extend(ticks(1000, START_US, 100).map(|t| (t, Packet::report(1, 2, 0x81, &[3, 0xfd]))));
    let end = START_US + 100_000;
    traffic.describe(end, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.hid_descriptor(end, 1, 2, 0, 0x20);
    let folder = TempDir::new("late-descriptors");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("late.pcap");
    std::fs::write(&path, traffic.pcap().bytes()).unwrap();
    let mut metadata = crate::testkit::fixtures::sidecar("complete");
    metadata.as_object_mut().unwrap().remove("decoder");
    std::fs::write(path.with_extension("json"), metadata.to_string()).unwrap();

    let data = open_recording(&path, None).unwrap().into_data();
    let key = Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    assert!(matches!(data.decoders[&key], Decoder::Descriptor(_)));
    let motion: Vec<(i32, i32)> = data.bus.streams[&key]
        .view(0, u64::MAX)
        .motion()
        .map(|(_, dx, dy)| (dx, dy))
        .collect();
    assert_eq!(motion, vec![(3, -3); 100]);
}

#[test]
fn a_backward_report_keeps_its_raw_bytes_but_never_wraps_derived_time() {
    // the train runs from 1 s to 1.5 s and the first report only starts the recording. after it
    // come a report with the latest stamp again, one stamped before every recorded report and
    // one that moved forward again but is still behind the latest
    let mut traffic = mouse_train(1000, 500);
    for t in [1_499_000, 1_000_500, 1_000_600] {
        traffic.add(t, Packet::report(1, 2, 0x81, &mouse_payload(3, 3, 7)));
    }
    let run = Run::new(&traffic, |_| {});
    run.assert_failed_with_evidence("timestamp_regressed");
    assert!(
        reports_in(&run.path("pcap"))
            .iter()
            .any(|r| r.t_us == 1_000_500)
    );
    let csv = String::from_utf8(run.file("csv")).unwrap();
    assert!(csv.lines().any(|row| row.starts_with("1000500,,")));
    assert!(csv.lines().any(|row| row.starts_with("1000600,,")));
    assert!(csv.lines().any(|row| row.starts_with("1499000,0,")));
    let motion = String::from_utf8(run.file("motion.csv")).unwrap();
    let times: Vec<u64> = motion
        .lines()
        .filter_map(|row| row.split(',').next()?.parse().ok())
        .collect();
    assert!(times.len() >= 400);
    assert!(times.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(times.iter().all(|t| *t < 1_000_000));
    // the equal stamp keeps its motion row, the two behind the latest get none
    let latest = 1_499_000 - 1_001_000;
    assert_eq!(times.iter().filter(|t| **t == latest).count(), 2);
    assert_eq!(times.last(), Some(&latest));
}

#[test]
fn another_device_failing_during_a_recording_leaves_it_untouched() {
    let folder = TempDir::new("foreign-faults");
    let (begin_us, end_us) = (START_US + 200_000, START_US + 800_000);
    let mut traffic = bench_traffic(1000, true);
    // a report of 1:3 stamped behind the ones it already sent, then 2:2 unplugged
    let at = |traffic: &Traffic, t| traffic.0.iter().position(|(at, _)| *at >= t).unwrap();
    let index = at(&traffic, START_US + 500_000);
    traffic.0.insert(
        index,
        (
            START_US + 400_000,
            Packet::report(1, 3, 0x81, &mouse_payload(1, 1, 8)),
        ),
    );
    traffic
        .0
        .retain(|(t, packet)| packet.bus != 2 || *t < START_US + 600_000);
    let index = at(&traffic, START_US + 600_000);
    traffic
        .0
        .insert(index, (START_US + 600_000, Packet::disconnect(2, 2, 0x81)));
    let (data, recorded) = record_selected(&traffic, folder.path(), begin_us, end_us);
    let foreign = Key {
        bus: 1,
        device: 3,
        endpoint: 0x81,
    };
    assert_eq!(data.bus.streams[&foreign].regressions, 1);
    assert!(!data.bus.streams.keys().any(|key| key.bus == 2));
    assert_eq!(data.health.timestamp_regressions, 0);
    assert!(data.health.error.is_none());
    assert!(data.health.notice.is_none());
    let (_, alone) = record_selected(&bench_traffic(1000, false), folder.path(), begin_us, end_us);
    assert_eq!(recorded, alone);
}

#[test]
fn an_optional_participant_degrades_the_recording_and_the_reader_agrees() {
    use crate::evidence::reader::{
        eligibility::{COMPARISON, PAIRED, Stream, TARGETS, eligibility},
        recording::open_recording,
        status::{Degradation, Status},
    };
    let traffic = mouse_train(1000, 300);
    let motion = |box_us| crate::hardware::medius::Motion {
        arrived_us: wall_us(),
        arrived_qpc: crate::capture::clock::qpc(),
        box_us,
        dx: 1,
        dy: 0,
        dz: 0,
    };
    let mut sent = 0;
    let observers = Run::new(&traffic, |data| {
        data.take_part("injection");
        data.degrade("observers", "the observer stopped");
    });
    let vision = Run::new(&traffic, |data| {
        data.take_part("vision");
        sent += 1;
        if sent == 100 {
            data.degrade("vision", "frames were lost");
        }
    });
    let mut sent = 0;
    let reset = Run::new(&traffic, |data| {
        data.take_part("medius_input");
        sent += 1;
        if sent == 100 || sent == 200 {
            data.input(&motion(1_000_000 - sent * 1000));
        }
    });
    for (run, degraded, paired, targets) in [
        (&observers, vec![Degradation::ParticipantFailed], true, true),
        (&vision, vec![Degradation::TargetsFailed], true, false),
        (&reset, vec![Degradation::PairingInvalid], false, true),
    ] {
        assert!(run.finish.is_ok(), "{:?}", run.finish);
        let recording = open_recording(&run.path("pcap"), None).unwrap();
        assert_eq!(recording.status, Status::Degraded(degraded));
        let allows = |needs| {
            eligibility(
                &recording.metadata,
                &recording.status,
                // the degradation decides here, as if the box input were saved
                &[Stream::PhysicalInput],
                needs,
            )
            .is_ok()
        };
        assert!(allows(COMPARISON));
        assert_eq!(allows(PAIRED), paired);
        assert_eq!(allows(TARGETS), targets);
    }
    // what took part and did not fail is listed as complete
    let manifest: serde_json::Value = serde_json::from_slice(&observers.file("json")).unwrap();
    assert_eq!(
        manifest["participants"],
        serde_json::json!([
            {"id": "injection", "outcome": "complete", "error": null},
            {"id": "observers", "outcome": "failed", "error": "the observer stopped"},
        ])
    );
}

#[test]
fn a_recording_that_failed_asks_to_be_ended() {
    let folder = TempDir::new("failed-ends");
    let mut data = Data::new(true);
    report(&mut data, 100, 0x81);
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    data.trust_selection();
    data.begin("test", folder.path(), None).unwrap();
    report(&mut data, 200, 0x81);
    assert!(!data.recording_failed());
    // another device's trouble is not this recording's
    data.ingest(&Record::Packet {
        header: &[0; 16],
        packet: &[],
        bus: 1,
        device: 3,
        event: Some(Event::Disconnected { bus: 1, address: 3 }),
    });
    assert!(!data.recording_failed());
    data.ingest(&Record::Packet {
        header: &[0; 16],
        packet: &[],
        bus: 1,
        device: 2,
        event: Some(Event::Disconnected { bus: 1, address: 2 }),
    });
    assert!(data.recording_failed());
}

#[test]
fn the_injection_fails_only_a_recording_it_took_part_in_and_the_box_only_pairs_its_own() {
    let unarmed = Run::new(&mouse_train(1000, 100), |data| {
        data.fail_injection("the button feed was lost");
        data.degrade_pairing("Medius reports were lost");
    });
    assert!(unarmed.finish.is_ok(), "{:?}", unarmed.finish);
    let manifest: serde_json::Value = serde_json::from_slice(&unarmed.file("json")).unwrap();
    assert_eq!(manifest["outcome"], "complete");
    assert_eq!(manifest["health"]["pairing_valid"], true);
    assert_eq!(manifest["participants"], serde_json::json!([]));

    let armed = Run::new(&mouse_train(1000, 100), |data| {
        data.take_part(crate::evidence::commit::participant::INJECTION);
        data.fail_injection("the aim controller stopped");
    });
    armed.assert_failed_with_evidence("participant_failed");
    let manifest: serde_json::Value = serde_json::from_slice(&armed.file("json")).unwrap();
    assert_eq!(manifest["participants"][0]["id"], "injection");
    assert_eq!(manifest["participants"][0]["outcome"], "failed");
}

#[test]
fn a_report_the_layout_cannot_decode_is_skipped_and_without_a_layout_every_report_counts() {
    let undecodable = |data: &mut Data, t| {
        data.ingest(&Record::Packet {
            header: &[0; 16],
            packet: &[],
            bus: 1,
            device: 2,
            event: Some(Event::Report(pcap::Report {
                t_us: t,
                bus: 1,
                device: 2,
                endpoint: 0x81,
                len: 0,
                data: Vec::new(),
            })),
        })
    };
    let mut data = Data::new(true);
    report(&mut data, 100, 0x81);
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    for t in 0..VALIDATION_REPORTS as u64 - 1 {
        report(&mut data, 200 + t, 0x81);
    }
    undecodable(&mut data, 300);
    assert!(data.readiness().is_err());
    report(&mut data, 301, 0x81);
    assert!(data.readiness().is_ok());

    let mut data = Data::new(true);
    undecodable(&mut data, 100);
    data.select("1:2:129", Some(Decoder::Unavailable)).unwrap();
    for t in 0..VALIDATION_REPORTS as u64 {
        undecodable(&mut data, 200 + t);
    }
    assert!(data.readiness().is_ok());
}

#[test]
fn a_recording_the_box_degraded_leaves_the_next_one_whole() {
    let folder = TempDir::new("after-degraded");
    let motion = |box_us| crate::hardware::medius::Motion {
        arrived_us: wall_us(),
        arrived_qpc: crate::capture::clock::qpc(),
        box_us,
        dx: 1,
        dy: 0,
        dz: 0,
    };
    let mut data = Data::new(true);
    let mut stems = Vec::new();
    let mut notice = None;
    let mut sent = 0u64;
    pcap::read_pcap(mouse_train(1000, 400).pcap().bytes(), |record| {
        data.ingest(&record);
        let reports = data.health.reports;
        if stems.is_empty() && reports > 0 {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            stems.push(data.begin("test", folder.path(), None).unwrap());
        } else if stems.len() == 1 && reports == 200 {
            data.finish().unwrap();
            notice = data.health.notice.clone();
            stems.push(data.begin("test", folder.path(), None).unwrap());
        }
        if data.recorder.is_some() && matches!(record, Record::Packet { .. }) {
            sent += 1;
            // the box's clock goes back once, in the first recording only
            let box_us = if sent == 100 {
                1_000
            } else {
                1_000_000 + sent * 1000
            };
            data.input(&motion(box_us));
        }
    })
    .unwrap();
    // the notice of the first recording is gone once the second starts
    assert!(notice.is_some());
    assert!(data.health.notice.is_none());
    data.finish().unwrap();
    let manifest = |stem: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(folder.path().join(format!("{stem}.json"))).unwrap())
            .unwrap()
    };
    assert_eq!(manifest(&stems[0])["outcome"], "degraded");
    // the live notice is not the recording's health
    assert!(manifest(&stems[0])["health"].get("notice").is_none());
    let second = manifest(&stems[1]);
    assert_eq!(second["outcome"], "complete", "{second}");
    assert_eq!(second["health"]["pairing_valid"], true);
    let input =
        std::fs::read_to_string(folder.path().join(format!("{}.input.csv", stems[1]))).unwrap();
    assert!(input.lines().filter(|l| !l.starts_with('#')).count() > 100);
}

#[test]
fn an_unplug_that_only_cancels_and_aborts_still_loses_the_device() {
    // the reports stop, then the pending reads are cancelled and the pipe aborted, as the
    // maintainer's mouse did; no device gone status ever comes
    let mut traffic = mouse_train(1000, 300);
    for packet in Packet::unplug(1, 2, 0x81) {
        traffic.add(1_400_000, packet);
    }
    let run = Run::new(&traffic, |_| {});
    run.assert_failed_with_evidence("device_lost");
    let live = live(&traffic.pcap());
    assert!(live.devices().is_empty());
    assert!(live.health.error.is_none());
}

#[test]
fn an_abort_that_is_not_an_unplug_of_the_selected_endpoint_leaves_the_recording_whole() {
    let (submitted, completed) = Packet::abort(1, 2, 0x81);
    let failed_on = |bus, device, endpoint| {
        let (_, completed) = Packet::abort(bus, device, endpoint);
        Packet {
            status: 0x8000_0300,
            ..completed
        }
    };
    for (what, packets) in [
        ("an orderly abort", vec![submitted, completed]),
        ("another endpoint", vec![failed_on(1, 2, 0x82)]),
        ("the default pipe", vec![failed_on(1, 2, 0x00)]),
        ("another device", vec![failed_on(1, 3, 0x81)]),
    ] {
        let mut traffic = mouse_train(1000, 300);
        for packet in packets {
            traffic.add(1_150_000, packet);
        }
        let traffic = traffic.sorted();
        let run = Run::new(&traffic, |_| {});
        assert!(run.finish.is_ok(), "{what}: {:?}", run.finish);
        let live = live(&traffic.pcap());
        assert_eq!(live.devices()[0].kind, "mouse", "{what}");
    }
}

#[test]
fn capture_ends_with_what_usbpcap_said_whichever_pipe_closed_first() {
    let ended = |read: Result<(), &str>, stopping, said: &str| {
        ending(read.map_err(str::to_string), stopping, said).map(|f| (f.reason, f.message))
    };
    // a stop that was asked for is no failure, even with the reader cut off by the kill
    assert_eq!(ended(Err("broken pipe"), true, "killed"), None);
    assert_eq!(ended(Ok(()), true, ""), None);
    assert_eq!(
        ended(Ok(()), false, ""),
        Some((
            Reason::TransportEnded,
            "USBPcap stream ended unexpectedly".into()
        ))
    );
    assert_eq!(
        ended(Ok(()), false, "  driver not installed\n"),
        Some((Reason::DriverReported, "driver not installed".into()))
    );
    assert_eq!(
        ended(Err("truncated record"), false, ""),
        Some((Reason::TransportFailed, "truncated record".into()))
    );
    assert_eq!(
        ended(Err("truncated record"), false, "access denied"),
        Some((
            Reason::TransportFailed,
            "truncated record, and USBPcap said: access denied".into()
        ))
    );
}

#[test]
fn usbpcap_words_win_over_the_plain_stream_end_whichever_pipe_closes_first() {
    use crate::testkit::traffic::Traffic;
    use std::sync::{atomic::AtomicBool, mpsc};
    let ended = |delay: Option<u64>| {
        let data = Mutex::new(Data::new(true));
        let (said, heard) = mpsc::channel();
        let words = std::thread::spawn(move || {
            if let Some(delay) = delay {
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
            said.send("access denied".to_string()).unwrap();
        });
        if delay.is_none() {
            words.join().unwrap();
        }
        let bytes = Traffic::default().pcap().bytes().to_vec();
        stream(bytes.as_slice(), &heard, &AtomicBool::new(false), &data);
        let data = data.into_inner().unwrap();
        (data.health.reason, data.health.error, data.health.ended)
    };
    let driver = (
        Some(Reason::DriverReported),
        Some("access denied".to_string()),
        true,
    );
    // stderr closed first, then stdout, and the other way around
    assert_eq!(ended(None), driver);
    assert_eq!(ended(Some(200)), driver);
}

#[test]
fn usbpcap_words_that_come_after_capture_ended_are_added_to_its_failure() {
    use crate::testkit::traffic::Traffic;
    use std::sync::{atomic::AtomicBool, mpsc};
    let data = Mutex::new(Data::new(true));
    let (_said, heard) = mpsc::channel::<String>();
    let bytes = Traffic::default().pcap().bytes().to_vec();
    stream(bytes.as_slice(), &heard, &AtomicBool::new(false), &data);
    let mut data = data.into_inner().unwrap();
    assert_eq!(data.health.reason, Some(Reason::TransportEnded));
    data.add_driver_words(" access denied\n");
    assert_eq!(
        data.health.error.as_deref(),
        Some("USBPcap stream ended unexpectedly, and USBPcap said: access denied")
    );
    // the code stays the one the stream end gave
    assert_eq!(data.health.reason, Some(Reason::TransportEnded));
    // a capture stopped on purpose has no failure to add words to
    let mut stopped = Data::new(true);
    stopped.add_driver_words("killed");
    assert_eq!(stopped.health.error, None);
}

/// the same mouse at another address is still the one a pick meant: the box re-cloning it moves
/// the address and leaves the id a list was drawn from behind
#[test]
fn a_pick_finds_the_same_mouse_at_its_new_address() {
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, 40)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, 0, 7)))),
    );
    // it goes, and the same mouse comes back on another address
    traffic.extend(Packet::unplug(1, 2, 0x81).map(|packet| (1_100_000, packet)));
    traffic.describe(1_200_000, 1, 3, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_300_000, 40)
            .map(|t| (t, Packet::report(1, 3, 0x81, &mouse_payload(1, 0, 7)))),
    );
    let mut data = Data::new(true);
    pcap::read_pcap(traffic.sorted().pcap().bytes(), |record| {
        data.ingest(&record)
    })
    .unwrap();
    assert!(data.devices().iter().all(|d| d.id != "1:2:129"));
    assert!(data.devices().iter().any(|d| d.id == "1:3:129"));
    assert_eq!(data.device("1:2:129").map(|d| d.id), Some("1:3:129".into()));
    data.select("1:2:129", None).unwrap();
    assert_eq!(
        data.selected,
        Some(Key {
            bus: 1,
            device: 3,
            endpoint: 0x81,
        })
    );
}

/// a pick that cannot tell which device was meant is refused, and so is one whose device is gone
#[test]
fn a_pick_that_cannot_tell_which_device_was_meant_is_refused() {
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, 40)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, 0, 7)))),
    );
    traffic.extend(Packet::unplug(1, 2, 0x81).map(|packet| (1_100_000, packet)));
    // two of the same mouse on the same bus, and then another one entirely
    traffic.describe(1_200_000, 1, 3, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.describe(1_200_000, 1, 4, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.describe(1_200_000, 1, 5, &Descriptors::mouse(0x046d, 0xc52b));
    traffic.extend(ticks(1000, 1_300_000, 40).flat_map(|t| {
        [
            (t, Packet::report(1, 3, 0x81, &mouse_payload(1, 0, 7))),
            (t, Packet::report(1, 4, 0x81, &mouse_payload(1, 0, 7))),
            (t, Packet::report(1, 5, 0x81, &mouse_payload(1, 0, 7))),
        ]
    }));
    let mut data = Data::new(true);
    pcap::read_pcap(traffic.sorted().pcap().bytes(), |record| {
        data.ingest(&record)
    })
    .unwrap();
    // two rows could be the one the id named, so nothing is picked
    assert_eq!(data.device("1:2:129").map(|d| d.id), None);
    assert!(data.select("1:2:129", None).is_err());
    // another mouse at another address is not the one that was picked
    assert_eq!(data.device("1:6:129").map(|d| d.id), None);
    // and neither is an id nothing ever had
    assert_eq!(data.device("1:2").map(|d| d.id), None);
    assert_eq!(parse_id("1:2:129"), Some((1, 2, 0x81)));
}

/// a press on the box is matched to the one endpoint whose button went down at that moment, and
/// to none when two did
#[test]
fn a_press_is_found_on_the_one_endpoint_it_went_down_on() {
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.describe(0, 1, 3, &Descriptors::mouse(0x046d, 0xc08b));
    let down = |device: u16, t: u64| match device {
        2 => (1_500_000..1_600_000).contains(&t) || (2_510_000..2_600_000).contains(&t),
        _ => (2_500_000..2_600_000).contains(&t),
    };
    traffic.extend(ticks(1000, 1_000_000, 2_000).flat_map(|t| {
        [2, 3].map(|device| {
            let mut payload = mouse_payload(0, 0, 7);
            payload[0] = u8::from(down(device, t));
            (t, Packet::report(1, device, 0x81, &payload))
        })
    }));
    let mouse = |device| Key {
        bus: 1,
        device,
        endpoint: 0x81,
    };
    // neither is picked; their layouts are the ones Windows gives for every mouse
    let windows = |data: &mut Data, record: &Record<'_>| {
        data.ingest(record);
        for device in [2, 3] {
            if let Decoder::Descriptor(layout) = crate::testkit::traffic::mouse_decoder() {
                data.remember_windows_layout(mouse(device), layout);
            }
        }
    };
    let mut data = Data::new(true);
    pcap::read_pcap(traffic.pcap().bytes(), |record| windows(&mut data, &record)).unwrap();
    // after the first press only device 2 had gone down, and nothing went down near 1.2 s
    let mut first = Data::new(true);
    pcap::read_pcap(traffic.pcap().bytes(), |record| {
        if let Record::Packet {
            event: Some(Event::Report(r)),
            ..
        } = &record
            && r.t_us >= 2_000_000
        {
            return;
        }
        windows(&mut first, &record)
    })
    .unwrap();
    assert_eq!(first.pressed_at(1_510_000, 50_000), Some(mouse(2)));
    assert_eq!(first.pressed_at(1_200_000, 50_000), None);
    // later both went down 10 ms apart, so a press then can't say which
    assert_eq!(data.pressed_at(2_505_000, 50_000), None);
}
