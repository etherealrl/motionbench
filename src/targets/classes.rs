//! the class names a target model was exported with, read from the onnx file's metadata
//! without loading the model, so the target class can be picked by name

use std::io::{self, Read, Seek, SeekFrom};

/// `ModelProto.metadata_props`, and the key and value of each entry
const METADATA_PROPS: u64 = 14;
const KEY: u64 = 1;
const VALUE: u64 = 2;

fn varint(reader: &mut impl Read) -> io::Result<Option<u64>> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let mut byte = [0u8];
        if reader.read(&mut byte)? == 0 {
            return if shift == 0 {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        value |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(Some(value));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "varint too long",
    ))
}

/// the length delimited fields of one message level, other fields skipped; the graph, by far
/// the largest field, is seeked past rather than read
fn fields(
    reader: &mut (impl Read + Seek),
    end: u64,
    mut field: impl FnMut(u64, Vec<u8>),
    keep: impl Fn(u64) -> bool,
) -> io::Result<()> {
    while reader.stream_position()? < end {
        let Some(key) = varint(reader)? else { break };
        let (number, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                varint(reader)?;
            }
            1 => {
                reader.seek(SeekFrom::Current(8))?;
            }
            5 => {
                reader.seek(SeekFrom::Current(4))?;
            }
            2 => {
                let length = varint(reader)?.ok_or(io::ErrorKind::UnexpectedEof)?;
                // a broken or foreign file can claim any length, which would allocate forever or
                // seek backwards and loop
                if length > end.saturating_sub(reader.stream_position()?) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "not an onnx model",
                    ));
                }
                if keep(number) {
                    let mut bytes = vec![0; length as usize];
                    reader.read_exact(&mut bytes)?;
                    field(number, bytes);
                } else {
                    reader.seek(SeekFrom::Current(length as i64))?;
                }
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "not an onnx model",
                ));
            }
        }
    }
    Ok(())
}

/// the model's `names` metadata, as ultralytics writes it: `{0: 'person', 1: 'car'}`. empty
/// when the model has none
pub fn class_names(reader: &mut (impl Read + Seek)) -> io::Result<Vec<String>> {
    let end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;
    let mut names = None;
    fields(
        reader,
        end,
        |_, entry| {
            let mut key = String::new();
            let mut value = String::new();
            let length = entry.len() as u64;
            let _ = fields(
                &mut io::Cursor::new(entry),
                length,
                |number, bytes| {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    if number == KEY {
                        key = text;
                    } else {
                        value = text;
                    }
                },
                |number| number == KEY || number == VALUE,
            );
            if key == "names" {
                names = Some(value);
            }
        },
        |number| number == METADATA_PROPS,
    )?;
    Ok(names.map(|names| parse(&names)).unwrap_or_default())
}

/// `{0: 'person', 1: "cat's toy"}` into its names in index order; a gap in the indices or
/// anything else unexpected gives no names rather than names on the wrong classes
fn parse(names: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = names.trim().trim_start_matches('{');
    while let Some((index, after)) = rest.split_once(':') {
        let Ok(index) = index.trim().trim_start_matches(',').trim().parse::<usize>() else {
            return Vec::new();
        };
        let after = after.trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '\'' || *c == '"') else {
            return Vec::new();
        };
        let Some((name, tail)) = after[1..].split_once(quote) else {
            return Vec::new();
        };
        if index != out.len() {
            return Vec::new();
        }
        out.push(name.to_string());
        rest = tail;
    }
    out
}

#[cfg(test)]
mod tests;
