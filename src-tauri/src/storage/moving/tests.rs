use super::*;

fn folder(name: &str) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("motion-bench-move-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn recordings_move_with_their_folders_and_nothing_already_there_is_replaced() {
    let from = folder("from");
    let to = folder("to");
    std::fs::write(from.join("a.pcap"), b"capture").unwrap();
    std::fs::write(from.join("library.json"), b"{}").unwrap();
    std::fs::create_dir(from.join("results")).unwrap();
    std::fs::write(from.join("results").join("r.json"), b"result").unwrap();
    std::fs::create_dir(to.join("results")).unwrap();
    std::fs::write(to.join("results").join("old.json"), b"kept").unwrap();
    std::fs::write(to.join("b.pcap"), b"theirs").unwrap();
    std::fs::write(from.join("b.pcap"), b"mine").unwrap();
    std::fs::write(from.join("a.json"), b"sidecar").unwrap();
    std::fs::write(from.join("notes.txt"), b"not ours").unwrap();

    let left = move_recordings(&from, &to).unwrap();

    assert_eq!(std::fs::read(to.join("a.pcap")).unwrap(), b"capture");
    assert_eq!(std::fs::read(to.join("library.json")).unwrap(), b"{}");
    assert_eq!(
        std::fs::read(to.join("results").join("r.json")).unwrap(),
        b"result"
    );
    assert_eq!(
        std::fs::read(to.join("results").join("old.json")).unwrap(),
        b"kept"
    );
    // the name that was taken stays behind, and so does the folder holding it
    assert_eq!(std::fs::read(to.join("b.pcap")).unwrap(), b"theirs");
    assert_eq!(std::fs::read(from.join("b.pcap")).unwrap(), b"mine");
    assert_eq!(left.len(), 1);
    assert!(left[0].contains("b.pcap"));
    assert!(!from.join("a.pcap").exists() && !from.join("results").exists());
    assert_eq!(std::fs::read(to.join("a.json")).unwrap(), b"sidecar");
    // what isn't the app's stays where it is, and isn't news
    assert_eq!(std::fs::read(from.join("notes.txt")).unwrap(), b"not ours");
    assert!(!to.join("notes.txt").exists());

    std::fs::remove_dir_all(&from).unwrap();
    std::fs::remove_dir_all(&to).unwrap();
}

#[test]
fn an_emptied_old_folder_goes_and_one_inside_the_other_stays_put() {
    let from = folder("emptied");
    let to = folder("emptied-to");
    std::fs::write(from.join("a.pcap"), b"capture").unwrap();
    assert!(move_recordings(&from, &to).unwrap().is_empty());
    assert!(!from.exists());

    let inner = to.join("inner");
    std::fs::create_dir(&inner).unwrap();
    assert_eq!(move_recordings(&to, &inner).unwrap().len(), 1);
    assert!(to.join("a.pcap").exists());
    std::fs::remove_dir_all(&to).unwrap();
}

#[test]
fn a_gone_folder_moves_nothing_and_only_a_missing_drive_is_tried_again() {
    let to = folder("unreached-to");
    let gone = std::env::temp_dir().join(format!("motion-bench-move-gone-{}", std::process::id()));
    let left = move_recordings(&gone, &to).unwrap();
    assert!(left[0].contains("none of them moved"), "{left:?}");
    #[cfg(windows)]
    {
        // a drive letter nothing uses stands for a drive that isn't plugged in
        let unplugged = ('D'..='Z')
            .map(|letter| std::path::PathBuf::from(format!("{letter}:\\")))
            .find(|drive| !drive.exists())
            .unwrap();
        assert!(move_recordings(&unplugged.join("recordings"), &to).is_err());
    }
    std::fs::remove_dir_all(&to).unwrap();
}

#[test]
fn a_new_folder_inside_the_old_one_is_still_refused() {
    let from = folder("outer");
    std::fs::write(from.join("a.pcap"), b"capture").unwrap();
    let inner = from.join("not-made-yet");
    assert_eq!(move_recordings(&from, &inner).unwrap().len(), 1);
    assert!(from.join("a.pcap").exists());
    std::fs::remove_dir_all(&from).unwrap();
}

#[cfg(windows)]
#[test]
fn the_same_folder_in_other_letters_moves_nothing() {
    let from = folder("same");
    std::fs::write(from.join("a.pcap"), b"capture").unwrap();
    let shouted = std::path::PathBuf::from(from.to_string_lossy().to_uppercase());
    assert!(move_recordings(&from, &shouted).unwrap().is_empty());
    assert!(from.join("a.pcap").exists());
    std::fs::remove_dir_all(&from).unwrap();
}
