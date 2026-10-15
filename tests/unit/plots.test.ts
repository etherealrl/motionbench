import { describe, expect, it } from "vite-plus/test";
import { brushRange, captureRange, formatMs, niceTicks, sharedScale } from "@/lib/plots";
import { runPlot } from "@/fixture";
import type { Plot } from "@/types";
const plot: Plot = runPlot;
describe("comparison scales", () => {
  it("shares a band and a jitter span wide enough for both recordings", () => {
    expect(
      sharedScale(
        { ...plot, scale: { band: [950, 1050], jitter_us: 12 } },
        { ...plot, scale: { band: [800, 1200], jitter_us: 6 } },
      ),
    ).toEqual({ band: [800, 1200], jitter_us: 12 });
  });
  it("clamps a reversed brush and refuses clicks", () => {
    expect(brushRange(2, -1, 1)).toEqual([0, 1]);
    expect(brushRange(0.4, 0.401, 1)).toBeNull();
  });
  it("sends integer microseconds for fractional brushes on epoch timestamps", () => {
    const selected = captureRange(1790897533000000, [0.3258912, 1.1256789]);
    expect(selected).toEqual({
      from_us: 1790897533325891,
      to_us: 1790897534125679,
    });
    expect(Number.isSafeInteger(selected.from_us)).toBe(true);
    expect(Number.isSafeInteger(selected.to_us)).toBe(true);
  });
});
describe("axis ticks", () => {
  it("steps by 1, 2 or 5 times a power of ten inside the range", () => {
    expect(niceTicks(982, 1017)).toEqual([990, 1000, 1010]);
    expect(niceTicks(0, 0.249)).toEqual([0, 0.1, 0.2]);
    expect(niceTicks(0, 1)).toEqual([0, 0.5, 1]);
    expect(niceTicks(5, 5)).toEqual([5]);
  });
  it("names log axis lengths in milliseconds", () => {
    expect([500, 1000, 20_000].map(formatMs)).toEqual(["0.5 ms", "1 ms", "20 ms"]);
  });
});
