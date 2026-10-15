use crate::analysis::{compare::Selection, descriptors, texture};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

#[derive(Debug)]
struct Array {
    kind: char,
    shape: Vec<usize>,
    numbers: Vec<f64>,
    strings: Vec<String>,
}

fn value<'a>(header: &'a str, key: &str) -> Result<&'a str, String> {
    let after = header
        .split_once(&format!("'{key}'"))
        .or_else(|| header.split_once(&format!("\"{key}\"")))
        .ok_or_else(|| format!("NPY header is missing {key}"))?
        .1;
    Ok(after
        .split_once(':')
        .ok_or("invalid NPY header")?
        .1
        .trim_start())
}

fn read_array(bytes: &[u8]) -> Result<Array, String> {
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err("invalid NPY header".into());
    }
    let (start, header_len) = match bytes[6] {
        1 => (
            10,
            u16::from_le_bytes(bytes[8..10].try_into().unwrap()) as usize,
        ),
        2 | 3 if bytes.len() >= 12 => (
            12,
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
        ),
        _ => return Err("unsupported NPY version".into()),
    };
    let header = std::str::from_utf8(
        bytes
            .get(start..start + header_len)
            .ok_or("truncated NPY header")?,
    )
    .map_err(|e| e.to_string())?;
    if value(header, "fortran_order")?.starts_with("True") {
        return Err("Fortran-order arrays aren't supported, so export it in C order".into());
    }
    let raw_shape = value(header, "shape")?
        .strip_prefix('(')
        .ok_or("invalid NPY shape")?
        .split_once(')')
        .ok_or("invalid NPY shape")?
        .0;
    let shape = raw_shape
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse::<usize>().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let count = shape
        .iter()
        .try_fold(1usize, |a, b| a.checked_mul(*b))
        .ok_or("NPY shape overflow")?;
    let raw_dtype = value(header, "descr")?;
    let quote = raw_dtype.chars().next().ok_or("missing NPY dtype")?;
    let dtype = raw_dtype[quote.len_utf8()..]
        .split(quote)
        .next()
        .ok_or("missing NPY dtype")?;
    if dtype.starts_with('>') || dtype.contains('O') {
        return Err("big-endian and pickle/object arrays are unsupported".into());
    }
    let dtype = dtype.trim_start_matches(['<', '|', '=']);
    let kind = dtype.chars().next().ok_or("missing NPY dtype")?;
    let width = dtype[kind.len_utf8()..]
        .parse::<usize>()
        .map_err(|e| e.to_string())?;
    let stride = if kind == 'U' {
        width.checked_mul(4).ok_or("string width overflow")?
    } else {
        width
    };
    if stride == 0 {
        return Err("empty NPY dtype".into());
    }
    let expected = count.checked_mul(stride).ok_or("NPY data overflow")?;
    let payload = bytes
        .get(start + header_len..)
        .ok_or("truncated NPY data")?;
    if payload.len() != expected {
        return Err("NPY shape and data length disagree".into());
    }
    let mut numbers = Vec::new();
    let mut strings = Vec::new();
    for row in payload.chunks_exact(stride) {
        match (kind, width) {
            ('f', 4) => numbers.push(f32::from_le_bytes(row.try_into().unwrap()) as f64),
            ('f', 8) => numbers.push(f64::from_le_bytes(row.try_into().unwrap())),
            ('i', 2) => numbers.push(i16::from_le_bytes(row.try_into().unwrap()) as f64),
            ('i', 4) => numbers.push(i32::from_le_bytes(row.try_into().unwrap()) as f64),
            ('i', 8) => numbers.push(i64::from_le_bytes(row.try_into().unwrap()) as f64),
            ('u', 1) | ('b', 1) => numbers.push(row[0] as f64),
            ('U', _) => {
                let mut text = String::new();
                for code in row.chunks_exact(4) {
                    let code = u32::from_le_bytes(code.try_into().unwrap());
                    if code != 0 {
                        text.push(char::from_u32(code).ok_or("invalid NPY unicode")?);
                    }
                }
                strings.push(text);
            }
            ('S', _) => strings.push(
                std::str::from_utf8(row.split(|v| *v == 0).next().unwrap())
                    .map_err(|e| e.to_string())?
                    .into(),
            ),
            _ => return Err(format!("unsupported NPY dtype {dtype}")),
        }
    }
    if numbers.iter().any(|v| !v.is_finite()) {
        return Err("dataset contains non-finite values".into());
    }
    Ok(Array {
        kind,
        shape,
        numbers,
        strings,
    })
}

