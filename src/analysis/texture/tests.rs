// the reference arrays are numpy's printout. log1p(1) is ln 2, and clippy would rather a
// literal that close to a constant were the constant. it is data
#![allow(clippy::approx_constant)]
use super::*;
use crate::testkit::fixtures::fixture_ticks;

fn close(got: &[f64], want: &[f64; 19], what: &str) {
    for (j, (g, w)) in got.iter().zip(want).enumerate() {
        let tolerance = 1e-6 * w.abs().max(1.0);
        assert!(
            (g - w).abs() <= tolerance,
            "{what} {}: got {g}, reference {w}",
            NAMES[j]
        );
    }
}

/// reference values printed by abcurves/texture.py (numpy 1.26) over the same fixture, same
/// resample rule, 100 tick windows. window 4 is fully idle and the reference scores it too
#[test]
fn features_match_the_python_reference() {
    let ticks = fixture_ticks();
    assert_eq!(ticks.len(), 841);
    assert_eq!(&ticks[..4], &[[0, 1], [0, 0], [0, 0], [-1, 0]]);
    assert_eq!(ticks.iter().filter(|v| v[0] != 0 || v[1] != 0).count(), 395);

    let windows: Vec<_> = ticks.chunks_exact(100).collect();
    assert_eq!(windows.len(), 8);
    assert!(is_idle(windows[4]));

    let reference: [[f64; 19]; 8] = [
        [
            0.53,
            0.734819876961,
            1.09861228867,
            0.0,
            1.29472716759,
            1.17498526745,
            0.0,
            0.395713870482,
            0.0822820786419,
            0.0938088771574,
            0.414971839352,
            2.5205167655,
            0.88137358702,
            1.17435900562,
            1.44926555993,
            2.2353763433,
            0.248121750369,
            0.331687584509,
            0.345364393936,
        ],
        [
            0.01,
            0.969400557188,
            1.38629436112,
            0.0,
            0.69314718056,
            3.92197333628,
            0.01,
            0.180627322428,
            0.0145521553093,
            0.0475370273475,
            0.651425775379,
            2.31741648821,
            1.17435900562,
            1.18006697477,
            0.0,
            0.69314718056,
            -0.24623694661,
            -0.0496240081168,
            0.317569123974,
        ],
        [
            0.35,
            0.700810053306,
            1.09861228867,
            0.0,
            1.2527629685,
            1.73039052285,
            0.0,
            0.49713545794,
            0.0613172515981,
            0.0866219994038,
            0.351759455021,
            2.19556922583,
            0.88137358702,
            0.996978579764,
            1.03061022536,
            1.94591014906,
            0.488157282672,
            0.588845778802,
            0.291552845968,
        ],
        [
            0.92,
            0.69314718056,
            0.69314718056,
            0.0,
            2.52572864431,
            0.762140052047,
            0.0,
            0.0841496488682,
            0.0634749162633,
            0.255419612207,
            0.596955822661,
            1.47483835917,
            0.69314718056,
            0.69314718056,
            2.77673755226,
            3.6571307558,
            0.0480769230769,
            0.0445402298851,
            0.116255857764,
        ],
        [
            1.0,
            0.0,
            0.0,
            0.0,
            4.61512051684,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            4.61512051684,
            0.0,
            0.0,
            0.0,
        ],
        [
            0.96,
            0.69314718056,
            0.69314718056,
            0.0,
            3.00568260441,
            0.69314718056,
            0.0,
            0.0307361042603,
            0.0866782357996,
            0.240270031001,
            0.621697072238,
            1.04393501463,
            0.69314718056,
            0.69314718056,
            2.93343832976,
            3.85439389259,
            -0.0421052631579,
            -0.0378595993037,
            0.0700381495553,
        ],
        [
            0.67,
            0.69314718056,
            0.69314718056,
            0.0,
            1.27450295513,
            0.819440905884,
            0.0,
            0.127528666791,
            0.0376184152632,
            0.117571708961,
            0.694919849415,
            2.35177798608,
            0.69314718056,
            0.69314718056,
            1.01447202424,
            1.94591014906,
            -0.16798298599,
            0.330353361582,
            0.264473033041,
        ],
        [
            0.02,
            1.16123571187,
            1.60943791243,
            0.0714285714286,
            0.69314718056,
            3.51650822817,
            0.37,
            0.659546950897,
            0.123012932683,
            0.0481095866498,
            0.160964994025,
            3.08392563503,
            1.60943791243,
            1.63376082267,
            0.0,
            0.69314718056,
            0.764142919411,
            0.803854370561,
            0.591979482511,
        ],
    ];
    for (i, window) in windows.iter().enumerate() {
        close(&features(window), &reference[i], &format!("window {i}"));
    }
}

/// w1_table from abcurves/judges.py with windows 0..2 as the real bag and 2..4 as the fake one.
/// two rows a side is degenerate on purpose, the p99 row blowing up to 116 is the reference's
/// own behaviour when the real spread of a feature is tiny, and this has to reproduce it
#[test]
fn w1_matches_the_python_reference() {
    let ticks = fixture_ticks();
    let rows: Vec<[f64; 19]> = ticks.chunks_exact(100).map(features).collect();

    let got = w1(&rows[..2], &rows[2..4]);

    let reference = [
        1.40384615385,
        1.32262895641,
        2.40942083965,
        0.0,
        2.97652399222,
        0.948103145547,
        1.0,
        0.458033977016,
        0.570091836467,
        4.33731756847,
        0.497696672029,
        5.74851833913,
        1.64244291528,
        116.381186116,
        1.62708773525,
        1.73418934462,
        1.08089410635,
        0.921352613162,
        9.17871330357,
    ];
    close(&got, &reference, "w1");
}

#[test]
fn a_bag_against_itself_is_zero_everywhere() {
    let ticks = fixture_ticks();
    let rows: Vec<[f64; 19]> = ticks.chunks_exact(100).map(features).collect();

    assert!(w1(&rows, &rows).iter().all(|v| *v == 0.0));
}

#[test]
fn numpy_corners_are_reproduced() {
    assert_eq!(sign(0.0), 0.0);
    assert_eq!(2.5f64.round_ties_even(), 2.0);
    assert_eq!(
        run_lengths(&[false, false, true, false]),
        (vec![2.0, 1.0], vec![1.0])
    );
    assert_eq!(features(&[]), [0.0; 19]);
    assert_eq!(features(&[[1, 0]; 7])[7..], [0.0; 12]);
}

/// two bags that really differ, 25 tick windows, first half against second. the reference test
/// only holds to 1e-6, this holds every comparison's gaps to the last digit or two
#[test]
fn w1_between_different_bags_stays_put() {
    let ticks = fixture_ticks();
    let rows: Vec<[f64; 19]> = ticks.chunks_exact(25).map(features).collect();
    let half = rows.len() / 2;

    let got = w1(&rows[..half], &rows[half..]);

    let before = [
        0.4509498501581294,
        1.316313750036254,
        1.1148568399446577,
        0.0,
        0.6700621229454049,
        0.48788170732230923,
        12.456344829792192,
        0.0,
        0.5332460322690541,
        0.4040720825925705,
        1.0430081558489275,
        0.5924880630344381,
        1.1303162109078126,
        1.153122812349773,
        0.37430705598198033,
        0.5526472454203,
        0.49368892858932234,
        0.8737299148916128,
        0.5116467068500771,
    ];
    for (j, (g, b)) in got.iter().zip(before).enumerate() {
        assert!((g - b).abs() <= 1e-12 * b.max(1.0), "{}: {g} {b}", NAMES[j]);
    }
}
