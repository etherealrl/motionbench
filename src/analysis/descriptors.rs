use crate::analysis::numpy::mean;

pub const TRAJECTORY: [&str; 14] = [
    "log_duration",
    "along_endpoint_frac",
    "lateral_endpoint_over_radius",
    "endpoint_to_target_over_radius",
    "endpoint_angle_abs_deg",
    "speed_mean",
    "speed_max",
    "peak_speed_fraction",
    "tail_speed_ratio",
    "decel_tail_slope",
    "reversal_count",
    "jerk_roughness_log1p",
    "straightness",
    "overshoot",
];
pub const TARGET: [&str; 16] = [
    "seam_vector_jump_over_local_speed",
    "seam_speed_log_ratio_abs",
    "seam_direction_change_deg",
    "target_ever_entered",
    "first_target_entry_fraction",
    "target_entry_count",
    "target_exit_count",
    "inside_target_fraction",
    "tail_inside_target_fraction",
    "endpoint_inside_target",
    "endpoint_inside_inner_target",
    "tail_speed_mean",
    "tail_speed_over_mean",
    "tail_path_over_radius",
    "tail_excursion_over_radius",
    "stable_stop_flag",
];
fn norm(v: [f64; 2]) -> f64 {
    v[0].hypot(v[1])
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn vector_mean(v: &[[f64; 2]]) -> [f64; 2] {
    [
        v.iter().map(|v| v[0]).sum::<f64>() / v.len().max(1) as f64,
        v.iter().map(|v| v[1]).sum::<f64>() / v.len().max(1) as f64,
    ]
}
fn path(v: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut p = [0.0; 2];
    v.iter()
        .map(|v| {
            p[0] += v[0];
            p[1] += v[1];
            p
        })
        .collect()
}

pub fn trajectory(v: &[[f64; 2]], target: [f64; 2], radius: f64) -> Vec<f64> {
    if v.len() < 2 {
        return vec![0.0; 14];
    }
    let length = v.len();
    let speed = v.iter().map(|v| norm(*v)).collect::<Vec<_>>();
    let path = path(v);
    let endpoint = *path.last().unwrap();
    let distance = norm(target).max(1.0);
    let r = radius.max(1.0);
    let direction = if norm(target) < 1e-6 {
        [1.0, 0.0]
    } else {
        [target[0] / norm(target), target[1] / norm(target)]
    };
    let lateral = [-direction[1], direction[0]];
    let along = dot(endpoint, direction);
    let lat = dot(endpoint, lateral);
    let peak = speed
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(&a.0)))
        .unwrap()
        .0;
    let segment = &speed[(length as f64 * 0.66) as usize..];
    let mx = (segment.len() - 1) as f64 / 2.0;
    let my = mean(segment);
    let slope = if segment.len() < 2 {
        0.0
    } else {
        segment
            .iter()
            .enumerate()
            .map(|(i, y)| (i as f64 - mx) * (y - my))
            .sum::<f64>()
            / (0..segment.len())
                .map(|i| (i as f64 - mx).powi(2))
                .sum::<f64>()
    };
    let signs = v
        .iter()
        .map(|v| dot(*v, direction))
        .filter(|v| v.abs() > 1e-6)
        .map(f64::signum)
        .collect::<Vec<_>>();
    let jerk = v
        .windows(3)
        .map(|w| {
            norm([
                w[2][0] - 2.0 * w[1][0] + w[0][0],
                w[2][1] - 2.0 * w[1][1] + w[0][1],
            ])
        })
        .collect::<Vec<_>>();
    vec![
        (length as f64).ln(),
        along / distance,
        lat.abs() / r,
        norm(sub(endpoint, target)) / r,
        lat.abs().atan2(along.max(1e-6)).to_degrees().abs(),
        mean(&speed),
        speed.iter().copied().fold(0.0, f64::max),
        peak as f64 / (length - 1) as f64,
        mean(&speed[(length as f64 * 0.75) as usize..]) / mean(&speed).max(1e-6),
        slope,
        signs.windows(2).filter(|w| w[0] != w[1]).count() as f64,
        mean(&jerk).max(0.0).ln_1p(),
        norm(endpoint) / speed.iter().sum::<f64>().max(1e-6),
        (path
            .iter()
            .map(|p| dot(*p, direction))
            .fold(f64::NEG_INFINITY, f64::max)
            - distance)
            .max(0.0)
            / distance,
    ]
}

