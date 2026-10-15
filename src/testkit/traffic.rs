//! USBPcap streams built byte for byte, so tests go through `capture::read_pcap` and the real
//! session and recorder paths instead of handing them made up records

use crate::capture::pcap::LINKTYPE_USBPCAP;
use std::path::{Path, PathBuf};

const HEADER_LEN: usize = 27;
pub const INTERRUPT: u8 = 1;
pub const CONTROL: u8 = 2;
const BULK_OR_INTERRUPT: u16 = 9;
const CONTROL_TRANSFER: u16 = 8;
const GET_DESCRIPTOR: u16 = 11;
const URB_FUNCTION_ABORT_PIPE: u16 = 2;
const TRANSFER_IRP_INFO: u8 = 0xFE;
const USBD_STATUS_CANCELED: u32 = 0xC001_0000;
const USBD_STATUS_INVALID_PARAMETER: u32 = 0x8000_0300;

/// one USBPcap packet. the constructors copy the field values real captures use, the
/// fidelity tests below hold them to the checked in fixtures
#[derive(Clone, Debug)]
pub struct Packet {
    pub irp: u64,
    pub status: u32,
    pub function: u16,
    pub info: u8,
    pub bus: u16,
    pub device: u16,
    pub endpoint: u8,
    pub transfer: u8,
    /// control transfers carry a stage byte after the 27 byte header
    pub stage: Option<u8>,
    pub payload: Vec<u8>,
}

impl Packet {
    /// a completed interrupt-IN transfer, which is what a report is
    pub fn report(bus: u16, device: u16, endpoint: u8, payload: &[u8]) -> Self {
        Packet {
            irp: 0,
            status: 0,
            function: BULK_OR_INTERRUPT,
            info: 1,
            bus,
            device,
            endpoint,
            transfer: INTERRUPT,
            stage: None,
            payload: payload.to_vec(),
        }
    }

    /// the setup stage of a request to the default pipe, as usbpcapcmd injects it
    pub fn setup(bus: u16, device: u16, request: [u8; 8]) -> Self {
        Packet {
            function: GET_DESCRIPTOR,
            info: 0,
            endpoint: 0x80,
            transfer: CONTROL,
            stage: Some(0),
            ..Packet::report(bus, device, 0, &request)
        }
    }

    /// the completion of a control request, with whatever the device answered
    pub fn control_in(bus: u16, device: u16, payload: &[u8]) -> Self {
        Packet {
            function: CONTROL_TRANSFER,
            info: 1,
            endpoint: 0x80,
            transfer: CONTROL,
            stage: Some(3),
            ..Packet::report(bus, device, 0, payload)
        }
    }

    /// the status a pending urb completes with when its device goes away
    pub fn disconnect(bus: u16, device: u16, endpoint: u8) -> Self {
        Packet {
            status: 0xC000_7000,
            ..Packet::report(bus, device, endpoint, &[])
        }
    }

    /// what usbpcapcmd wrote when a mouse was unplugged on the maintainer's machine: its
    /// pending reads cancelled, then its pipe aborted, the abort failing, with no device gone
    /// status at all
    pub fn unplug(bus: u16, device: u16, endpoint: u8) -> [Self; 4] {
        let cancelled = Packet {
            status: USBD_STATUS_CANCELED,
            ..Packet::report(bus, device, endpoint, &[])
        };
        let (submitted, failed) = Packet::abort(bus, device, endpoint);
        [
            cancelled.clone(),
            cancelled,
            submitted,
            Packet {
                status: USBD_STATUS_INVALID_PARAMETER,
                ..failed
            },
        ]
    }

    /// an abort of one pipe as a driver sends it, submitted and then completed fine
    pub fn abort(bus: u16, device: u16, endpoint: u8) -> (Self, Self) {
        let submitted = Packet {
            function: URB_FUNCTION_ABORT_PIPE,
            info: 0,
            transfer: TRANSFER_IRP_INFO,
            ..Packet::report(bus, device, endpoint, &[])
        };
        let completed = Packet {
            info: 1,
            ..submitted.clone()
        };
        (submitted, completed)
    }

