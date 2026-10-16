use super::*;
use serde_json::json;

fn store(dir: &tempfile::TempDir) -> (Store, PathBuf) {
    let path = dir.path().join("config.json");
    (Store::open(path.clone()), path)
}
fn saved(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn patch(value: Value) -> Patch {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_missing_file_gives_the_defaults_and_is_written_with_the_first_change() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(&dir);
    assert_eq!(store.get(), Settings::default());
    assert!(store.notice().is_none());
    assert!(!path.exists());
    store.update(|s| s.aim.speed = 3100.0).unwrap();
    assert_eq!(saved(&path)["aim"]["speed"], 3100.0);
    assert_eq!(Store::open(path).get().aim.speed, 3100.0);
}

#[test]
fn changes_keep_the_other_sections_and_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(&dir);
    store
        .update(|s| {
            patch(json!({"aim":{"speed":3100,"deadzone_counts":9}})).apply(s);
            s.capture.root = Some("USBPcap2".into());
        })
        .unwrap();
    store
        .update(|s| s.recording.button = Some(Button(8)))
        .unwrap();
    let restarted = Store::open(path).get();
    assert_eq!(restarted, store.get());
    assert_eq!(restarted.aim.speed, 3100.0);
    assert_eq!(restarted.aim.button, Button(8));
    assert_eq!(restarted.capture.root.as_deref(), Some("USBPcap2"));
    assert_eq!(restarted.recording.button, Some(Button(8)));
}

#[test]
fn a_refused_change_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(&dir);
    store.update(|s| s.aim.speed = 3000.0).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    for speed in [99.0, 20_001.0, f64::NAN] {
        assert!(store.update(|s| s.aim.speed = speed).is_err());
    }
    for deadzone in [-1.0, 100.5] {
        assert!(store.update(|s| s.aim.deadzone_counts = deadzone).is_err());
    }
    assert!(store.update(|s| s.vision.fov = 179.0).is_err());
    assert_eq!(store.get().aim.speed, 3000.0);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn the_ui_cannot_patch_bindings_capture_or_unknown_fields() {
    for refused in [
        json!({"recording":{"button":8}}),
        json!({"capture":{"root":"USBPcap1","mouse":null}}),
        json!({"aim":{"speed":2000,"deadzone_counts":12,"button":4}}),
        json!({"aim":{"speed":2000}}),
        json!({"vision":{"stale":1}}),
        json!({"display":{"zoom":1.3,"sections":{"targets":true,"medius":false}}}),
        json!({"display":{"zoom":1.5,"sections":{"targets":"yes","medius":false}}}),
    ] {
        assert!(
            serde_json::from_value::<Patch>(refused.clone()).is_err(),
            "{refused}"
        );
    }
}

#[test]
fn a_saved_value_out_of_shape_goes_back_to_its_default_alone() {
    let mut good = serde_json::to_value(Settings::default()).unwrap();
    good["recording"]["button"] = json!(16);
    good["aim"]["speed"] = json!(3000.0);
    let good_settings: Settings = serde_json::from_value(good.clone()).unwrap();
    let write = |case: &Value| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, case.to_string()).unwrap();
        read(&path)
    };
    for (pointer, value) in [
        ("/recording/button", json!(24)),
        ("/aim/button", json!(0)),
        ("/display/zoom", json!(1.3)),
        ("/capture/mouse", json!({"bus":1})),
        // well shaped but out of the limits
        ("/aim/speed", json!(50)),
        ("/vision/fov", json!(179)),
    ] {
        let mut case = good.clone();
        *case.pointer_mut(pointer).unwrap() = value;
        let Err(Unusable::Corrupt { what, kept, .. }) = write(&case) else {
            panic!("{case}");
        };
        let mut expected = good.clone();
        *expected.pointer_mut(pointer).unwrap() = serde_json::to_value(Settings::default())
            .unwrap()
            .pointer(pointer)
            .unwrap()
            .clone();
        assert_eq!(serde_json::to_value(kept).unwrap(), expected, "{case}");
        assert!(what.contains(&pointer[1..].replace('/', ".")), "{what}");
    }
    // what an update added, dropped or left out is no reason to touch the file
    let mut cases = Vec::new();
    for (pointer, value) in [
        ("/display/sections/medius", json!(1)),
        ("/vision/stale", json!(1)),
        ("/aim/hotkey_mask", json!(8)),
        ("/version", json!(0)),
    ] {
        let mut case = good.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        case.pointer_mut(parent).unwrap()[key] = value;
        cases.push(case);
    }
    let mut missing = good.clone();
    missing["display"].as_object_mut().unwrap().remove("theme");
    cases.push(missing);
    for case in cases {
        assert_eq!(write(&case).unwrap(), Some(good_settings.clone()), "{case}");
    }
}

