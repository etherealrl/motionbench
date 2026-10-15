use std::path::PathBuf;
use std::sync::OnceLock;

static CHOSEN: OnceLock<PathBuf> = OnceLock::new();

/// where recordings and the library are kept: the folder picked in the settings when the app
/// started, or the default
pub fn recordings() -> PathBuf {
    CHOSEN.get().cloned().unwrap_or_else(default_recordings)
}

/// Motion Bench in Documents, wherever the user moved Documents
pub fn default_recordings() -> PathBuf {
    documents().join("Motion Bench")
}

/// the folder this run keeps its recordings in, set once before anything opens the library
pub fn choose(folder: PathBuf) {
    let _ = CHOSEN.set(folder);
}

#[cfg(windows)]
fn known(id: &windows::core::GUID) -> Result<PathBuf, String> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    let path =
        unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.map_err(|e| e.to_string())?;
    let folder = unsafe { path.to_string() }.map_err(|e| e.to_string());
    unsafe { CoTaskMemFree(Some(path.0 as *const _)) };
    folder.map(PathBuf::from)
}

#[cfg(windows)]
fn documents() -> PathBuf {
    // a broken folder redirection can refuse, and the profile's own Documents is where it would be anyway
    known(&windows::Win32::UI::Shell::FOLDERID_Documents).unwrap_or_else(|_| {
        PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Documents")
    })
}

#[cfg(not(windows))]
fn documents() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Documents")
}

#[cfg(windows)]
pub fn program_files() -> Result<PathBuf, String> {
    known(&windows::Win32::UI::Shell::FOLDERID_ProgramFiles)
}