    pub fn bytes(&self) -> Vec<u8> {
        let header_len = HEADER_LEN + usize::from(self.stage.is_some());
        let mut p = Vec::with_capacity(header_len + self.payload.len());
        p.extend_from_slice(&(header_len as u16).to_le_bytes());
        p.extend_from_slice(&self.irp.to_le_bytes());
        p.extend_from_slice(&self.status.to_le_bytes());
        p.extend_from_slice(&self.function.to_le_bytes());
        p.push(self.info);
        p.extend_from_slice(&self.bus.to_le_bytes());
        p.extend_from_slice(&self.device.to_le_bytes());
        p.push(self.endpoint);
        p.push(self.transfer);
        p.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        p.extend(self.stage);
        p.extend_from_slice(&self.payload);
        p
    }
}

pub struct Pcap(Vec<u8>);

impl Default for Pcap {
    fn default() -> Self {
        let mut out = Vec::new();
        out.extend_from_slice(&0xa1b2_c3d4u32.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&65535u32.to_le_bytes());
        out.extend_from_slice(&LINKTYPE_USBPCAP.to_le_bytes());
        Pcap(out)
    }
}

impl Pcap {
    pub fn record(&mut self, t_us: u64, packet: &[u8]) -> &mut Self {
        self.0
            .extend_from_slice(&((t_us / 1_000_000) as u32).to_le_bytes());
        self.0
            .extend_from_slice(&((t_us % 1_000_000) as u32).to_le_bytes());
        self.0
            .extend_from_slice(&(packet.len() as u32).to_le_bytes());
        self.0
            .extend_from_slice(&(packet.len() as u32).to_le_bytes());
        self.0.extend_from_slice(packet);
        self
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

/// a capture's packets in arrival order. sorting is stable, so packets stamped with the same
/// microsecond keep the order they were added in
#[derive(Default, Clone)]
pub struct Traffic(pub Vec<(u64, Packet)>);

impl Traffic {
    pub fn add(&mut self, t_us: u64, packet: Packet) -> &mut Self {
        self.0.push((t_us, packet));
        self
    }

    pub fn extend(&mut self, packets: impl IntoIterator<Item = (u64, Packet)>) -> &mut Self {
        self.0.extend(packets);
        self
    }

    pub fn sorted(mut self) -> Self {
        self.0.sort_by_key(|(t, _)| *t);
        self
    }

    pub fn pcap(&self) -> Pcap {
        let mut pcap = Pcap::default();
        for (t, packet) in &self.0 {
            pcap.record(*t, &packet.bytes());
        }
        pcap
    }

    /// the device and configuration descriptors usbpcapcmd injects for one address when a
    /// capture starts, all at the same timestamp
    pub fn describe(&mut self, t_us: u64, bus: u16, device: u16, descriptors: &Descriptors) {
        let device_request = [0x80, 6, 0, 1, 0, 0, 18, 0];
        let config = descriptors.config();
        let length = (config.len() as u16).to_le_bytes();
        let config_request = [0x80, 6, 0, 2, 0, 0, length[0], length[1]];
        self.add(t_us, Packet::setup(bus, device, device_request))
            .add(t_us, Packet::control_in(bus, device, &descriptors.device()))
            .add(t_us, Packet::setup(bus, device, config_request))
            .add(t_us, Packet::control_in(bus, device, &config));
    }

    /// a HID report descriptor request and its answer, paired by irp like the real ones
    pub fn hid_descriptor(&mut self, t_us: u64, bus: u16, device: u16, interface: u8, irp: u64) {
        let layout = [
            0x05, 1, 0x09, 2, 0xa1, 1, 0x05, 1, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7f,
            0x75, 8, 0x95, 2, 0x81, 6, 0xc0,
        ];
        let request = [0x81, 6, 0, 0x22, interface, 0, layout.len() as u8, 0];
        let setup = Packet {
            irp,
            ..Packet::setup(bus, device, request)
        };
        let answer = Packet {
            irp,
            ..Packet::control_in(bus, device, &layout)
        };
        self.add(t_us, setup).add(t_us, answer);
    }
}

pub struct Descriptors {
    pub vid: u16,
    pub pid: u16,
    pub interfaces: Vec<Interface>,
}

pub struct Interface {
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    /// (endpoint address, bInterval, max packet) for each interrupt-IN endpoint
    pub endpoints: Vec<(u8, u8, u16)>,
}

impl Interface {
    pub fn hid(subclass: u8, protocol: u8, endpoints: &[(u8, u8, u16)]) -> Self {
        Interface {
            class: 3,
            subclass,
            protocol,
            endpoints: endpoints.to_vec(),
        }
    }
}

impl Descriptors {
    pub fn mouse(vid: u16, pid: u16) -> Self {
        Descriptors {
            vid,
            pid,
            interfaces: vec![Interface::hid(1, 2, &[(0x81, 1, 8)])],
        }
    }

    pub fn keyboard(vid: u16, pid: u16) -> Self {
        Descriptors {
            vid,
            pid,
            interfaces: vec![Interface::hid(1, 1, &[(0x81, 8, 8)])],
        }
    }

    fn device(&self) -> Vec<u8> {
        let mut d = vec![18, 1, 0, 2, 0, 0, 0, 64];
        d.extend_from_slice(&self.vid.to_le_bytes());
        d.extend_from_slice(&self.pid.to_le_bytes());
        d.extend_from_slice(&[0, 1, 1, 2, 0, 1]);
        d
    }

    fn config(&self) -> Vec<u8> {
        let mut c = vec![9, 2, 0, 0, self.interfaces.len() as u8, 1, 0, 0xa0, 50];
        for (number, interface) in self.interfaces.iter().enumerate() {
            c.extend_from_slice(&[
                9,
                4,
                number as u8,
                0,
                interface.endpoints.len() as u8,
                interface.class,
                interface.subclass,
                interface.protocol,
                0,
            ]);
            for (address, interval, max_packet) in &interface.endpoints {
                let size = max_packet.to_le_bytes();
                c.extend_from_slice(&[7, 5, *address, 3, size[0], size[1], *interval]);
            }
        }
        let total = (c.len() as u16).to_le_bytes();
        c[2..4].copy_from_slice(&total);
        c
    }
}

/// report times for a device polled at `rate_hz`, in integer microseconds so no rounding error
/// builds up over a long train
pub fn ticks(rate_hz: u64, start_us: u64, count: u64) -> impl Iterator<Item = u64> {
    (0..count).map(move |i| start_us + i * 1_000_000 / rate_hz)
}

/// the report descriptor of the layout `mouse_payload` builds: eight buttons, then signed 16 bit
/// relative x and y, the way many mice report
pub const MOUSE_REPORT: [u8; 41] = [
    0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x05, 0x09, 0x19, 0x01, 0x29, 0x08, 0x15, 0x00, 0x25, 0x01,
    0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x16, 0x01, 0x80, 0x26,
    0xff, 0x7f, 0x75, 0x10, 0x95, 0x02, 0x81, 0x06, 0xc0,
];

/// the decoder a recording of `mouse_payload` reports saves, its layout read from `MOUSE_REPORT`
pub fn mouse_decoder() -> crate::capture::hid::Decoder {
    crate::capture::hid::Decoder::Descriptor(
        crate::capture::hid::Layout::parse(&MOUSE_REPORT).unwrap(),
    )
}

/// buttons, then little-endian int16 x and y, padded to `len`
pub fn mouse_payload(dx: i16, dy: i16, len: usize) -> Vec<u8> {
    let mut p = vec![0u8; len.max(5)];
    p[1..3].copy_from_slice(&dx.to_le_bytes());
    p[3..5].copy_from_slice(&dy.to_le_bytes());
    p
}

pub use crate::capture::pcap::record_time;

pub fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// a folder unique to one test, removed again even when the test panics
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "motion-bench-test-{name}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests;
