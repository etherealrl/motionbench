use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const LINKTYPE_USBPCAP: u32 = 249;
pub const MAX_REPORT: usize = 65_535;

const PCAP_MAGIC_US: u32 = 0xa1b2_c3d4;
const HEADER_LEN: usize = 27;
const TRANSFER_INTERRUPT: u8 = 1;
const TRANSFER_CONTROL: u8 = 2;
const DESCRIPTOR_DEVICE: u8 = 1;
const DESCRIPTOR_CONFIGURATION: u8 = 2;
const DESCRIPTOR_INTERFACE: u8 = 4;
const DESCRIPTOR_ENDPOINT: u8 = 5;
const HID_CLASS: u8 = 3;
const INFO_PDO_TO_FDO: u8 = 0x01;
const ENDPOINT_IN: u8 = 0x80;
/// the urb a driver sends when it stops a pipe for good, which is what unplugging a device
/// looks like when its pending reads are cancelled instead of failing as gone
const URB_FUNCTION_ABORT_PIPE: u16 = 0x0002;
const USBD_STATUS_DEVICE_GONE: u32 = 0xC000_7000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub t_us: u64,
    pub bus: u16,
    pub device: u16,
    pub endpoint: u8,
    pub len: usize,
    pub data: Vec<u8>,
}

impl Report {
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

/// when USBPcap took the record, in UTC microseconds, from its pcap header
pub fn record_time(header: &[u8; 16]) -> u64 {
    let seconds = u32::from_le_bytes(header[..4].try_into().unwrap()) as u64;
    let micros = u32::from_le_bytes(header[4..8].try_into().unwrap()) as u64;
    seconds * 1_000_000 + micros
}

/// the layout the test traffic is built with: buttons, then little-endian int16 X and Y
#[cfg(any(test, feature = "testkit"))]
pub fn motion(payload: &[u8]) -> Option<(i16, i16)> {
    if payload.len() < 5 {
        return None;
    }
    Some((
        i16::from_le_bytes([payload[1], payload[2]]),
        i16::from_le_bytes([payload[3], payload[4]]),
    ))
}

/// one interrupt-IN endpoint as its configuration descriptor advertises it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub interface: u8,
    pub address: u8,
    pub interval: u8,
    pub max_packet: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

impl Endpoint {
    /// the hid boot protocol names the device outright, everything else is just a class
    pub fn kind(&self) -> &'static str {
        match (self.class, self.subclass, self.protocol) {
            (HID_CLASS, 1, 2) => "mouse",
            (HID_CLASS, 1, 1) => "keyboard",
            (HID_CLASS, _, _) => "hid",
            _ => "other",
        }
    }

