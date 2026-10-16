import { describe, expect, it } from "vite-plus/test";
import { crosshairAt, frameAt, trail } from "@/lib/replay";
import type { Replay } from "@/types";

// a 90 degree screen where one count turns the camera a tenth of a degree, still for 100 ms,
// then turned 100 counts right, 10 degrees, by 200 ms
const replay: Replay = {
  width: 2000,
  height: 1000,
  focal: 1000,
  degrees: [0.1, 0.1],
  starting_pitch: 0,
  from_us: 0,
  mouse: Array.from({ length: 300 }, (_, i): [number, number] => [
    Math.max(0, Math.min(100, i - 100)),
    0,
  ]),
  clicks: [],
  frames: [
    { at_ms: 50, boxes: [] },
    { at_ms: 150, boxes: [[0, 0, 10, 10]] },
  ],
};

describe("replay", () => {
  it("puts the crosshair of now in the middle of the screen", () => {
    expect(crosshairAt(replay, 250, 250)).toEqual([1000, 500]);
  });
  it("puts where the crosshair was before a turn right to the left of it", () => {
    const [x, y] = crosshairAt(replay, 0, 250)!;
    // the focal length is half the width at 90 degrees, so 10 degrees is 1000 tan 10 pixels
    expect(x).toBeCloseTo(1000 - 1000 * Math.tan((10 * Math.PI) / 180), 6);
    expect(y).toBeCloseTo(500, 6);
  });
  it("shows the last frame at or before now", () => {
    expect(frameAt(replay, 10)).toBeNull();
    expect(frameAt(replay, 149)?.at_ms).toBe(50);
    expect(frameAt(replay, 150)?.at_ms).toBe(150);
  });
  it("draws the trail as one path while it stays in front of the camera", () => {
    const path = trail(replay, 250, 0.2);
    expect(path.startsWith("M")).toBe(true);
    expect(path.match(/M/g)).toHaveLength(1);
  });
});
