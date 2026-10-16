import type { Replay } from "@/types";

type Vec = [number, number, number];
const dot = (a: Vec, b: Vec) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const radians = (degrees: number) => (degrees * Math.PI) / 180;

/** the camera after `tick` ms of the mouse's counts, the same turn the target events use: the
 * counts across turn it about the vertical, the counts down tilt it down from its starting pitch */
function camera(replay: Replay, tick: number) {
  const last = replay.mouse.length - 1;
  const [x, y] = replay.mouse[Math.max(0, Math.min(last, Math.floor(tick)))] ?? [0, 0];
  const yaw = radians(x * replay.degrees[0]);
  const pitch = radians(replay.starting_pitch - y * replay.degrees[1]);
  return {
    forward: [
      Math.cos(pitch) * Math.sin(yaw),
      Math.sin(pitch),
      Math.cos(pitch) * Math.cos(yaw),
    ] as Vec,
    right: [Math.cos(yaw), 0, -Math.sin(yaw)] as Vec,
    up: [
      -Math.sin(pitch) * Math.sin(yaw),
      Math.cos(pitch),
      -Math.sin(pitch) * Math.cos(yaw),
    ] as Vec,
  };
}

/** where the crosshair pointed at ms `then` shows on screen at ms `now`, in the screen's pixels,
 * or null when it's behind the camera */
export function crosshairAt(replay: Replay, then: number, now: number) {
  const direction = camera(replay, then).forward;
  const view = camera(replay, now);
  const depth = dot(direction, view.forward);
  if (depth <= 1e-6) return null;
  const focal = replay.focal;
  return [
    replay.width / 2 + (focal * dot(direction, view.right)) / depth,
    replay.height / 2 - (focal * dot(direction, view.up)) / depth,
  ] as const;
}

/** the frame on screen at ms `now`, the last one shown at or before it */
export function frameAt(replay: Replay, now: number) {
  const { frames } = replay;
  let lo = 0,
    hi = frames.length - 1,
    found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (frames[mid].at_ms <= now) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found < 0 ? null : frames[found];
}

/** the crosshair's path over the last `seconds` before `now`, as it shows on screen now: an SVG
 * path that breaks where it goes behind the camera */
export function trail(replay: Replay, now: number, seconds: number) {
  const step = 2;
  let path = "";
  let drawing = false;
  for (let then = Math.max(0, now - seconds * 1000); then <= now; then += step) {
    const point = crosshairAt(replay, then, now);
    if (!point) {
      drawing = false;
      continue;
    }
    path += `${drawing ? "L" : "M"}${point[0].toFixed(1)} ${point[1].toFixed(1)}`;
    drawing = true;
  }
  return path;
}
