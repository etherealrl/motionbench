import { describe, expect, it } from "vite-plus/test";
import { targetCircle } from "@/lib/vision";

describe("target preview geometry", () => {
  it("fits a circle inside the detected bounds without stretching", () => {
    expect(targetCircle([100, 200, 220, 280], 2560, 1440)).toEqual({
      x: 160,
      y: 240,
      radius: 40,
    });
    expect(targetCircle([10, 10, 30, 30], 800, 600)).toEqual({
      x: 20,
      y: 20,
      radius: 10,
    });
  });
  it("rejects invalid or off-screen geometry", () => {
    for (const bounds of [
      [1, 2, 0, 4],
      [1, 2, 3, 2],
      [-1, 2, 3, 4],
      [0, 0, 900, 900],
      [0, 0, NaN, 20],
      [1],
    ]) {
      expect(targetCircle(bounds, 800, 600)).toBeNull();
    }
  });
});
