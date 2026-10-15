import { expect, it } from "vite-plus/test";
import { recordingTime } from "@/capture/recording";

it("shows the recording time as minutes and seconds", () => {
  expect(recordingTime(0)).toBe("00:00");
  expect(recordingTime(61_999)).toBe("01:01");
  expect(recordingTime(3_600_000)).toBe("60:00");
});
