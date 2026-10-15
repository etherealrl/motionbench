use super::*;
#[test]
fn rate_is_independent_of_frame_cadence_and_keeps_signed_fractional_counts() {
    for hz in [60, 144, 360, 1000] {
        let mut linear = Linear::default();
        let mut total = [0i32; 2];
        for _ in 0..hz {
            let step = linear.step(
                [3000.0, -4000.0],
                &Config {
                    speed: 500.0,
                    ..Config::default()
                },
                1.0 / hz as f64,
            );
            for axis in 0..2 {
                total[axis] += step[axis] as i32;
            }
        }
        assert!((total[0] - 300).abs() <= 1);
        assert!((total[1] + 400).abs() <= 1);
    }
}
#[test]
fn target_distance_stalls_and_reset_do_not_create_a_large_catch_up_step() {
    let mut linear = Linear::default();
    let config = Config {
        deadzone_counts: 0.0,
        ..Config::default()
    };
    assert_eq!(linear.step([3.0, -4.0], &config, 0.01), [3, -4]);
    assert_eq!(linear.step([5000.0, 0.0], &config, 5.0), [100, 0]);
    let slow = Config {
        speed: 100.0,
        ..config.clone()
    };
    linear.step([10.0, 0.0], &slow, 0.005);
    linear.reset();
    assert_eq!(linear.step([10.0, 0.0], &slow, 0.005), [0, 0]);
    assert_eq!(linear.step([f64::NAN, 0.0], &config, 0.01), [0, 0]);
}
#[test]
fn deadzone_latches_through_jitter_and_release_clears_fractional_motion() {
    let mut linear = Linear::default();
    let config = Config::default();
    assert_eq!(linear.step([30.0, 0.0], &config, 0.01), [18, 0]);
    for target in [[12.0, 0.0], [-13.0, 0.0], [0.0, 17.0], [-10.0, -10.0]] {
        assert_eq!(linear.step(target, &config, 0.01), [0, 0]);
        assert!(linear.settled());
    }
    assert_eq!(linear.step([-30.0, 0.0], &config, 0.01), [-18, 0]);
    assert!(!linear.settled());
    let slow = Config {
        speed: 100.0,
        ..config.clone()
    };
    assert_eq!(linear.step([30.0, 0.0], &slow, 0.009), [0, 0]);
    assert!(linear.hold([0.0, 0.0], config.deadzone_counts));
    assert_eq!(linear.step([30.0, 0.0], &slow, 0.002), [0, 0]);
    linear.hold([0.0, 0.0], config.deadzone_counts);
    linear.reset();
    assert_eq!(linear.step([16.0, 0.0], &config, 0.01), [4, 0]);
}
