use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum Decoder {
    #[default]
    Unavailable,
    Descriptor(Layout),
    WindowsDescriptor(Layout),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Axis {
    offset: usize,
    width: usize,
    signed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    reports: BTreeMap<u8, [Axis; 2]>,
    report_ids: bool,
    buttons: BTreeMap<u8, Axis>,
}

#[derive(Clone, Default)]
struct Global {
    page: u32,
    minimum: i32,
    size: usize,
    count: usize,
    id: u8,
}

impl Layout {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut global = Global::default();
        let mut stack = Vec::new();
        let mut usages = Vec::new();
        let mut offsets = BTreeMap::<u8, usize>::new();
        let mut axes = BTreeMap::<u8, [Option<Axis>; 2]>::new();
        let mut buttons = BTreeMap::new();
        let mut ids = false;
        let mut i = 0;
        let mut usage_min = None;
        while i < bytes.len() {
            let prefix = bytes[i];
            i += 1;
            if prefix == 0xfe {
                return Err("long HID items are unsupported".into());
            }
            let size = [0, 1, 2, 4][(prefix & 3) as usize];
            if i + size > bytes.len() {
                return Err("truncated HID item".into());
            }
            let mut raw = [0u8; 4];
            raw[..size].copy_from_slice(&bytes[i..i + size]);
            let value = u32::from_le_bytes(raw);
            let signed = if size == 0 {
                0
            } else {
                ((value << (32 - size * 8)) as i32) >> (32 - size * 8)
            };
            i += size;
            match ((prefix >> 2) & 3, prefix >> 4) {
                (1, 0) => global.page = value,
                (1, 1) => global.minimum = signed,
                (1, 7) => global.size = value as usize,
                (1, 8) => {
                    if value == 0 || value > 255 {
                        return Err("invalid report ID".into());
                    }
                    global.id = value as u8;
                    ids = true;
                }
                (1, 9) => global.count = value as usize,
                (1, 10) => stack.push(global.clone()),
                (1, 11) => global = stack.pop().ok_or("unbalanced HID globals")?,
                (2, 0) => usages.push(if size == 4 {
                    value
                } else {
                    global.page << 16 | value
                }),
                (2, 1) => usage_min = Some(value),
                (2, 2) => {
                    let first = usage_min.take().ok_or("usage maximum without minimum")?;
                    if value < first || value - first > 1024 {
                        return Err("invalid HID usage range".into());
                    }
                    usages.extend((first..=value).map(|v| global.page << 16 | v));
                }
                (0, 8) => {
                    if global.size == 0 || global.size > 32 || global.count > 4096 {
                        return Err("unsupported HID field size".into());
                    }
                    let offset = offsets.entry(global.id).or_default();
                    for field in 0..global.count {
                        let usage = usages
                            .get(field)
                            .copied()
                            .or_else(|| usages.last().copied())
                            .unwrap_or(0);
                        if value & 3 == 2 && usage == 0x90001 {
                            buttons.insert(
                                global.id,
                                Axis {
                                    offset: *offset + field * global.size,
                                    width: global.size,
                                    signed: false,
                                },
                            );
                        }
                        // relative variable axes only; absolute tablet coordinates are not mouse deltas
                        if value & 7 == 6 && matches!(usage, 0x10030 | 0x10031) {
                            let axis = Axis {
                                offset: *offset + field * global.size,
                                width: global.size,
                                signed: global.minimum < 0,
                            };
                            axes.entry(global.id).or_insert([None, None])
                                [(usage - 0x10030) as usize] = Some(axis);
                        }
                    }
                    *offset += global.count * global.size;
                    if *offset > 65_535 * 8 {
                        return Err("report exceeds capture limit".into());
                    }
                    usages.clear();
                    usage_min = None;
                }
                (0, _) => {
                    usages.clear();
                    usage_min = None;
                }
                _ => {}
            }
        }
        let reports = axes
            .into_iter()
            .filter_map(|(id, [x, y])| Some((id, [x?, y?])))
            .collect::<BTreeMap<_, _>>();
        if reports.is_empty() {
            return Err("descriptor has no relative X/Y report".into());
        }
        Ok(Self {
            reports,
            report_ids: ids,
            buttons,
        })
    }

    pub fn left_button(&self, payload: &[u8]) -> Option<bool> {
        let (id, bytes) = if self.report_ids {
            (*payload.first()?, payload.get(1..)?)
        } else {
            (0, payload)
        };
        let field = self.buttons.get(&id)?;
        if field.offset + field.width > bytes.len() * 8 {
            return None;
        }
        Some(
            (0..field.width).any(|bit| {
                bytes[(field.offset + bit) / 8] & (1 << ((field.offset + bit) % 8)) != 0
            }),
        )
    }

    pub fn decode(&self, payload: &[u8]) -> Option<(i32, i32)> {
        let (id, bytes) = if self.report_ids {
            (*payload.first()?, payload.get(1..)?)
        } else {
            (0, payload)
        };
        let fields = self.reports.get(&id)?;
        let read = |axis: &Axis| -> Option<i32> {
            if axis.offset + axis.width > bytes.len() * 8 {
                return None;
            }
            let mut value = 0u32;
            for bit in 0..axis.width {
                value |= (((bytes[(axis.offset + bit) / 8] >> ((axis.offset + bit) % 8)) & 1)
                    as u32)
                    << bit;
            }
            Some(if axis.signed {
                ((value << (32 - axis.width)) as i32) >> (32 - axis.width)
            } else {
                i32::try_from(value).ok()?
            })
        };
        Some((read(&fields[0])?, read(&fields[1])?))
    }
}

impl Decoder {
    pub fn decode(&self, bytes: &[u8]) -> Option<(i32, i32)> {
        match self {
            Self::Unavailable => None,
            Self::Descriptor(layout) | Self::WindowsDescriptor(layout) => layout.decode(bytes),
        }
    }
    /// whether a button is down in the report, when the layout has buttons
    pub fn left_button(&self, bytes: &[u8]) -> Option<bool> {
        match self {
            Self::Unavailable => None,
            Self::Descriptor(layout) | Self::WindowsDescriptor(layout) => layout.left_button(bytes),
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unavailable => "timing only",
            Self::Descriptor(_) => "HID descriptor",
            Self::WindowsDescriptor(_) => "Windows HID layout",
        }
    }
}

#[cfg(test)]
mod tests;
