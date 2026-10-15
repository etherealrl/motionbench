use super::*;
use motion_bench::capture::clock::Anchor;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

fn wait(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(start.elapsed() < Duration::from_secs(5), "never settled");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// a router reading a feed the test writes, with a recording binding saved in `folder`; no aim
/// controller exists at all
fn routed(
    folder: &std::path::Path,
) -> (
    Sender<Feed>,
    Arc<Buttons>,
    Arc<hotkey::Controller>,
    Receiver<Anchor>,
) {
    let (controller, requests) = hotkey::Controller::for_test(folder);
    let buttons = Buttons::new(8);
    let (feed, read) = mpsc::channel();
    let (routing, recording) = (buttons.clone(), controller.clone());
    std::thread::spawn(move || routing.route(read, recording, Arc::new(AtomicBool::new(false))));
    (feed, buttons, controller, requests)
}

fn press(feed: &Sender<Feed>, mask: u32) {
    feed.send(Feed::Buttons {
        mask,
        at: Anchor::take(),
    })
    .unwrap();
}

/// the recording button is learned and toggles a recording with no aim controller running
#[test]
fn the_recording_button_works_without_an_aim_controller() {
    let folder = tempfile::tempdir().unwrap();
    let (feed, _buttons, controller, requests) = routed(folder.path());
    feed.send(Feed::Ready).unwrap();
    wait(|| controller.snapshot().ready);
    controller.learn(false);
    press(&feed, 16);
    wait(|| controller.snapshot().hotkey_mask == Some(16));
    assert!(requests.try_recv().is_err());
    press(&feed, 0);
    wait(|| controller.snapshot().held_mask == 0);
    press(&feed, 16);
    let anchor = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(anchor.qpc_after >= anchor.qpc_before);
    // still held, no second press
    press(&feed, 16);
    assert!(requests.recv_timeout(Duration::from_millis(100)).is_err());
}

/// a saved recording button on the default aim button still records while disarmed
#[test]
fn a_saved_recording_button_works_when_disarmed_despite_the_default_aim_mask() {
    let folder = tempfile::tempdir().unwrap();
    let (feed, buttons, controller, requests) = routed(folder.path());
    controller.update(|s| s.hotkey_mask = Some(8));
    feed.send(Feed::Ready).unwrap();
    wait(|| controller.snapshot().ready);
    assert_eq!(buttons.aim().mask, 8);
    assert!(!buttons.aim().armed);
    press(&feed, 8);
    requests.recv_timeout(Duration::from_secs(2)).unwrap();
}

/// a lost feed leaves no button held and the binding unavailable with its reason, and the
/// feed coming back brings it back; the held buttons start from none again
#[test]
fn a_lost_feed_holds_nothing_and_its_return_brings_the_binding_back() {
    let folder = tempfile::tempdir().unwrap();
    let (feed, buttons, controller, _requests) = routed(folder.path());
    feed.send(Feed::Ready).unwrap();
    press(&feed, 8);
    wait(|| buttons.held() == 8);
    feed.send(Feed::Lost("Medius button CATCH disconnected".into()))
        .unwrap();
    wait(|| !buttons.ready());
    assert_eq!(buttons.held(), 0);
    let lost = controller.snapshot();
    assert!(!lost.ready && lost.error.unwrap().contains("disconnected"));
    assert!(buttons.require().unwrap_err().contains("disconnected"));
    #[cfg(feature = "developer-tools")]
    {
        let mut status = experiment::Status::default();
        buttons.status(&mut status);
        assert!(status.error.is_none() && status.buttons_error.unwrap().contains("disconnected"));
    }
    feed.send(Feed::Ready).unwrap();
    wait(|| buttons.ready() && controller.snapshot().ready);
    assert_eq!(buttons.held(), 0);
}

/// the aim button is learned from one press, saved once, never the recording button's, and
/// a press while it learns records nothing
#[cfg(feature = "developer-tools")]
#[test]
fn the_aim_button_is_learned_and_saved_apart_from_the_recording_button() {
    let folder = tempfile::tempdir().unwrap();
    let (feed, buttons, controller, requests) = routed(folder.path());
    controller.update(|s| s.hotkey_mask = Some(16));
    feed.send(Feed::Ready).unwrap();
    wait(|| controller.snapshot().ready);
    buttons.learn_aim(false).unwrap();
    // the recording button is refused as the aim button, and records nothing meanwhile
    press(&feed, 16);
    std::thread::sleep(Duration::from_millis(20));
    assert!(buttons.aim().learning);
    assert!(requests.try_recv().is_err());
    press(&feed, 0);
    press(&feed, 4);
    wait(|| !buttons.aim().learning);
    assert_eq!((buttons.aim().mask, buttons.aim().seq), (4, 1));
    let restarted = crate::config::Store::open(folder.path().join("config.json"));
    assert_eq!(restarted.get().aim.button.mask(), 4);
    // armed, the binding is fixed
    buttons.arm(true);
    assert!(buttons.learn_aim(false).is_err());
}

/// an aim button that could not be written is used until the app closes, the settings say so
#[cfg(feature = "developer-tools")]
#[test]
fn an_aim_button_that_cannot_be_saved_is_kept_for_the_session() {
    let folder = tempfile::tempdir().unwrap();
    // a file where the settings folder should be, so writing them fails
    let blocked = folder.path().join("blocked");
    std::fs::write(&blocked, b"file").unwrap();
    let (feed, buttons, controller, _requests) = routed(&blocked);
    feed.send(Feed::Ready).unwrap();
    wait(|| buttons.ready());
    buttons.learn_aim(false).unwrap();
    press(&feed, 4);
    wait(|| !buttons.aim().learning);
    assert_eq!(buttons.aim().mask, 4);
    assert!(buttons.aim().error.is_none());
    assert_eq!(controller.settings().get().aim.button.mask(), 4);
    assert!(controller.settings().unsaved().is_some());
}

#[test]
fn a_feed_that_ends_with_a_button_held_lets_go_of_it() {
    let folder = tempfile::tempdir().unwrap();
    let (feed, buttons, controller, _requests) = routed(folder.path());
    feed.send(Feed::Ready).unwrap();
    press(&feed, 16);
    wait(|| buttons.held() == 16);
    drop(feed);
    wait(|| !buttons.ready());
    assert_eq!(buttons.held(), 0);
    assert!(buttons.require().is_err());
    assert!(!controller.snapshot().ready);
}