    /// bInterval is milliseconds on a full speed bus and 2^(n-1) frames of 125 µs on high speed,
    /// and nothing in the descriptor says which bus the device sits on. both, in that order
    pub fn period_us(&self) -> (u32, u32) {
        let n = self.interval.clamp(1, 16) as u32;
        (n * 1000, 125 << (n - 1))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// one endpoint's pipe was torn down after the device went away. some unplugs show only
    /// this, with no device gone status
    Aborted {
        bus: u16,
        address: u16,
        endpoint: u8,
    },
    Disconnected {
        bus: u16,
        address: u16,
    },
    Hid {
        bus: u16,
        address: u16,
        interface: u8,
        descriptor: Vec<u8>,
    },
    Report(Report),
    /// vid:pid for one address, from the descriptors usbpcapcmd injects at capture start
    Device {
        bus: u16,
        address: u16,
        vid: u16,
        pid: u16,
    },
    /// every interrupt-IN endpoint one address advertises, from the same injected descriptors
    Config {
        bus: u16,
        address: u16,
        endpoints: Vec<Endpoint>,
    },
}

/// walks a configuration descriptor and keeps the interrupt-IN endpoints with the interface they
/// belong to. the first alternate setting wins, later ones for the same address are skipped
fn configuration(payload: &[u8]) -> Vec<Endpoint> {
    let mut out: Vec<Endpoint> = Vec::new();
    let mut interface = (0u8, 0u8, 0u8, 0u8);
    let mut i = 0;
    while i + 2 <= payload.len() {
        let len = payload[i] as usize;
        if len < 2 {
            break;
        }
        let d = &payload[i..(i + len).min(payload.len())];
        match d[1] {
            DESCRIPTOR_INTERFACE if d.len() >= 8 => interface = (d[5], d[6], d[7], d[2]),
            DESCRIPTOR_ENDPOINT
                if d.len() >= 7
                    && d[3] & 0x03 == 0x03
                    && d[2] & ENDPOINT_IN != 0
                    && !out.iter().any(|e| e.address == d[2]) =>
            {
                out.push(Endpoint {
                    interface: interface.3,
                    address: d[2],
                    interval: d[6],
                    max_packet: u16::from_le_bytes([d[4], d[5]]),
                    class: interface.0,
                    subclass: interface.1,
                    protocol: interface.2,
                });
            }
            _ => {}
        }
        i += len;
    }
    out
}

fn decode(pkt: &[u8], t_us: u64) -> Option<Event> {
    if pkt.len() < HEADER_LEN {
        return None;
    }
    let header_len = u16::from_le_bytes([pkt[0], pkt[1]]) as usize;
    let status = u32::from_le_bytes([pkt[10], pkt[11], pkt[12], pkt[13]]);
    let function = u16::from_le_bytes([pkt[14], pkt[15]]);
    let info = pkt[16];
    let bus = u16::from_le_bytes([pkt[17], pkt[18]]);
    let device = u16::from_le_bytes([pkt[19], pkt[20]]);
    let endpoint = pkt[21];
    let transfer = pkt[22];
    let data_len = u32::from_le_bytes([pkt[23], pkt[24], pkt[25], pkt[26]]) as usize;

    if header_len < HEADER_LEN || header_len > pkt.len() {
        return None;
    }
    let payload = &pkt[header_len..header_len + data_len.min(pkt.len() - header_len)];
    if status == USBD_STATUS_DEVICE_GONE {
        return Some(Event::Disconnected {
            bus,
            address: device,
        });
    }
    // an orderly abort completes fine, the one an unplug causes fails because the pipe is
    // already gone
    if function == URB_FUNCTION_ABORT_PIPE && info & INFO_PDO_TO_FDO != 0 && status != 0 {
        return Some(Event::Aborted {
            bus,
            address: device,
            endpoint,
        });
    }

    if transfer == TRANSFER_CONTROL {
        // bLength 18, bDescriptorType 1, then vid at 8 and pid at 10
        if payload.len() >= 18 && payload[0] == 18 && payload[1] == DESCRIPTOR_DEVICE {
            return Some(Event::Device {
                bus,
                address: device,
                vid: u16::from_le_bytes([payload[8], payload[9]]),
                pid: u16::from_le_bytes([payload[10], payload[11]]),
            });
        }
        if payload.len() >= 9 && payload[0] == 9 && payload[1] == DESCRIPTOR_CONFIGURATION {
            let endpoints = configuration(payload);
            if !endpoints.is_empty() {
                return Some(Event::Config {
                    bus,
                    address: device,
                    endpoints,
                });
            }
        }
        return None;
    }

    // a cancelled urb completes too, at unplug or when the driver tears down, and it is not a
    // report. USBD_STATUS_SUCCESS is the only status that carries data the device sent
    if transfer != TRANSFER_INTERRUPT
        || endpoint & ENDPOINT_IN == 0
        || info & INFO_PDO_TO_FDO == 0
        || status != 0
    {
        return None;
    }

    if data_len > payload.len() || data_len > MAX_REPORT || data_len == 0 {
        return None;
    }
    let len = data_len;
    let data = payload[..len].to_vec();

    Some(Event::Report(Report {
        t_us,
        bus,
        device,
        endpoint,
        len,
        data,
    }))
}

/// the raw bytes travel alongside the decoded event so a recorder can write records verbatim
pub enum Record<'a> {
    Header(&'a [u8; 24]),
    Packet {
        header: &'a [u8; 16],
        packet: &'a [u8],
        bus: u16,
        device: u16,
        event: Option<Event>,
    },
}

pub fn read_pcap<R: Read>(mut r: R, mut on_record: impl FnMut(Record<'_>)) -> io::Result<()> {
    let mut global = [0u8; 24];
    r.read_exact(&mut global)?;

    let magic = u32::from_le_bytes([global[0], global[1], global[2], global[3]]);
    if magic != PCAP_MAGIC_US {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("not a little-endian microsecond pcap (magic {magic:08x})"),
        ));
    }
    let linktype = u32::from_le_bytes([global[20], global[21], global[22], global[23]]);
    if linktype != LINKTYPE_USBPCAP {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("linktype {linktype}, expected {LINKTYPE_USBPCAP}"),
        ));
    }
    let snaplen = u32::from_le_bytes([global[16], global[17], global[18], global[19]]) as usize;
    if !(1..=1 << 20).contains(&snaplen) || global[4..8] != [2, 0, 4, 0] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid pcap version or snaplen",
        ));
    }
    on_record(Record::Header(&global));

    let mut buf = vec![0u8; snaplen];
    let mut record = [0u8; 16];
    let mut hid_requests = std::collections::BTreeMap::new();
    loop {
        loop {
            match r.read(&mut record[..1]) {
                Ok(0) => return Ok(()),
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        r.read_exact(&mut record[1..])?;
        let secs = u32::from_le_bytes([record[0], record[1], record[2], record[3]]) as u64;
        let usecs = u32::from_le_bytes([record[4], record[5], record[6], record[7]]) as u64;
        let incl = u32::from_le_bytes([record[8], record[9], record[10], record[11]]) as usize;
        let original = u32::from_le_bytes(record[12..16].try_into().unwrap()) as usize;
        if incl > buf.len() || incl > original || usecs >= 1_000_000 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a record of {incl} bytes doesn't fit this capture"),
            ));
        }
        r.read_exact(&mut buf[..incl])?;
        // USBPcap cuts a transfer longer than the snaplen short. only another device's bulk
        // traffic gets that big, like a USB drive on the same hub, so it's skipped and the capture
        // goes on
        if incl < original {
            continue;
        }
        let packet = &buf[..incl];
        if packet.len() < HEADER_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "short USBPcap header",
            ));
        }
        let header_len = u16::from_le_bytes(packet[..2].try_into().unwrap()) as usize;
        let data_len = u32::from_le_bytes(packet[23..27].try_into().unwrap()) as usize;
        if header_len < HEADER_LEN || header_len > incl || data_len > incl - header_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated USBPcap payload",
            ));
        }
        let bus = u16::from_le_bytes([packet[17], packet[18]]);
        let device = u16::from_le_bytes([packet[19], packet[20]]);
        let irp = u64::from_le_bytes(packet[2..10].try_into().unwrap());
        let payload = &packet[header_len..header_len + data_len];
        let mut event = decode(packet, secs * 1_000_000 + usecs);
        if packet[22] == TRANSFER_CONTROL && header_len > HEADER_LEN {
            if packet[27] == 0 && payload.len() >= 8 && payload[1] == 6 && payload[3] == 0x22 {
                if hid_requests.len() >= 4096 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many unfinished HID descriptor requests",
                    ));
                }
                hid_requests.insert((bus, device, irp), payload[4]);
            } else if packet[16] & 1 != 0
                && !payload.is_empty()
                && let Some(interface) = hid_requests.remove(&(bus, device, irp))
            {
                event = Some(Event::Hid {
                    bus,
                    address: device,
                    interface,
                    descriptor: payload.to_vec(),
                });
            }
        }
        on_record(Record::Packet {
            header: &record,
            packet,
            bus: u16::from_le_bytes([packet[17], packet[18]]),
            device: u16::from_le_bytes([packet[19], packet[20]]),
            event,
        });
    }
}

