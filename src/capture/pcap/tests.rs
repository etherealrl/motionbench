use super::*;
use crate::testkit::traffic::{Packet, Pcap};
use std::io::BufReader;

fn pcap(records: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut pcap = Pcap::default();
    for (t_us, packet) in records {
        pcap.record(*t_us, packet);
    }
    pcap.bytes().to_vec()
}

fn events(stream: &[u8]) -> io::Result<Vec<Event>> {
    let mut got = Vec::new();
    read_pcap(stream, |record| {
        if let Record::Packet {
            event: Some(event), ..
        } = record
        {
            got.push(event)
        }
    })?;
    Ok(got)
}

fn reports(stream: &[u8]) -> io::Result<Vec<Report>> {
    Ok(events(stream)?
        .into_iter()
        .filter_map(|e| match e {
            Event::Report(r) => Some(r),
            _ => None,
        })
        .collect())
}

fn packet(info: u8, endpoint: u8, transfer: u8, payload: &[u8]) -> Vec<u8> {
    Packet {
        info,
        transfer,
        ..Packet::report(1, 31, endpoint, payload)
    }
    .bytes()
}

#[test]
fn partial_record_header_is_not_clean_eof() {
    let mut stream = pcap(&[]);
    stream.extend_from_slice(&[1, 2, 3]);
    assert_eq!(
        read_pcap(&stream[..], |_| {}).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn large_report_is_not_truncated() {
    let payload = vec![0x5a; 1024];
    let got = reports(&pcap(&[(0, packet(1, 0x81, TRANSFER_INTERRUPT, &payload))])).unwrap();
    assert_eq!(got[0].bytes(), payload);
}

#[test]
fn invalid_payload_length_is_rejected() {
    let mut p = packet(1, 0x81, TRANSFER_INTERRUPT, &[1]);
    p[23..27].copy_from_slice(&10u32.to_le_bytes());
    assert!(reports(&pcap(&[(0, p)])).is_err());
}

#[test]
fn keeps_only_interrupt_in_completions() {
    let stream = pcap(&[
        (1_000_000, packet(0, 0x81, TRANSFER_INTERRUPT, &[1, 2, 3])),
        (1_001_000, packet(1, 0x81, TRANSFER_INTERRUPT, &[9, 8, 7])),
        (1_002_000, packet(1, 0x01, TRANSFER_INTERRUPT, &[4, 4])),
        (1_003_000, packet(1, 0x81, 2, &[5, 5])),
    ]);

    let got = reports(&stream).unwrap();

    assert_eq!(got.len(), 1);
    assert_eq!(got[0].t_us, 1_001_000);
    assert_eq!(got[0].device, 31);
    assert_eq!(got[0].bytes(), &[9, 8, 7]);
}

#[test]
fn motion_reads_the_wire_to_csv_layout() {
    assert_eq!(
        motion(&[0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00]),
        Some((-1, -1))
    );
    assert_eq!(
        motion(&[0x01, 0x0c, 0x00, 0xf4, 0xff, 0x00]),
        Some((12, -12))
    );
    assert_eq!(motion(&[0x00, 0x01, 0x02, 0x03]), None);
}

#[test]
fn a_cancelled_urb_is_not_a_report() {
    let mut p = packet(1, 0x81, TRANSFER_INTERRUPT, &[]);
    p[10..14].copy_from_slice(&0xC001_0000u32.to_le_bytes());

    let got = reports(&pcap(&[(0, p)])).unwrap();

    assert!(got.is_empty());
}

#[test]
fn a_64_byte_report_survives_whole() {
    let payload = [0x5au8; 64];

    let got = reports(&pcap(&[(0, packet(1, 0x81, TRANSFER_INTERRUPT, &payload))])).unwrap();

    assert_eq!(got[0].bytes(), &payload);
}

#[test]
fn header_len_moves_the_payload() {
    let mut p = packet(1, 0x81, TRANSFER_INTERRUPT, &[0xaa, 0xbb]);
    p[0..2].copy_from_slice(&28u16.to_le_bytes());
    p.insert(HEADER_LEN, 0x00);

    let got = reports(&pcap(&[(0, p)])).unwrap();

    assert_eq!(got[0].bytes(), &[0xaa, 0xbb]);
}

#[test]
fn truncated_stream_is_an_error_not_a_report() {
    let mut stream = pcap(&[(0, packet(1, 0x81, TRANSFER_INTERRUPT, &[1, 2, 3]))]);
    stream.truncate(stream.len() - 2);

    let err = reports(&stream).unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

/// 400 real reports off a 1000hz mouse, sliced out of a usbpcap capture
#[test]
fn parses_a_real_capture() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    );
    let file = std::fs::File::open(path).unwrap();

    let mut got = Vec::new();
    read_pcap(BufReader::new(file), |record| {
        if let Record::Packet {
            event: Some(Event::Report(r)),
            ..
        } = record
        {
            got.push(r)
        }
    })
    .unwrap();

    assert_eq!(got.len(), 400);
    assert!(
        got.iter()
            .all(|r| r.bus == 1 && r.device == 31 && r.endpoint == 0x81 && r.len == 7)
    );

    let mut gaps: Vec<u64> = got.windows(2).map(|w| w[1].t_us - w[0].t_us).collect();
    gaps.sort_unstable();
    assert_eq!(gaps[gaps.len() / 2], 1000);
}

#[test]
fn rejects_a_non_usbpcap_linktype() {
    let mut stream = pcap(&[]);
    stream[20..24].copy_from_slice(&1u32.to_le_bytes());

    let err = read_pcap(&stream[..], |_| {}).unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

/// device and configuration descriptors for dev 31, sliced out of a real capture. no input data
#[test]
fn reads_the_advertised_endpoint_off_a_real_configuration_descriptor() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/descriptors.pcap"
    );
    let got = events(&std::fs::read(path).unwrap()).unwrap();

    let mouse = Endpoint {
        interface: 0,
        address: 0x81,
        interval: 1,
        max_packet: 7,
        class: 3,
        subclass: 1,
        protocol: 2,
    };
    assert!(got.contains(&Event::Device {
        bus: 1,
        address: 31,
        vid: 0x373b,
        pid: 0x11d9
    }));
    // three interfaces: the mouse, and two config/vendor hid interfaces polled at 4 ms
    let endpoints = got
        .iter()
        .find_map(|e| match e {
            Event::Config {
                address: 31,
                endpoints,
                ..
            } => Some(endpoints),
            _ => None,
        })
        .expect("a config event for dev 31");
    assert_eq!(endpoints[0], mouse);
    assert_eq!(endpoints.len(), 3);
    assert!(
        endpoints[1..]
            .iter()
            .all(|e| e.class == HID_CLASS && e.interval == 4)
    );
    assert_eq!(mouse.kind(), "mouse");
    assert_eq!(mouse.period_us(), (1000, 125));
}