#[test]
fn settings_from_0_3_0_keep_everything_this_version_still_has() {
    let mut old = serde_json::to_value(Settings::default()).unwrap();
    let vision = old["vision"].as_object_mut().unwrap();
    vision.remove("fov");
    vision.remove("fov_axis");
    vision.insert("horizontal_fov".into(), json!(97.5));
    vision.insert("width".into(), json!(2560));
    vision.insert("height".into(), json!(1440));
    old["aim"]["speed"] = json!(1300.0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, old.to_string()).unwrap();
    let store = Store::open(path);
    assert!(store.notice().is_none());
    let settings = store.get();
    assert_eq!(settings.vision.fov, 97.5);
    assert_eq!(settings.vision.fov_axis, vision::FovAxis::Horizontal);
    assert_eq!(settings.aim.speed, 1300.0);
}

#[test]
fn a_change_that_cannot_be_written_is_kept_until_the_app_closes() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(&dir);
    std::fs::create_dir(dir.path().join("config.json.tmp")).unwrap();
    let kept = store.update(|s| s.aim.speed = 4000.0).unwrap();
    assert_eq!(kept.aim.speed, 4000.0);
    assert_eq!(store.get().aim.speed, 4000.0);
    assert!(store.unsaved().unwrap().contains("could not be saved"));
    assert!(!path.exists());
    std::fs::remove_dir(dir.path().join("config.json.tmp")).unwrap();
    store.update(|s| s.aim.speed = 4100.0).unwrap();
    assert!(store.unsaved().is_none());
    assert_eq!(saved(&path)["aim"]["speed"], 4100.0);
}

fn corpus(group: &str) -> Vec<PathBuf> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/corpus")
        .join(group);
    let mut files: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    assert!(!files.is_empty());
    files
}

#[test]
fn unreadable_or_unsupported_config_is_refused_without_a_write() {
    for case in corpus("config") {
        let bytes = std::fs::read(&case).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, &bytes).unwrap();
        assert!(read(&path).is_err(), "{case:?}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{case:?}");
    }
}

#[test]
fn startup_keeps_a_copy_of_a_corrupt_config_before_resetting_it() {
    for case in corpus("config") {
        let bytes = std::fs::read(&case).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, &bytes).unwrap();
        let store = Store::open(path.clone());
        assert!(store.notice().is_some(), "{case:?}");
        let copy = dir.path().join("config.corrupt-1.json");
        if case.ends_with("version-2.json") {
            // settings from a newer app are left alone and nothing is saved over them
            assert!(!copy.exists());
            assert_eq!(store.get(), Settings::default());
            store.update(|s| s.aim.speed = 3000.0).unwrap();
            assert_eq!(store.get().aim.speed, 3000.0);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        } else if case.ends_with("zoom-out-of-range.json") {
            // only the zoom goes back to its default
            assert_eq!(store.get().display.zoom, Settings::default().display.zoom);
            assert_eq!(store.get().vision.fov, 90.0);
            assert_eq!(std::fs::read(&copy).unwrap(), bytes);
            assert_eq!(read(&path).unwrap(), Some(store.get()));
        } else {
            assert_eq!(store.get(), Settings::default(), "{case:?}");
            assert_eq!(std::fs::read(&copy).unwrap(), bytes, "{case:?}");
            assert!(read(&path).unwrap().is_some(), "{case:?}");
        }
    }
}

