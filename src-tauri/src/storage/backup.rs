use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// keeps a byte-identical copy of an unreadable file next to it, before anything replaces it, and
/// returns where the copy is. `library.json` goes to `library.corrupt-1.json`, then `-2` and so
/// on. a copy that already holds the same bytes is reused, so a file that stays broken across
/// restarts is only copied once. any error means the original must stay where it is
pub fn preserve(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let stem = path
        .file_stem()
        .ok_or("the file has no name")?
        .to_string_lossy();
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let (mut n, mut lost_races) = (1, 0);
    loop {
        let copy = path.with_file_name(format!("{stem}.corrupt-{n}{extension}"));
        match std::fs::read(&copy) {
            Ok(existing) if existing == bytes => return Ok(copy),
            Ok(_) => n += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match write_new(&copy, bytes) {
                // another caller took the slot first, so look at what it holds. a slot that reads
                // as missing but cannot be created (a dangling link) would loop forever, hence the cap
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && lost_races < 3 => {
                    lost_races += 1
                }
                Err(e) => return Err(e.to_string()),
                // read back, a copy nobody checked is not a backup
                Ok(()) => {
                    return match std::fs::read(&copy) {
                        Ok(written) if written == bytes => Ok(copy),
                        Ok(_) => Err(format!("{} does not match the original", copy.display())),
                        Err(e) => Err(e.to_string()),
                    };
                }
            },
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests;
