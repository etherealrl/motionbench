//! the saved preferences, held in memory and written whole; every limit on them is checked here
use crate::storage::backup;
use motion_bench::sync::LockExt;
use motion_bench::targets::vision;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

const VERSION: u64 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    version: u64,
    pub recording: Recording,
    pub capture: Capture,
    pub aim: Aim,
    pub vision: vision::Config,
    #[serde(default)]
    pub targets: Targets,
    pub display: Display,
    #[serde(default)]
    pub storage: Storage,
}
/// where recordings, the library and results go. none means Motion Bench in Documents. it's read
/// once at startup, since the library and the recorder already use the folder they opened with
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    pub folder: Option<std::path::PathBuf>,
}
/// target capture starts by itself when the app does, if asked to; off unless chosen, since it
/// takes the GPU and the screen
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Targets {
    pub start_on_launch: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub button: Option<Button>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub root: Option<String>,
    pub mouse: Option<SavedMouse>,
    /// the box to take again, picked or the only one there the first time
    #[serde(default, rename = "box")]
    pub box_: Option<motion_bench::hardware::boxes::SavedBox>,
}
/// the mouse to select again, by what stays the same when it is plugged back in
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedMouse {
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub bus: u16,
    pub endpoint: u8,
}
/// the aim controller's settings; kept in every build, so a build without the developer tools
/// never drops them
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Aim {
    pub speed: f64,
    pub deadzone_counts: f64,
    pub button: Button,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub zoom: Zoom,
    pub sections: Sections,
    /// how loud the recording cues are, kept while muted so unmuting goes back to it
    #[serde(default)]
    pub volume: Volume,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub theme: Theme,
}
/// the app's colors, or whatever the system uses
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
/// which Capture setup sections are open
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sections {
    pub targets: bool,
    #[serde(rename = "box")]
    pub box_: bool,
    pub box_settings: bool,
}

/// one mouse button, as its bit in the box's button mask
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Button(u32);
impl TryFrom<u32> for Button {
    type Error = String;
    fn try_from(mask: u32) -> Result<Self, String> {
        if mask.count_ones() != 1 {
            return Err("a binding is one mouse button".into());
        }
        Ok(Self(mask))
    }
}
impl From<Button> for u32 {
    fn from(button: Button) -> u32 {
        button.0
    }
}
impl Button {
    pub fn mask(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Zoom(f64);
impl TryFrom<f64> for Zoom {
    type Error = String;
    fn try_from(zoom: f64) -> Result<Self, String> {
        if ![1.0, 1.1, 1.25, 1.5].contains(&zoom) {
            return Err("the UI scale is 100, 110, 125 or 150%".into());
        }
        Ok(Self(zoom))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Volume(f64);
impl Default for Volume {
    fn default() -> Self {
        Self(0.6)
    }
}
impl TryFrom<f64> for Volume {
    type Error = String;
    fn try_from(volume: f64) -> Result<Self, String> {
        if !(0.0..=1.0).contains(&volume) {
            return Err("the volume goes from 0 to 100%".into());
        }
        Ok(Self(volume))
    }
}
impl From<Volume> for f64 {
    fn from(volume: Volume) -> f64 {
        volume.0
    }
}
impl Display {
    /// the cues' loudness right now, nothing while muted
    pub fn cue_volume(&self) -> f64 {
        if self.muted { 0.0 } else { self.volume.0 }
    }
}
impl From<Zoom> for f64 {
    fn from(zoom: Zoom) -> f64 {
        zoom.0
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: VERSION,
            recording: Recording { button: None },
            capture: Capture {
                root: None,
                mouse: None,
                box_: None,
            },
            aim: Aim {
                speed: 2000.0,
                deadzone_counts: 12.0,
                button: Button(8),
            },
            vision: vision::Config::default(),
            targets: Targets::default(),
            storage: Storage::default(),
            display: Display {
                zoom: Zoom(1.25),
                volume: Volume::default(),
                muted: false,
                theme: Theme::default(),
                sections: Sections {
                    targets: false,
                    box_: false,
                    box_settings: false,
                },
            },
        }
    }
}
impl Settings {
    fn validate(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err("unsupported configuration version".into());
        }
        let aim = &self.aim;
        if !(100.0..=20_000.0).contains(&aim.speed) {
            return Err("aim speed is between 100 and 20,000 counts/s".into());
        }
        if !(0.0..=100.0).contains(&aim.deadzone_counts) {
            return Err("the deadzone is between 0 and 100 mouse counts".into());
        }
        if self
            .storage
            .folder
            .as_ref()
            .is_some_and(|f| !f.is_absolute())
        {
            return Err("the recordings folder has to be a full path".into());
        }
        self.vision.validate()
    }
}

/// what the ui may change; bindings and the capture choice are saved where they are made
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    aim: Option<AimPatch>,
    vision: Option<vision::Config>,
    targets: Option<Targets>,
    display: Option<Display>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AimPatch {
    speed: f64,
    deadzone_counts: f64,
}
impl Patch {
    pub fn apply(self, settings: &mut Settings) {
        if let Some(aim) = self.aim {
            settings.aim.speed = aim.speed;
            settings.aim.deadzone_counts = aim.deadzone_counts;
        }
        if let Some(vision) = self.vision {
            settings.vision = vision;
        }
        if let Some(targets) = self.targets {
            settings.targets = targets;
        }
        if let Some(display) = self.display {
            settings.display = display;
        }
    }
}

fn path() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join(".motion-bench/config.json"))
        .ok_or_else(|| "could not locate the user configuration folder".into())
}