#[test]
fn a_corrupt_config_stays_when_its_copy_cannot_be_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, b"broken").unwrap();
    std::fs::create_dir(dir.path().join("config.corrupt-1.json")).unwrap();
    let store = Store::open(path.clone());
    assert!(store.notice().is_some());
    assert_eq!(store.get(), Settings::default());
    store.update(|s| s.aim.speed = 3000.0).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"broken");
}

#[test]
fn a_config_that_cannot_be_read_as_a_file_is_refused_without_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::create_dir(&path).unwrap();
    let store = Store::open(path.clone());
    assert_eq!(store.get(), Settings::default());
    assert!(store.notice().is_some());
    store.update(|s| s.aim.speed = 3000.0).unwrap();
    assert!(path.is_dir());
    assert!(!dir.path().join("config.corrupt-1.json").exists());
}

#[test]
fn startup_goes_on_when_the_reset_config_cannot_be_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, b"broken").unwrap();
    std::fs::create_dir(dir.path().join("config.json.tmp")).unwrap();
    let store = Store::open(path.clone());
    assert_eq!(store.get(), Settings::default());
    assert!(store.notice().unwrap().contains("could not be saved"));
    assert!(store.unsaved().is_some());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    assert_eq!(
        std::fs::read(dir.path().join("config.corrupt-1.json")).unwrap(),
        b"broken"
    );
}

#[test]
fn a_recordings_folder_is_saved_whole_and_a_partial_path_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(&dir);
    assert_eq!(store.get().storage.folder, None);
    let folder = dir.path().join("recordings");
    store
        .update(|s| s.storage.folder = Some(folder.clone()))
        .unwrap();
    assert_eq!(Store::open(path.clone()).get().storage.folder, Some(folder));
    assert!(
        store
            .update(|s| s.storage.folder = Some("recordings".into()))
            .is_err()
    );
    store.update(|s| s.storage.folder = None).unwrap();
    assert_eq!(saved(&path)["storage"]["folder"], Value::Null);
}

#[test]
fn a_sensitivity_is_saved_per_game_and_a_bad_one_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store(&dir);
    store
        .update(|s| {
            s.targets.sensitivities.insert("valorant".into(), 0.35);
        })
        .unwrap();
    assert_eq!(
        Store::open(path).get().targets.sensitivities["valorant"],
        0.35
    );
    for (game, sensitivity) in [("", 1.0), ("cs2", 0.0), ("cs2", f64::NAN)] {
        assert!(
            store
                .update(|s| {
                    s.targets.sensitivities.insert(game.into(), sensitivity);
                })
                .is_err()
        );
    }
    assert_eq!(store.get().targets.sensitivities.len(), 1);
}

#[test]
fn settings_from_0_3_1_with_saved_projections_load_without_a_notice() {
    let mut old = serde_json::to_value(Settings::default()).unwrap();
    old["targets"] = json!({"start_on_launch": false, "projections": [{"name": "CS2"}]});
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, old.to_string()).unwrap();
    let store = Store::open(path);
    assert!(store.notice().is_none());
    assert!(store.get().targets.sensitivities.is_empty());
}

#[test]
fn the_ui_names_every_fov_axis_the_way_native_reads_it() {
    for axis in ["horizontal", "vertical", "horizontal_4x3"] {
        let mut vision = serde_json::to_value(vision::Config::default()).unwrap();
        vision["fov_axis"] = json!(axis);
        let mut settings = Settings::default();
        patch(json!({ "vision": vision })).apply(&mut settings);
        assert_eq!(
            serde_json::to_value(settings.vision.fov_axis).unwrap(),
            axis
        );
    }
}
