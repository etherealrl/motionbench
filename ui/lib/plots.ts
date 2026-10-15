import type { Plot, Scale } from "@/types";
/** axes that hold both plots, so a baseline and a candidate share their bins */
export function sharedScale(a: Plot, b: Plot): Scale {
  const lo = Math.min(a.scale.band[0], b.scale.band[0]);
  const hi = Math.max(a.scale.band[1], b.scale.band[1]);
  return {
    band: [lo, Math.max(lo + 1, hi)],
    jitter_us: Math.max(a.scale.jitter_us, b.scale.jitter_us),
  };
}
export function brushRange(start: number, end: number, duration: number): [number, number] | null {
  const lo = Math.max(0, Math.min(start, end, duration));
  const hi = Math.max(0, Math.min(Math.max(start, end), duration));
  return hi - lo < 0.01 ? null : [lo, hi];
}
export function formatUs(value: number) {
  return value >= 1000
    ? `${(value / 1000).toFixed(value >= 10000 ? 1 : 3)} ms`
    : `${Math.round(value)} µs`;
}
export function captureRange(origin: number, range: [number, number]) {
  return {
    from_us: Math.round(origin + range[0] * 1e6),
    to_us: Math.round(origin + range[1] * 1e6),
  };
}
export function hex(value: number | null) {
  return value === null ? "----" : value.toString(16).padStart(4, "0");
}
/** round axis values from `lo` to `hi`: about `count` steps of 1, 2 or 5 times a power of ten */
export function niceTicks(lo: number, hi: number, count = 4) {
  const raw = (hi - lo) / count;
  if (!(raw > 0)) return [lo];
  const power = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * power).find((s) => s >= raw) ?? 10 * power;
  const ticks = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi + step * 1e-9; v += step)
    ticks.push(Number(v.toPrecision(12)));
  return ticks;
}
/** milliseconds with as few digits as the value needs, for log axes */
export function formatMs(us: number) {
  return `${Number((us / 1000).toPrecision(2))} ms`;
}