/// the settings in use. a change is checked, kept and then written; a write that fails keeps the
/// change until the app closes and says so
pub struct Store {
    path: PathBuf,
    state: Mutex<State>,
    /// why startup did not use the saved settings
    notice: Option<String>,
}
struct State {
    settings: Settings,
    /// the file is never written: a newer app's settings, or a corrupt file no copy was kept of
    refused: bool,
    unsaved: Option<String>,
}
impl Store {
    pub fn load() -> Self {
        match path() {
            Ok(path) => Self::open(path),
            Err(reason) => Self::refused(PathBuf::new(), format!("{reason}, {NOT_SAVED}")),
        }
    }
    /// the saved settings, or the defaults with a notice. a corrupt file is replaced only once a
    /// copy of it is kept; a missing one is written with the first change
    pub fn open(path: PathBuf) -> Self {
        let notice = match read(&path) {
            Ok(saved) => return Self::with(path, saved.unwrap_or_default(), false, None, None),
            Err(Unusable::Corrupt { bytes, reason }) => match backup::preserve(&path, &bytes) {
                Ok(copy) => {
                    let notice = format!(
                        "the settings could not be read ({reason}), so they were reset to the defaults. the old file is kept as {}",
                        copy.display()
                    );
                    let (notice, unsaved) = match write(&path, &Settings::default()) {
                        Ok(()) => (notice, None),
                        Err(e) => (
                            format!(
                                "{notice}. the settings could not be saved ({e}), so they only last until the app closes"
                            ),
                            Some(e),
                        ),
                    };
                    return Self::with(path, Settings::default(), false, Some(notice), unsaved);
                }
                Err(e) => format!(
                    "the settings could not be read ({reason}) and a copy of them could not be kept ({e}), so the file was left as it is and {NOT_SAVED}"
                ),
            },
            Err(Unusable::Refused(reason)) => format!("{reason}, {NOT_SAVED}"),
        };
        Self::refused(path, notice)
    }
    fn refused(path: PathBuf, notice: String) -> Self {
        Self::with(path, Settings::default(), true, Some(notice), None)
    }
    fn with(
        path: PathBuf,
        settings: Settings,
        refused: bool,
        notice: Option<String>,
        unsaved: Option<String>,
    ) -> Self {
        Self {
            path,
            state: Mutex::new(State {
                settings,
                refused,
                unsaved,
            }),
            notice,
        }
    }
    pub fn notice(&self) -> Option<String> {
        self.notice.clone()
    }
    pub fn get(&self) -> Settings {
        self.state.locked().settings.clone()
    }
    /// why the last change was not written, until one is
    pub fn unsaved(&self) -> Option<String> {
        self.state.locked().unsaved.clone()
    }
    /// a change the limits refuse changes nothing and says why; any other change is kept
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        let mut state = self.state.locked();
        let mut next = state.settings.clone();
        change(&mut next);
        next.validate()?;
        if !state.refused {
            state.unsaved = write(&self.path, &next).err().map(|e| {
                format!("the settings could not be saved ({e}), so the last change only lasts until the app closes")
            });
        }
        state.settings = next.clone();
        Ok(next)
    }
}

const NOT_SAVED: &str = "so the defaults are in use and changes to them are not saved";

/// why the saved settings were not used. only a corrupt file may be replaced, and only once a copy
/// of it is kept
#[derive(Debug)]
enum Unusable {
    Corrupt { bytes: Vec<u8>, reason: String },
    Refused(String),
}
/// the saved settings, none when there is no file yet
fn read(path: &Path) -> Result<Option<Settings>, Unusable> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Unusable::Refused(format!(
                "could not read configuration: {e}"
            )));
        }
    };
    let saved: Value = match serde_json::from_slice(&bytes) {
        Ok(saved) => saved,
        Err(e) => {
            let reason = e.to_string();
            return Err(Unusable::Corrupt { bytes, reason });
        }
    };
    // a newer app may save settings this one cannot read, they are never reset from here
    if saved["version"]
        .as_u64()
        .is_some_and(|version| version > VERSION)
    {
        return Err(Unusable::Refused(
            "the settings were saved by a newer version of the app".into(),
        ));
    }
    match serde_json::from_value::<Settings>(saved)
        .map_err(|e| e.to_string())
        .and_then(|settings| settings.validate().map(|()| settings))
    {
        Ok(settings) => Ok(Some(settings)),
        Err(reason) => Err(Unusable::Corrupt { bytes, reason }),
    }
}
fn write(path: &Path, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("configuration path has no parent")?)
        .map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
