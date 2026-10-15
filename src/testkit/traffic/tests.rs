use super::*;
use crate::capture::pcap::{Event, Record, read_pcap};

fn records(bytes: &[u8]) -> Vec<(u64, Vec<u8>, Option<Event>)> {
    let mut out = Vec::new();
    read_pcap(bytes, |record| {
        if let Record::Packet {
            header,
            packet,
            event,
            ..
        } = record
        {
            out.push((record_time(header), packet.to_vec(), event));
        }
    })
    .unwrap();
    out
}

fn field<const N: usize>(packet: &[u8], at: usize) -> [u8; N] {
    packet[at..at + N].try_into().unwrap()
}

#[test]
fn report_packets_match_a_real_capture_byte_for_byte() {
    let real = fixture("mouse-1000hz.pcap");
    let mut rebuilt = Pcap::default();
    for (t, packet, _) in records(&real) {
        let built = Packet {
            irp: u64::from_le_bytes(field(&packet, 2)),
            ..Packet::report(1, 31, 0x81, &packet[HEADER_LEN..])
        };
        rebuilt.record(t, &built.bytes());
    }
    assert_eq!(rebuilt.bytes(), &real[..]);
}

#[test]
fn descriptor_packets_match_the_injected_ones_byte_for_byte() {
    let real = fixture("descriptors.pcap");
    let mut checked = 0;
    for (_, packet, _) in records(&real) {
        let device = u16::from_le_bytes(field(&packet, 19));
        let payload = &packet[HEADER_LEN + 1..];
        let built = match (packet[21], packet[27]) {
            (0x80, 0) => Packet::setup(1, device, payload.try_into().unwrap()),
            (0x80, 3) => Packet::control_in(1, device, payload),
            _ => continue,
        };
        assert_eq!(built.bytes(), packet);
        checked += 1;
    }
    assert_eq!(checked, 8);
}

#[test]
fn built_descriptors_decode_like_the_real_ones() {
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 31, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.hid_descriptor(0, 1, 31, 0, 7);
    let events: Vec<Event> = records(traffic.pcap().bytes())
        .into_iter()
        .filter_map(|(_, _, event)| event)
        .collect();
    assert_eq!(
        events[0],
        Event::Device {
            bus: 1,
            address: 31,
            vid: 0x373b,
            pid: 0x11d9
        }
    );
    let Event::Config { endpoints, .. } = &events[1] else {
        panic!("expected a config event, got {:?}", events[1]);
    };
    assert_eq!(
        (
            endpoints[0].address,
            endpoints[0].interval,
            endpoints[0].kind()
        ),
        (0x81, 1, "mouse")
    );
    assert!(matches!(events[2], Event::Hid { interface: 0, .. }));
    assert_eq!(events.len(), 3);
}

#[test]
fn ticks_stay_on_the_integer_grid() {
    let times: Vec<u64> = ticks(8000, 5, 8001).collect();
    assert_eq!(times[1] - times[0], 125);
    assert_eq!(times[8000], 1_000_005);
    let times: Vec<u64> = ticks(3, 0, 4).collect();
    assert_eq!(times, [0, 333_333, 666_666, 1_000_000]);
}

#[test]
fn the_mouse_report_descriptor_reads_what_mouse_payload_builds() {
    let decoder = mouse_decoder();
    for (dx, dy) in [(0, 0), (1, -1), (-32767, 32767), (300, -12)] {
        let payload = mouse_payload(dx, dy, 7);
        assert_eq!(decoder.decode(&payload), Some((dx as i32, dy as i32)));
        assert_eq!(crate::capture::pcap::motion(&payload), Some((dx, dy)));
    }
    let crate::capture::hid::Decoder::Descriptor(layout) = decoder else {
        unreachable!()
    };
    let mut pressed = mouse_payload(0, 0, 7);
    pressed[0] = 1;
    assert_eq!(layout.left_button(&pressed), Some(true));
    assert_eq!(layout.left_button(&mouse_payload(0, 0, 7)), Some(false));
}