pub struct Dataset {
    missing_fields: std::collections::BTreeSet<String>,
    /// each event's span on the pcap clock, from its prefix's start to its click
    pub spans: Option<Vec<[u64; 2]>>,
    pub future: Vec<Vec<[f64; 2]>>,
    pub prefix: Option<Vec<Vec<[f64; 2]>>>,
    pub targets: Option<Vec<[f64; 2]>>,
    pub radii: Option<Vec<f64>>,
    pub groups: Option<Vec<String>>,
}

fn streams(raw: &Array, mask: &Array) -> Result<Vec<Vec<[f64; 2]>>, String> {
    if raw.shape.len() != 3
        || raw.shape[2] != 2
        || mask.shape != raw.shape[..2]
        || raw.numbers.is_empty()
        || mask.numbers.is_empty()
    {
        return Err("movement arrays need [N,H,2] counts and [N,H] masks".into());
    }
    let n = raw.shape[0];
    let h = raw.shape[1];
    Ok((0..n)
        .map(|i| {
            (0..h)
                .filter(|j| mask.numbers[i * h + j] > 0.5)
                .map(|j| {
                    [
                        raw.numbers[(i * h + j) * 2],
                        raw.numbers[(i * h + j) * 2 + 1],
                    ]
                })
                .collect()
        })
        .collect())
}

/// each event's source as judges.py spells its group, since the folds sort those strings and a
/// different spelling is a different split
fn identities(array: &Array) -> Result<Vec<String>, String> {
    // integers are read as f64, which holds them exactly only up to 2^53 - 1
    const EXACT: f64 = 9_007_199_254_740_991.0;
    if !array.strings.is_empty() {
        return Ok(array.strings.iter().map(|s| format!("str:{s}")).collect());
    }
    if ['i', 'u'].contains(&array.kind) && array.numbers.iter().any(|n| n.abs() > EXACT) {
        return Err(
            "source integer identity exceeds exact numeric range, export identities as strings"
                .into(),
        );
    }
    Ok(array
        .numbers
        .iter()
        .map(|v| match array.kind {
            'f' => format!("float:{v:?}"),
            'b' => format!("bool:{}", if *v == 0.0 { "False" } else { "True" }),
            _ => format!("int:{v}"),
        })
        .collect())
}