pub fn target_event(
    v: &[[f64; 2]],
    prefix: &[[f64; 2]],
    target: [f64; 2],
    radius: f64,
) -> Vec<f64> {
    if v.is_empty() {
        return vec![0.0; 16];
    }
    let before = &prefix[prefix.len().saturating_sub(8)..];
    let after = &v[..v.len().min(8)];
    let (jump, ratio, angle) = if before.is_empty() {
        (0.0, 0.0, 0.0)
    } else {
        let b = vector_mean(before);
        let a = vector_mean(after);
        let bn = norm(b);
        let an = norm(a);
        let local = 0.5
            * (mean(&before.iter().map(|v| norm(*v)).collect::<Vec<_>>())
                + mean(&after.iter().map(|v| norm(*v)).collect::<Vec<_>>()));
        (
            norm(sub(a, b)) / local.max(1e-6),
            (an.ln_1p() - bn.ln_1p()).abs(),
            if bn <= 1e-9 && an <= 1e-9 {
                0.0
            } else if bn <= 1e-9 || an <= 1e-9 {
                90.0
            } else {
                (dot(a, b) / (an * bn)).clamp(-1.0, 1.0).acos().to_degrees()
            },
        )
    };
    let p = path(v);
    let r = radius.max(1e-6);
    let distances = p.iter().map(|p| norm(sub(*p, target))).collect::<Vec<_>>();
    let inside = distances.iter().map(|d| *d <= r).collect::<Vec<_>>();
    let at_b = norm(target) <= r;
    let mut previous = at_b;
    let mut entries = 0;
    let mut exits = 0;
    let mut first = None;
    for (i, &current) in inside.iter().enumerate() {
        if current && !previous {
            entries += 1;
            first.get_or_insert(i);
        }
        if !current && previous {
            exits += 1;
        }
        previous = current;
    }
    let tail_start = v.len().saturating_sub(32);
    let speed = v.iter().map(|v| norm(*v)).collect::<Vec<_>>();
    let tail = &speed[tail_start..];
    let endpoint = *p.last().unwrap();
    let anchor = if tail_start == 0 {
        [0.0; 2]
    } else {
        p[tail_start - 1]
    };
    let excursion = std::iter::once(&anchor)
        .chain(p[tail_start..].iter())
        .map(|p| norm(sub(*p, endpoint)))
        .fold(0.0, f64::max);
    let fraction = inside[tail_start..].iter().filter(|v| **v).count() as f64 / tail.len() as f64;
    let endpoint_inside = *inside.last().unwrap();
    vec![
        jump,
        ratio,
        angle,
        if at_b || entries > 0 { 1.0 } else { 0.0 },
        if at_b {
            0.0
        } else {
            first.map_or(1.0, |i| i as f64 / (v.len() - 1).max(1) as f64)
        },
        entries as f64,
        exits as f64,
        inside.iter().filter(|v| **v).count() as f64 / v.len() as f64,
        fraction,
        endpoint_inside as u8 as f64,
        (*distances.last().unwrap() <= r * 0.5) as u8 as f64,
        mean(tail),
        mean(tail) / mean(&speed).max(1e-6),
        tail.iter().sum::<f64>() / r,
        excursion / r,
        (endpoint_inside && fraction >= 0.95 && excursion <= (r * 0.25).max(2.0)) as u8 as f64,
    ]
}
