use motion_bench::capture::hid::Layout;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ConnectedLayout {
    pub layout: Layout,
    pub path: PathBuf,
}

#[cfg(windows)]
pub fn read(vid: u16, pid: u16, interface: u8) -> Result<ConnectedLayout, String> {
    let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
    let (layout, bytes) = layout(&api, vid, pid, interface)?;
    let folder = crate::storage::folders::recordings().join("descriptors");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    use sha2::{Digest, Sha256};
    // the same layout always gets the same name, whichever build wrote it
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let path = folder.join(format!(
        "{vid:04x}-{pid:04x}-if{interface}-{}.bin",
        &hash[..16]
    ));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    std::fs::write(path.with_extension("json"), serde_json::to_vec_pretty(&serde_json::json!({
        "source":"Windows HID preparsed data reconstructed by hidapi 2.6.7",
        "original_usb_descriptor":false,"vid":vid,"pid":pid,"interface":interface,"layout":layout
    })).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(ConnectedLayout { layout, path })
}

/// the layout of every mouse among `keys`, read from Windows without keeping a backup, so a
/// button going down on any of them can be seen
#[cfg(windows)]
pub fn layouts(keys: &[(u16, u16, u8)]) -> HashMap<(u16, u16, u8), Layout> {
    let Ok(api) = hidapi::HidApi::new() else {
        return HashMap::new();
    };
    keys.iter()
        .filter_map(|&(vid, pid, interface)| {
            let (layout, _) = layout(&api, vid, pid, interface).ok()?;
            Some(((vid, pid, interface), layout))
        })
        .collect()
}

/// the layout Windows rebuilds for a mouse collection, and the descriptor bytes it came from
#[cfg(windows)]
fn layout(
    api: &hidapi::HidApi,
    vid: u16,
    pid: u16,
    interface: u8,
) -> Result<(Layout, Vec<u8>), String> {
    let devices = api
        .device_list()
        .filter(|device| {
            device.vendor_id() == vid
                && device.product_id() == pid
                && device.interface_number() == i32::from(interface)
                && device.usage_page() == 1
                && device.usage() == 2
        })
        .collect::<Vec<_>>();
    if devices.is_empty() {
        return Err(format!(
            "Windows has no mouse HID collection for {vid:04x}:{pid:04x}, interface {interface}"
        ));
    }
    let mut layouts = Vec::new();
    for device in devices {
        let handle = device
            .open_device(api)
            .map_err(|e| format!("Windows could not open the mouse layout: {e}"))?;
        let mut bytes = vec![0; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
        let length = handle
            .get_report_descriptor(&mut bytes)
            .map_err(|e| format!("Windows could not reconstruct the mouse layout: {e}"))?;
        bytes.truncate(length);
        let layout = Layout::parse(&bytes)
            .map_err(|e| format!("Windows mouse layout is unsupported: {e}"))?;
        layouts.push((layout, bytes));
    }
    let (layout, bytes) = layouts.first().ok_or("Windows returned no mouse layout")?;
    if layouts.iter().any(|(other, _)| other != layout) {
        return Err("several matching mouse collections have different layouts, so capture the report descriptor or import the right one".into());
    }
    Ok((layout.clone(), bytes.clone()))
}

#[cfg(not(windows))]
pub fn read(_vid: u16, _pid: u16, _interface: u8) -> Result<ConnectedLayout, String> {
    Err("looking up the connected mouse layout needs Windows".into())
}

#[cfg(not(windows))]
pub fn layouts(_keys: &[(u16, u16, u8)]) -> HashMap<(u16, u16, u8), Layout> {
    HashMap::new()
}

/// what Windows calls each connected HID device, by the ids a capture knows it by. a device Windows
/// did not name, or one it did not give an interface for, is left out: the interface number is what
/// tells two collections of the same device apart, so a name without one could land on the wrong row
#[cfg(windows)]
pub fn names() -> HashMap<(u16, u16, u8), String> {
    let Ok(api) = hidapi::HidApi::new() else {
        return HashMap::new();
    };
    api.device_list()
        .filter_map(|device| {
            let interface = u8::try_from(device.interface_number()).ok()?;
            let name = device.product_string()?.trim();
            (!name.is_empty()).then(|| {
                (
                    (device.vendor_id(), device.product_id(), interface),
                    name.to_string(),
                )
            })
        })
        .collect()
}

#[cfg(not(windows))]
pub fn names() -> HashMap<(u16, u16, u8), String> {
    HashMap::new()
}