/// USBPcap's buffer for a reader that falls behind. its default 1 MiB lasts about 2.7 s of a busy
/// root hub's traffic, this lasts about eight times that
pub const BUFFER_BYTES: u32 = 8 << 20;

pub fn usbpcapcmd() -> Option<PathBuf> {
    if cfg!(feature = "e2e") {
        return None;
    }
    ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|dir| Path::new(&dir).join("USBPcap").join("USBPcapCMD.exe"))
        .find(|p| p.is_file())
}

/// one control device per root hub, the mouse sits behind exactly one of them. a USBPcapCMD that
/// does not answer within 5 s is stopped, startup goes on without it
pub fn roots(exe: &Path) -> Result<Vec<String>, String> {
    let mut command = Command::new(exe);
    command.arg("--extcap-interfaces");
    let out = output_within(command, Duration::from_secs(5))?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| line.split_once("{value=")?.1.split_once('}'))
        .map(|(value, _)| value.to_string())
        .collect())
}

/// what `command` writes, or why there is nothing: it could not start or did not finish within
/// `bound`, and was stopped
fn output_within(mut command: Command, bound: Duration) -> Result<Vec<u8>, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("USBPcap could not list its root hubs: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("USBPcap gave no output")?;
    // read while it runs, so a full pipe never stops it
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let start = Instant::now();
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if start.elapsed() > bound {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "USBPcap did not list its root hubs within {} s",
                bound.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    reader
        .join()
        .map_err(|_| "USBPcap's output could not be read".into())
}

#[cfg(test)]
mod tests;
