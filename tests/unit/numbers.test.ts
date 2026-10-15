import { describe, expect, it } from "vite-plus/test";
import { difference, median, present } from "@/lib/numbers";

const hz = (v: number) => `${Math.round(v)} Hz`;

describe("numbers by recording", () => {
  it("takes the median over the recordings that have the figure", () => {
    expect(median(present([1000, null, 1200, undefined]))).toBe(1100);
    expect(median(present([3, 1, 2]))).toBe(2);
    expect(median(present([null]))).toBeNull();
  });
  it("shows a difference only when both medians exist, signed as it reads", () => {
    expect(difference(1100, 2000, hz)).toBe("+900 Hz");
    expect(difference(2000, 1100, hz)).toBe("−900 Hz");
    expect(difference(1000, 1000.2, hz)).toBe("0 Hz");
    expect(difference(null, 1000, hz)).toBeNull();
  });
});
