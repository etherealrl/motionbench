from pathlib import Path
import json
import numpy as np
from abcurves.texture import texture_features
from abcurves.judges import trajectory_features, full_system_features, c2st_report

ROOT = Path(__file__).resolve().parents[1] / "fixtures"
rng = np.random.default_rng(31)
n, h, p = 48, 160, 64
future = rng.integers(-5, 6, (n, h, 2)).astype(np.float32)
future[rng.random((n, h)) < 0.25] = 0
prefix = rng.integers(-4, 5, (n, p, 2)).astype(np.float32)
mask = np.ones((n, h), dtype=np.float32)
prefix_mask = np.ones((n, p), dtype=np.float32)
for i in range(n):
    mask[i, h - i % 17:] = 0
targets = rng.uniform(-60, 60, (n, 2)).astype(np.float32)
radii = rng.uniform(4, 16, n).astype(np.float32)
groups = np.asarray([f"trial-{i // 2}" for i in range(n)])
np.savez_compressed(ROOT / "events.npz", future_raw_dxdy=future, future_mask=mask,
    prefix_raw_dxdy=prefix, prefix_mask=prefix_mask, target_rel_x_at_B=targets[:, 0],
    target_rel_y_at_B=targets[:, 1], target_radius=radii, source_trial_id=groups)
streams = [future[i][mask[i] > .5] for i in range(n)]
panels = {
    "texture19": texture_features(future, mask),
    "trajectory14": trajectory_features(streams, list(targets), list(radii)),
    "full49": full_system_features(future, mask, targets, radii, prefix_dxdy=prefix, prefix_mask=prefix_mask),
}
results = {}
for name, real in panels.items():
    fake = real.copy()
    fake[:, 0] += rng.normal(.12, .2, n)
    report = c2st_report(real, fake, groups=groups, repeats=2, bootstrap=20, permutations=8)
    results[name] = {"real": real.tolist(), "fake": fake.tolist(), "groups": [f"str:{g}" for g in groups], "report": report}
(ROOT / "upstream-parity.json").write_text(json.dumps({"revision": "e197441f8d343d3b4a4069c7cae20764d0953d15", "panels": results}, indent=2))
print("wrote native descriptor and grouped judge fixtures")