#[test]
fn a_composite_device_keeps_one_endpoint_per_interface_and_nothing_else() {
    let mut config = vec![9u8, DESCRIPTOR_CONFIGURATION, 0, 0, 2, 1, 0, 0xa0, 50];
    config.extend_from_slice(&[9, DESCRIPTOR_INTERFACE, 0, 0, 1, 3, 1, 2, 0]);
    config.extend_from_slice(&[7, DESCRIPTOR_ENDPOINT, 0x81, 0x03, 8, 0, 1]);
    config.extend_from_slice(&[9, DESCRIPTOR_INTERFACE, 0, 1, 1, 3, 1, 2, 0]);
    config.extend_from_slice(&[7, DESCRIPTOR_ENDPOINT, 0x81, 0x03, 16, 0, 4]);
    config.extend_from_slice(&[9, DESCRIPTOR_INTERFACE, 1, 0, 2, 3, 1, 1, 0]);
    config.extend_from_slice(&[7, DESCRIPTOR_ENDPOINT, 0x82, 0x03, 64, 0, 8]);
    config.extend_from_slice(&[7, DESCRIPTOR_ENDPOINT, 0x02, 0x03, 64, 0, 8]);
    config.extend_from_slice(&[7, DESCRIPTOR_ENDPOINT, 0x83, 0x02, 0, 2, 0]);
    let total = config.len() as u16;
    config[2..4].copy_from_slice(&total.to_le_bytes());

    let p = Packet::control_in(1, 31, &config).bytes();
    let got = events(&pcap(&[(0, p)])).unwrap();

    let Event::Config { endpoints, .. } = &got[0] else {
        panic!("expected a config event");
    };
    assert_eq!(endpoints.len(), 2);
    assert_eq!(
        (
            endpoints[0].address,
            endpoints[0].interval,
            endpoints[0].kind()
        ),
        (0x81, 1, "mouse")
    );
    assert_eq!(
        (
            endpoints[1].address,
            endpoints[1].interval,
            endpoints[1].kind()
        ),
        (0x82, 8, "keyboard")
    );
    assert_eq!(endpoints[1].period_us(), (8000, 16_000));
}

