//! which box the link takes, over mock boxes behind a fake search

use super::*;
use crate::hardware::boxes::{Boxes, Family, Listed, Opened, SavedBox, Scan, Unusable};
use std::sync::atomic::AtomicUsize;

/// a box the fake search lists, and the mock behind it when it can be opened
type Row = (Listed, Option<MockBox>);

/// what a fake scan sees and what opening each box does
#[derive(Clone, Default)]
struct Plug {
    boxes: Arc<Mutex<Vec<Row>>>,
    /// keys a scan sees as busy, because another program holds them, and opening them says so
    held_elsewhere: Arc<Mutex<Vec<String>>>,
    candidates: Arc<Mutex<Vec<String>>>,
    opens: Arc<AtomicUsize>,
    /// a port the scan sees but can't use, such as a box that doesn't answer
    broken: Arc<Mutex<Option<Unusable>>>,
}

impl Plug {
    /// a mock Medius with its own MAC, listed as the scan would list it
    fn medius(&self, last: u8) -> Listed {
        let mock = mock().with_version(medius::Version {
            proto_ver: 9,
            fw_major: 3,
            fw_minor: 4,
            fw_patch: 5,
            mac: [0x3c, 0xdc, 0x75, 0x52, 0x3e, last],
            name: format!("box{last}"),
        });
        let device = boxed(mock.clone());
        let row = Listed::of(&device);
        let _ = device.close();
        self.boxes.locked().push((row.clone(), Some(mock)));
        self.plugged();
        row
    }

    /// a box that gives neither buttons nor moves, a keyboard box
    fn keyboard(&self) -> Listed {
        let row = Listed {
            key: "keys".into(),
            port: "COM9".into(),
            family: Family::MakcuV4,
            model: None,
            usable: false,
            ferrum_api: None,
        };
        self.boxes.locked().push((row.clone(), None));
        self.plugged();
        row
    }

    fn unplug(&self, key: &str) -> Row {
        let mut boxes = self.boxes.locked();
        let at = boxes.iter().position(|(row, _)| row.key == key).unwrap();
        let row = boxes.remove(at);
        drop(boxes);
        self.plugged();
        row
    }

    fn plug_back(&self, row: Row) {
        self.boxes.locked().push(row);
        self.plugged();
    }

    /// what `candidates` says changes, as it does when a box is plugged in
    fn plugged(&self) {
        let count = self.boxes.locked().len();
        *self.candidates.locked() = (0..count).map(|n| format!("COM{n}")).collect();
    }
}

impl Boxes for Plug {
    fn candidates(&self) -> Vec<String> {
        self.candidates.locked().clone()
    }

    fn scan(&self) -> Scan {
        let held = self.held_elsewhere.locked().clone();
        let mut scan = Scan::default();
        for (row, _) in self.boxes.locked().iter() {
            if held.contains(&row.key) {
                scan.unusable.push(Unusable {
                    port: row.port.clone(),
                    why: format!("{} is already open in another program", row.port),
                });
            } else {
                scan.boxes.push(row.clone());
            }
        }
        scan.unusable.extend(self.broken.locked().clone());
        scan
    }

    fn open(&self, listed: &Listed) -> Opened {
        self.opens.fetch_add(1, Ordering::Relaxed);
        if self.held_elsewhere.locked().contains(&listed.key) {
            return Opened::Blocked("another program holds it".into());
        }
        let mock = self
            .boxes
            .locked()
            .iter()
            .find(|(row, _)| row.key == listed.key)
            .and_then(|(_, mock)| mock.clone());
        match mock {
            Some(mock) => Opened::Device(boxed(mock)),
            None => Opened::Absent("nothing answered".into()),
        }
    }
}

fn started(plug: &Plug, remembered: Option<SavedBox>) -> Link {
    let link = Link::with_boxes(Box::new(plug.clone()));
    link.send(Command::Start(remembered));
    settle(&link, |s| s.pending_commands == 0);
    link
}

#[test]
fn the_only_box_connects_and_is_remembered() {
    let plug = Plug::default();
    let only = plug.medius(1);
    let s = settle(&started(&plug, None), |s| s.connected);
    assert_eq!(s.key.as_deref(), Some(only.key.as_str()));
    assert_eq!(s.remember, Some(only.saved()));
    assert!(s.notice.is_none());
}

#[test]
fn a_keyboard_box_beside_a_mouse_box_doesnt_count() {
    let plug = Plug::default();
    plug.keyboard();
    let mouse = plug.medius(1);
    let s = settle(&started(&plug, None), |s| s.connected);
    assert_eq!(s.key.as_deref(), Some(mouse.key.as_str()));
    assert_eq!(s.boxes.len(), 2);
}

#[test]
fn the_remembered_box_is_taken_over_another() {
    let plug = Plug::default();
    plug.medius(1);
    let mine = plug.medius(2);
    let s = settle(&started(&plug, Some(mine.saved())), |s| s.connected);
    assert_eq!(s.key.as_deref(), Some(mine.key.as_str()));
}