impl Dataset {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut archive = zip::ZipArchive::new(File::open(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let mut arrays = BTreeMap::new();
        let mut total = 0u64;
        let wanted = [
            "future_raw_dxdy",
            "future_mask",
            "prefix_raw_dxdy",
            "prefix_mask",
            "target_rel_x_at_B",
            "target_rel_y_at_B",
            "target_radius",
            "source_trial_id",
            "source_group",
            "user_id",
            "event_from_us",
            "event_to_us",
        ];
        for key in wanted {
            if let Ok(mut file) = archive.by_name(&format!("{key}.npy")) {
                total = total
                    .checked_add(file.size())
                    .ok_or("dataset size overflow")?;
                if total > 512 * 1024 * 1024 {
                    return Err("dataset exceeds the 512 MiB import limit".into());
                }
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                arrays.insert(key, read_array(&bytes)?);
            }
        }
        let future = streams(
            arrays
                .get("future_raw_dxdy")
                .ok_or("missing future_raw_dxdy")?,
            arrays.get("future_mask").ok_or("missing future_mask")?,
        )?;
        let n = future.len();
        let prefix = match (arrays.get("prefix_raw_dxdy"), arrays.get("prefix_mask")) {
            (Some(raw), Some(mask)) => Some(streams(raw, mask)?),
            (None, None) => None,
            _ => None,
        };
        if prefix.as_ref().is_some_and(|p| p.len() != n) {
            return Err("prefix and future row counts differ".into());
        }
        let vector = |key| -> Result<Option<Vec<f64>>, String> {
            arrays
                .get(key)
                .map(|a| {
                    if a.shape != [n] || a.numbers.len() != n {
                        Err(format!("{key} needs [N] numeric values"))
                    } else {
                        Ok(a.numbers.clone())
                    }
                })
                .transpose()
        };
        let targets = match (vector("target_rel_x_at_B")?, vector("target_rel_y_at_B")?) {
            (Some(x), Some(y)) => Some(x.into_iter().zip(y).map(|(x, y)| [x, y]).collect()),
            (None, None) => None,
            _ => None,
        };
        let spans = match (vector("event_from_us")?, vector("event_to_us")?) {
            (Some(from), Some(to)) => Some(
                from.into_iter()
                    .zip(to)
                    .map(|(from, to)| [from as u64, to as u64])
                    .collect(),
            ),
            _ => None,
        };
        let radii = vector("target_radius")?;
        if radii.as_ref().is_some_and(|r| r.iter().any(|r| *r <= 0.0)) {
            return Err("target radii must be positive".into());
        }
        let groups = ["source_trial_id", "source_group", "user_id"]
            .iter()
            .find_map(|key| arrays.get(key))
            .map(|array| {
                if array.shape != [n] {
                    return Err("source identities need one value per event".to_string());
                }
                identities(array)
            })
            .transpose()?;
        let missing_fields = wanted
            .into_iter()
            .filter(|key| !arrays.contains_key(key))
            .map(str::to_string)
            .collect();
        Ok(Self {
            missing_fields,
            spans,
            future,
            prefix,
            targets,
            radii,
            groups,
        })
    }

    /// only the events that lie whole inside `crop`
    pub fn within(&mut self, crop: &Selection) -> Result<(), String> {
        let spans = self.spans.as_ref().ok_or(
            "this recording's target events have no times, so its crop cannot be applied to them, build its events again",
        )?;
        let keep: Vec<bool> = spans
            .iter()
            .map(|[from, to]| crop.contains(*from) && crop.contains(*to))
            .collect();
        fn retain<T>(values: &mut Vec<T>, keep: &[bool]) {
            let mut kept = keep.iter();
            values.retain(|_| *kept.next().unwrap());
        }
        retain(&mut self.future, &keep);
        if let Some(prefix) = &mut self.prefix {
            retain(prefix, &keep);
        }
        if let Some(targets) = &mut self.targets {
            retain(targets, &keep);
        }
        if let Some(radii) = &mut self.radii {
            retain(radii, &keep);
        }
        if let Some(groups) = &mut self.groups {
            retain(groups, &keep);
        }
        if let Some(spans) = &mut self.spans {
            retain(spans, &keep);
        }
        Ok(())
    }

    pub fn rows(&self, panel: &str) -> Result<(Vec<String>, Vec<Vec<f64>>), String> {
        let names = match panel {
            "texture19" => texture::NAMES.to_vec(),
            "trajectory14" => descriptors::TRAJECTORY.to_vec(),
            "full49" => descriptors::TRAJECTORY
                .iter()
                .chain(texture::NAMES.iter())
                .chain(descriptors::TARGET.iter())
                .copied()
                .collect(),
            _ => return Err("unknown descriptor panel".into()),
        };
        if panel != "texture19" && (self.targets.is_none() || self.radii.is_none()) {
            let missing = ["target_rel_x_at_B", "target_rel_y_at_B", "target_radius"]
                .into_iter()
                .filter(|key| self.missing_fields.contains(*key))
                .collect::<Vec<_>>();
            return Err(format!("missing {}", missing.join(", ")));
        }
        if panel == "full49" && self.prefix.is_none() {
            let missing = ["prefix_raw_dxdy", "prefix_mask"]
                .into_iter()
                .filter(|key| self.missing_fields.contains(*key))
                .collect::<Vec<_>>();
            return Err(format!(
                "missing {} for seam measurements",
                missing.join(", ")
            ));
        }
        let rows =
            self.future
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let texture = || -> Result<Vec<f64>, String> {
                        if v.iter().flatten().any(|v| {
                            v.fract() != 0.0 || *v < i32::MIN as f64 || *v > i32::MAX as f64
                        }) {
                            return Err("raw texture requires integer mouse counts".into());
                        }
                        Ok(texture::features(
                            &v.iter()
                                .map(|v| [v[0] as i32, v[1] as i32])
                                .collect::<Vec<_>>(),
                        )
                        .to_vec())
                    };
                    if panel == "texture19" {
                        return texture();
                    }
                    let target = self.targets.as_ref().unwrap()[i];
                    let radius = self.radii.as_ref().unwrap()[i];
                    let mut row = descriptors::trajectory(v, target, radius);
                    if panel == "full49" {
                        row.extend(texture()?);
                        row.extend(descriptors::target_event(
                            v,
                            &self.prefix.as_ref().unwrap()[i],
                            target,
                            radius,
                        ));
                    }
                    Ok(row)
                })
                .collect::<Result<Vec<_>, _>>()?;
        Ok((names.iter().map(|s| s.to_string()).collect(), rows))
    }
}

#[cfg(test)]
mod tests;
