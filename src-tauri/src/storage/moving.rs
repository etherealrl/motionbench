//! the recordings going along to the folder picked in the settings, at the start after the pick,
//! before the library opens either folder
use std::io;
use std::path::Path;

/// moves the app's files in `from` into `to`, the recordings, the library, its results and the
/// descriptors. nothing already in `to` gets replaced, a name that's taken stays where it was, and
/// what comes back says so, like anything that couldn't move. it's an error when `from` can't be
/// read while its drive isn't there, like one that isn't plugged in, so the move can be tried again
/// later
pub fn move_recordings(from: &Path, to: &Path) -> Result<Vec<String>, String> {
    let mut left = Vec::new();
    let names: Vec<String> = match std::fs::read_dir(from)
        .and_then(|entries| entries.collect::<Result<Vec<_>, _>>())
    {
        Ok(entries) => entries
            .iter()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
        // a folder that's gone from a drive that's there won't come back, so it isn't tried again
        Err(e) if from.ancestors().last().is_some_and(|drive| drive.exists()) => {
            return Ok(vec![format!(
                "the recordings in {} couldn't be read ({e}), so none of them moved",
                from.display()
            )]);
        }
        Err(e) => {
            return Err(format!(
                "the recordings in {} couldn't be reached ({e}), so they'll move at the next start instead",
                from.display()
            ));
        }
    };
    // the folder has to be there so its real path compares with the old one's
    if let Err(e) = std::fs::create_dir_all(to) {
        // like a drive that was unplugged before the restart, so the move waits for it
        return Err(format!(
            "the recordings couldn't move to {} ({e}), so they'll move at the next start instead",
            to.display()
        ));
    }
    // Windows paths ignore case, and a link or a trailing separator can name the same folder too
    let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let (real_from, real_to) = (real(from), real(to));
    if real_from == real_to {
        return Ok(left);
    }
    if real_to.starts_with(&real_from) || real_from.starts_with(&real_to) {
        left.push(format!(
            "{} and {} are inside one another, so the recordings stayed where they were",
            from.display(),
            to.display()
        ));
        return Ok(left);
    }
    // a recording is every file named after its capture, and anything else in the folder isn't
    // ours, since it could be a folder like Documents that holds other things too
    let stems: Vec<&str> = names
        .iter()
        .filter_map(|name| name.strip_suffix(".pcap"))
        .collect();
    let ours = |name: &str| {
        ["results", "descriptors"].contains(&name)
            || name.starts_with("library.")
            || stems.iter().any(|stem| {
                name.strip_prefix(stem)
                    .is_some_and(|rest| rest.starts_with('.'))
            })
    };
    move_into(from, to, &mut left, &ours);
    // the old folder goes once it's empty, and stays when something couldn't move
    let _ = std::fs::remove_dir(from);
    Ok(left)
}

fn move_into(from: &Path, to: &Path, left: &mut Vec<String>, ours: &dyn Fn(&str) -> bool) {
    let entries =
        match std::fs::read_dir(from).and_then(|entries| entries.collect::<Result<Vec<_>, _>>()) {
            Ok(entries) => entries,
            Err(e) => return left.push(format!("couldn't read {}: {e}", from.display())),
        };
    if let Err(e) = std::fs::create_dir_all(to) {
        return left.push(format!("couldn't make {}: {e}", to.display()));
    }
    for entry in entries {
        if !ours(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let source = entry.path();
        let target = to.join(entry.file_name());
        if target.exists() {
            // a folder of results or descriptors that's there already takes what's missing from it
            if source.is_dir() && target.is_dir() {
                move_into(&source, &target, left, &|_| true);
                let _ = std::fs::remove_dir(&source);
            } else {
                left.push(format!(
                    "{} was already in {}, so it stayed in {}",
                    entry.file_name().to_string_lossy(),
                    to.display(),
                    from.display()
                ));
            }
            continue;
        }
        if let Err(e) = relocate(&source, &target) {
            left.push(format!("couldn't move {}: {e}", source.display()));
        }
    }
}

/// a rename, or a copy and then a delete when the two folders are on different drives. a folder is
/// copied whole before anything in it is deleted, so a copy that fails leaves the source as it was
fn relocate(source: &Path, target: &Path) -> io::Result<()> {
    if std::fs::rename(source, target).is_ok() {
        return Ok(());
    }
    let folder = source.is_dir();
    if let Err(e) = copy(source, target) {
        let _ = if folder {
            std::fs::remove_dir_all(target)
        } else {
            std::fs::remove_file(target)
        };
        return Err(e);
    }
    if folder {
        std::fs::remove_dir_all(source)
    } else {
        std::fs::remove_file(source)
    }
}
fn copy(source: &Path, target: &Path) -> io::Result<()> {
    if source.is_dir() {
        std::fs::create_dir(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &target.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if std::fs::copy(source, target)? != std::fs::metadata(source)?.len() {
        return Err(io::Error::other("the copy came out shorter than the file"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