#[test]
fn a_missing_remembered_box_falls_back_and_stays_remembered() {
    let plug = Plug::default();
    let other = plug.medius(1);
    let gone = SavedBox {
        key: "MAC 00:00:00:00:00:00".into(),
        family: "Medius".into(),
        port: "COM77".into(),
        ferrum_api: None,
    };
    let s = settle(&started(&plug, Some(gone.clone())), |s| s.connected);
    assert_eq!(s.key.as_deref(), Some(other.key.as_str()));
    assert_eq!(s.remember, Some(gone));
    assert!(s.notice.unwrap().contains("for now"));
}

#[test]
fn a_remembered_box_another_program_holds_is_not_replaced() {
    let plug = Plug::default();
    plug.medius(1);
    let mine = plug.medius(2);
    plug.held_elsewhere.locked().push(mine.key.clone());
    let s = started(&plug, Some(mine.saved())).snapshot();
    assert!(!s.connected);
    assert!(s.error.unwrap().contains("another program"));
}

#[test]
fn several_boxes_wait_for_a_pick_and_the_pick_is_remembered() {
    let plug = Plug::default();
    plug.medius(1);
    let second = plug.medius(2);
    let link = started(&plug, None);
    let s = link.snapshot();
    assert!(s.picking && !s.connected);
    assert!(s.remember.is_none());
    link.send(Command::Pick(second.key.clone()));
    let s = settle(&link, |s| s.connected && s.pending_commands == 0);
    assert_eq!(s.key.as_deref(), Some(second.key.as_str()));
    assert_eq!(s.remember, Some(second.saved()));
    assert!(!s.picking);
}

#[test]
fn a_pick_that_isnt_listed_changes_nothing() {
    let plug = Plug::default();
    let only = plug.medius(1);
    let link = started(&plug, None);
    settle(&link, |s| s.connected);
    link.send(Command::Pick("MAC nothing".into()));
    let s = settle(&link, |s| s.notice.is_some());
    assert_eq!(s.key.as_deref(), Some(only.key.as_str()));
}

#[test]
fn switching_boxes_ends_the_old_one_for_good() {
    let plug = Plug::default();
    let first = plug.medius(1);
    let second = plug.medius(2);
    let link = started(&plug, Some(first.saved()));
    let before = settle(&link, |s| s.connected && !s.kind.is_empty());
    let port = link.injection_port().unwrap();
    link.send(Command::Pick(second.key.clone()));
    let after = settle(&link, |s| {
        s.key.as_deref() == Some(second.key.as_str()) && s.pending_commands == 0
    });
    assert!(after.generation > before.generation);
    assert!(after.discontinuities > before.discontinuities);
    assert!(matches!(port.state(), PortState::Broken(_)));
    // the new box is the remembered one, and the list still has both
    assert_eq!(after.remember, Some(second.saved()));
    assert_eq!(after.boxes.len(), 2);
}

#[test]
fn a_box_plugged_in_later_connects_by_itself() {
    let plug = Plug::default();
    let link = started(&plug, None);
    assert!(!link.snapshot().connected);
    let late = plug.medius(1);
    let s = settle(&link, |s| s.connected);
    assert_eq!(s.key.as_deref(), Some(late.key.as_str()));
}

#[test]
fn a_box_plugged_in_while_connected_is_listed_without_a_refresh() {
    let plug = Plug::default();
    let mine = plug.medius(1);
    let link = started(&plug, None);
    settle(&link, |s| s.connected);
    let other = plug.medius(2);
    let s = settle(&link, |s| s.boxes.len() == 2);
    assert!(s.boxes.iter().any(|row| row.key == other.key));
    assert_eq!(s.key.as_deref(), Some(mine.key.as_str()));
}

#[test]
fn a_box_let_go_on_purpose_stays_let_go() {
    let plug = Plug::default();
    plug.medius(1);
    let link = started(&plug, None);
    settle(&link, |s| s.connected);
    link.send(Command::Disconnect);
    settle(&link, |s| !s.connected && s.pending_commands == 0);
    let opens = plug.opens.load(Ordering::Relaxed);
    plug.medius(2);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!link.snapshot().connected);
    assert_eq!(plug.opens.load(Ordering::Relaxed), opens);
}

#[test]
fn a_refresh_keeps_the_connected_box_and_connects_nothing() {
    let plug = Plug::default();
    let mine = plug.medius(1);
    let link = started(&plug, None);
    settle(&link, |s| s.connected);
    // a scan sees the port this app holds as busy
    plug.held_elsewhere.locked().push(mine.key.clone());
    let other = plug.medius(2);
    let opens = plug.opens.load(Ordering::Relaxed);
    link.send(Command::Refresh);
    let s = settle(&link, |s| s.boxes.len() == 2 && s.pending_commands == 0);
    assert_eq!(s.key.as_deref(), Some(mine.key.as_str()));
    assert!(s.boxes.iter().any(|row| row.key == other.key));
    assert!(s.unusable.is_empty());
    assert_eq!(plug.opens.load(Ordering::Relaxed), opens);
}

