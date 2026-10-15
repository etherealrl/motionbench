//! what the comparisons and the tick rule give on the repo's fixtures, pinned, so changing how
//! they're built can't move a number unnoticed

use super::*;
use crate::analysis::views::plot::{Views, plot};
use crate::evidence::reader::recording::open_recording;
use crate::testkit::fixtures::digest;

fn fixture() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    ))
    .into()
}

/// a finished copy of the fixture with the box's input beside it, the wire's own counts on the
/// wire's clock
pub(crate) fn paired_fixture(folder: &Path) -> PathBuf {
    let path = crate::testkit::fixtures::recorded(
        folder,
        "mouse-1000hz.pcap",
        &serde_json::from_slice(&std::fs::read(fixture().with_extension("json")).unwrap()).unwrap(),
    );
    let source = std::fs::read(&path).unwrap();
    let mut text = String::from("# paired_shift_us=0\nbox_us,dx,dy,dz\n");
    crate::capture::pcap::read_pcap(&source[..], |record| {
        if let crate::capture::pcap::Record::Packet {
            event: Some(crate::capture::pcap::Event::Report(report)),
            ..
        } = record
        {
            let (x, y) = crate::capture::pcap::motion(report.bytes()).unwrap();
            text.push_str(&format!("{},{x},{y},0\n", report.t_us));
        }
    })
    .unwrap();
    std::fs::write(path.with_extension("input.csv"), text).unwrap();
    path
}

fn quick() -> Config {
    Config {
        bootstrap: 0,
        permutations: 0,
        ..Config::default()
    }
}

/// the numbers of a result, without the judge, which these tiny fixtures can't run
fn numbers(result: &ResultRow) -> String {
    serde_json::json!({
        "w1": result.w1, "real": result.real_features, "fake": result.fake_features,
        "idle": result.idle_dropped, "shift": result.alignment_shift_us,
        "range": result.metadata["selected_range_us"], "counts": result.metadata["counts"],
    })
    .to_string()
}

#[test]
fn the_fixture_ticks_stay_put() {
    let ticks = crate::testkit::fixtures::fixture_ticks();
    let data = open_recording(&fixture(), None).unwrap().into_data();
    let key = data.selected.unwrap();
    let view = data.bus.streams[&key].view(0, u64::MAX);
    let first = data.bus.streams[&key].first_us().unwrap();
    let from = crate::testkit::fixtures::ticks_from(&view, first + 500, 300);
    let (rows, idle) = windows(&view, first + 500, 300, 32, &AtomicBool::new(false)).unwrap();
    let rows: Vec<_> = rows.iter().map(|w| (w.index, &w.row)).collect();
    assert_eq!(
        [
            digest(&format!("{ticks:?}")),
            digest(&format!("{from:?}")),
            digest(&format!("{rows:?} {idle}")),
        ],
        ["16fdd3fcd8006ab8", "4fcda4fa4482e869", "2c971490ab250757"]
    );
}

#[test]
fn the_fixture_comparisons_stay_put() {
    let folder = crate::testkit::traffic::TempDir::new("digests");
    let path = paired_fixture(folder.path());
    let paired = paired(&path, None, quick(), None, &AtomicBool::new(false), |_| {}).unwrap();
    let source = Source {
        path: path.clone(),
        decoder: None,
        endpoint: None,
        crop: None,
    };
    let independent = |range| {
        independent_selected(
            std::slice::from_ref(&source),
            std::slice::from_ref(&source),
            "texture19",
            range,
            Config {
                width_ms: 32,
                ..quick()
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap()
    };
    assert_eq!(
        [
            digest(&numbers(&paired)),
            digest(&numbers(&independent(None))),
            digest(&numbers(&independent(Some([0.05, 0.3])))),
        ],
        ["26d80d09daa17f94", "a46a9e54911c5cf2", "bc1f3f66a5baa1f2"]
    );
}

#[test]
fn the_fixture_plot_stays_put() {
    let data = open_recording(&fixture(), None).unwrap().into_data();
    let shown = plot(&data, None, None, false, Views::All);
    assert_eq!(
        digest(&serde_json::to_string(&shown).unwrap()),
        "068ee877e002f461"
    );
}
