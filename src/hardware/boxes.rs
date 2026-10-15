//! which boxes are plugged in and opening the one picked. behind a trait so the pick can be tested
//! without a box: anybox's own scan and its `Found` can't be built from outside it

pub use anybox::Family;
use anybox::{Action, Connect, FerrumApi};
use serde::{Deserialize, Serialize};

/// one box a scan saw
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// what the app knows it by: its own id when it has one, otherwise its family and port
    pub key: String,
    pub port: String,
    pub family: Family,
    pub model: Option<String>,
    /// it gives buttons or moves right now; a keyboard box is listed but never counted
    pub usable: bool,
    /// a Ferrum's API, so opening it by its port asks for the one it was found on
    pub ferrum_api: Option<FerrumApi>,
}

impl Listed {
    /// "Medius COM18", how the list names it; the port tells two of one family apart
    pub fn label(&self) -> String {
        format!("{} {}", self.family, self.port)
    }

    /// the row a box that is already open gets, since a scan only sees its port as busy
    pub fn of(device: &anybox::Device) -> Self {
        let snapshot = device.snapshot();
        let can = |action| snapshot.capabilities.get(action).is_supported();
        Listed {
            key: key(&snapshot.info),
            port: snapshot.info.port.clone(),
            family: snapshot.info.family,
            model: snapshot.info.model.clone(),
            usable: can(Action::ButtonEvents) || can(Action::Move),
            ferrum_api: snapshot.info.ferrum_api,
        }
    }

    pub fn saved(&self) -> SavedBox {
        SavedBox {
            key: self.key.clone(),
            family: self.family.to_string(),
            port: self.port.clone(),
            ferrum_api: self.ferrum_api.map(|api| format!("{api:?}")),
        }
    }
}

/// the box to take again at the next start. its key says which box, the rest is where to look when
/// a scan can't see it: a box held by another program, or a Ferrum or the first version of the YSC 8K, which a scan
/// never looks for
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedBox {
    pub key: String,
    pub family: String,
    pub port: String,
    pub ferrum_api: Option<String>,
}

impl SavedBox {
    /// the row to open it with, none when the family is one this build doesn't know
    pub fn listed(&self) -> Option<Listed> {
        let family = [Family::Medius, Family::MakcuV4, Family::Ferrum, Family::Ysc]
            .into_iter()
            .find(|family| family.to_string() == self.family)?;
        let ferrum_api = [FerrumApi::Software, FerrumApi::Legacy]
            .into_iter()
            .find(|api| Some(format!("{api:?}")) == self.ferrum_api);
        Some(Listed {
            key: self.key.clone(),
            port: self.port.clone(),
            family,
            model: None,
            usable: true,
            ferrum_api,
        })
    }
}

/// what opening a box came to
pub enum Opened {
    Device(anybox::Device),
    /// it's there but can't be had right now: another program holds it, or it isn't allowed
    Blocked(String),
    /// nothing of that family answered there
    Absent(String),
}

pub trait Boxes: Send {
    /// the ports a scan would look at, nothing sent to them
    fn candidates(&self) -> Vec<String>;
    /// every box anybox looks for by itself, opened and described, slow
    fn scan(&self) -> Scan;
    fn open(&self, listed: &Listed) -> Opened;
}

/// what a scan saw, port by port
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub boxes: Vec<Listed>,
    pub unusable: Vec<Unusable>,
}

/// a port a scan couldn't use, and anybox's reason
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unusable {
    pub port: String,
    pub why: String,
}

/// the boxes on this PC
pub struct Plugged;

impl Boxes for Plugged {
    fn candidates(&self) -> Vec<String> {
        anybox::candidates()
    }

    fn scan(&self) -> Scan {
        let mut scan = Scan::default();
        for scanned in anybox::scan() {
            match scanned.result {
                Ok(found) => {
                    let can = |action| found.capabilities.get(action).is_supported();
                    scan.boxes.push(Listed {
                        key: key(&found.info),
                        usable: can(Action::ButtonEvents) || can(Action::Move),
                        port: found.port,
                        family: found.info.family,
                        model: found.info.model,
                        ferrum_api: found.info.ferrum_api,
                    });
                }
                Err(error) => scan.unusable.push(Unusable {
                    port: scanned.port,
                    why: why_unusable(&error),
                }),
            }
        }
        scan
    }

    fn open(&self, listed: &Listed) -> Opened {
        let mut connect = Connect::port(&listed.port).family(listed.family);
        if let Some(api) = listed.ferrum_api {
            connect = connect.ferrum_api(api);
        }
        match connect.open() {
            Ok(device) => Opened::Device(device),
            Err(
                error @ (anybox::Error::PortBusy { .. } | anybox::Error::PermissionDenied { .. }),
            ) => Opened::Blocked(error.to_string()),
            Err(error) => Opened::Absent(error.to_string()),
        }
    }
}

/// the app's words for a port a scan couldn't use, since anybox's own talk to whoever codes
/// against it
fn why_unusable(error: &anybox::Error) -> String {
    use anybox::Error;
    match error {
        Error::PortBusy { .. } => "another program has it open, close it there first".into(),
        Error::PermissionDenied { .. } => "Windows won't let the app open it".into(),
        Error::NotFound { .. } => {
            "nothing answered, so it's not a box, or one that's still starting up".into()
        }
        Error::FirmwareTooOld { family, needed, .. } => {
            format!("this {family} needs firmware {needed} or newer, so update it first")
        }
        Error::LibraryTooOld { family, .. } => {
            format!("this {family} runs firmware newer than the app knows, so update the app")
        }
        other => format!("it can't be used here, {other}"),
    }
}

/// a box's own id where it has one, so it is known again on another port; otherwise where it is
pub fn key(info: &anybox::Info) -> String {
    match &info.id {
        Some(id) => id.to_string(),
        None => format!("{} {}", info.family, info.port),
    }
}