#[test]
fn a_remembered_box_on_a_new_port_is_remembered_there() {
    let plug = Plug::default();
    let mine = plug.medius(1);
    let mut moved = mine.saved();
    moved.port = "COM3".into();
    let s = settle(&started(&plug, Some(moved)), |s| s.connected);
    assert_eq!(s.remember, Some(mine.saved()));
}

#[test]
fn a_failed_search_still_tries_the_remembered_box() {
    let plug = Plug::default();
    let mine = plug.medius(1);
    *plug.broken.locked() = Some(Unusable {
        port: "COM5".into(),
        why: "no box answered on COM5".into(),
    });
    let s = settle(&started(&plug, Some(mine.saved())), |s| s.connected);
    assert_eq!(s.key.as_deref(), Some(mine.key.as_str()));
}

#[test]
fn a_box_unplugged_while_waiting_for_a_pick_leaves_the_other() {
    let plug = Plug::default();
    let stays = plug.medius(1);
    let goes = plug.medius(2);
    let link = started(&plug, None);
    assert!(link.snapshot().picking);
    let _ = plug.unplug(&goes.key);
    let s = settle(&link, |s| s.connected);
    assert_eq!(s.key.as_deref(), Some(stays.key.as_str()));
    assert_eq!(s.boxes.len(), 1);
}

#[test]
fn picking_the_fallback_keeps_it_from_now_on() {
    let plug = Plug::default();
    let other = plug.medius(1);
    let gone = SavedBox {
        key: "MAC 00:00:00:00:00:00".into(),
        family: "Medius".into(),
        port: "COM77".into(),
        ferrum_api: None,
    };
    let link = started(&plug, Some(gone));
    settle(&link, |s| s.connected);
    let opens = plug.opens.load(Ordering::Relaxed);
    link.send(Command::Pick(other.key.clone()));
    let s = settle(&link, |s| s.remember == Some(other.saved()));
    assert_eq!(s.remember, Some(other.saved()));
    assert_eq!(plug.opens.load(Ordering::Relaxed), opens);
}

#[test]
fn a_port_a_refresh_cant_use_says_why_and_keeps_the_box() {
    let plug = Plug::default();
    let mine = plug.medius(1);
    let link = started(&plug, None);
    settle(&link, |s| s.connected);
    *plug.broken.locked() = Some(Unusable {
        port: "COM5".into(),
        why: "no box answered on COM5".into(),
    });
    link.send(Command::Refresh);
    let s = settle(&link, |s| !s.unusable.is_empty());
    assert!(s.unusable[0].why.contains("no box answered"));
    assert_eq!(s.key.as_deref(), Some(mine.key.as_str()));
    assert!(s.error.is_none());
}

#[test]
fn a_box_shows_connected_only_once_it_has_been_read() {
    let plug = Plug::default();
    plug.medius(1);
    let link = Link::with_boxes(Box::new(plug.clone()));
    link.send(Command::Start(None));
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        let s = link.snapshot();
        if s.connected {
            assert!(!s.name.is_empty() && !s.kind.is_empty());
            return;
        }
    }
    panic!("never connected");
}

#[test]
fn a_port_that_doesnt_answer_with_no_box_says_why() {
    let plug = Plug::default();
    *plug.broken.locked() = Some(Unusable {
        port: "COM5".into(),
        why: "no box answered on COM5".into(),
    });
    let s = started(&plug, None).snapshot();
    assert!(!s.picking && !s.connected);
    assert!(s.boxes.is_empty());
    assert_eq!(s.unusable.len(), 1);
    assert!(s.unusable[0].why.contains("no box answered"));
    assert!(s.error.is_none());
}

#[test]
fn the_remembered_box_takes_over_again_once_it_is_back() {
    let plug = Plug::default();
    let stand_in = plug.medius(1);
    let mine = plug.medius(2);
    let away = plug.unplug(&mine.key);
    let link = started(&plug, Some(mine.saved()));
    let s = settle(&link, |s| s.connected);
    assert_eq!(s.key.as_deref(), Some(stand_in.key.as_str()));
    plug.plug_back(away);
    let s = settle(&link, |s| {
        s.key.as_deref() == Some(mine.key.as_str()) && s.connected && s.pending_commands == 0
    });
    assert_eq!(s.key.as_deref(), Some(mine.key.as_str()));
    assert_eq!(s.remember, Some(mine.saved()));
}

#[test]
fn a_box_in_use_isnt_swapped_until_it_is_free() {
    let plug = Plug::default();
    let stand_in = plug.medius(1);
    let mine = plug.medius(2);
    let away = plug.unplug(&mine.key);
    let link = started(&plug, Some(mine.saved()));
    settle(&link, |s| s.connected);
    link.hold_still(true);
    plug.plug_back(away);
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(link.snapshot().key.as_deref(), Some(stand_in.key.as_str()));
    link.hold_still(false);
    let s = settle(&link, |s| {
        s.key.as_deref() == Some(mine.key.as_str()) && s.connected
    });
    assert_eq!(s.key.as_deref(), Some(mine.key.as_str()));
}