#[test]
fn reads_vid_pid_from_an_injected_descriptor() {
    let mut descriptor = vec![18u8, DESCRIPTOR_DEVICE];
    descriptor.extend_from_slice(&[0; 6]);
    descriptor.extend_from_slice(&0x373bu16.to_le_bytes());
    descriptor.extend_from_slice(&0x11d9u16.to_le_bytes());
    descriptor.extend_from_slice(&[0; 6]);

    let p = Packet::control_in(1, 31, &descriptor).bytes();

    let got = events(&pcap(&[(0, p)])).unwrap();

    assert_eq!(
        got,
        vec![Event::Device {
            bus: 1,
            address: 31,
            vid: 0x373b,
            pid: 0x11d9,
        }]
    );
}

#[test]
fn unfinished_hid_descriptor_requests_are_capped() {
    let request = |irp| {
        Packet {
            irp,
            ..Packet::setup(1, 31, [0x81, 6, 0, 0x22, 0, 0, 64, 0])
        }
        .bytes()
    };
    let mut pcap = Pcap::default();
    for irp in 0..4096 {
        pcap.record(0, &request(irp));
    }
    assert!(read_pcap(pcap.bytes(), |_| {}).is_ok());
    pcap.record(0, &request(4096));
    let error = read_pcap(pcap.bytes(), |_| {}).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("too many unfinished"), "{error}");
}

/// a command that answers in time gives its output, one that does not is stopped at the bound
#[cfg(windows)]
#[test]
fn a_command_that_does_not_answer_in_time_is_stopped() {
    let mut quick = Command::new("cmd");
    quick.args(["/c", "echo {value=USBPcap1}"]);
    let out = output_within(quick, Duration::from_secs(5)).unwrap();
    assert!(String::from_utf8_lossy(&out).contains("USBPcap1"));
    let mut stuck = Command::new("ping");
    stuck.args(["-n", "30", "127.0.0.1"]);
    let start = Instant::now();
    let error = output_within(stuck, Duration::from_millis(300)).unwrap_err();
    assert!(error.contains("within"));
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_transfer_cut_at_the_snaplen_is_skipped_and_the_capture_goes_on() {
    let mut stream = pcap(&[]);
    let mut cut = packet(1, 0x02, 3, &[0; 64]);
    // the header still says how long the whole transfer was
    cut[23..27].copy_from_slice(&(100_000u32 - 27).to_le_bytes());
    stream.extend_from_slice(&0u32.to_le_bytes());
    stream.extend_from_slice(&0u32.to_le_bytes());
    stream.extend_from_slice(&(cut.len() as u32).to_le_bytes());
    stream.extend_from_slice(&100_000u32.to_le_bytes());
    stream.extend_from_slice(&cut);
    let mut mouse = Pcap::default();
    mouse.record(1, &packet(1, 0x81, TRANSFER_INTERRUPT, &[1, 2, 3]));
    stream.extend_from_slice(&mouse.bytes()[24..]);
    let got = reports(&stream).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].bytes(), [1, 2, 3]);
}
