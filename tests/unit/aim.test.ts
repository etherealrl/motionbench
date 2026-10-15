import { describe, expect, it } from "vite-plus/test";
import { snapshot as fixture } from "@/fixture";
import type { Blocker, Need, Readiness, Snapshot } from "@/types";
import { aimMessage, aimState, fixedInCapture } from "@/experiment/aim";

type Experiment = NonNullable<Snapshot["experiment"]>;

const blocker = (need: Need, message: string, waits = false): Blocker => ({
  need,
  message,
  waits,
});
const free: Readiness = {
  record: null,
  arm: null,
  recording_button: null,
  clear_recording_button: null,
  aim_button: null,
  box_connect: null,
  box_settings: null,
  screen_capture: null,
};

// native decides what is in the way; the panel only shows it
function snapshot(
  readiness: Partial<Readiness> = {},
  experiment: Partial<Experiment> = {},
): Snapshot {
  const base: Snapshot = structuredClone(fixture);
  return {
    ...base,
    readiness: { ...free, ...readiness },
    experiment: {
      ready: true,
      armed: false,
      active: false,
      learning: false,
      hotkey_mask: 16,
      held_mask: 0,
      binding_seq: 0,
      phase: "Disarmed",
      commands: 0,
      counts: [0, 0],
      target_counts: null,
      recording: false,
      error: null,
      ...experiment,
    } as Experiment,
  };
}
const armed = (readiness: Partial<Readiness> = {}, experiment = {}) =>
  snapshot(readiness, { armed: true, phase: "Hold the hotkey", ...experiment });

describe("the status line", () => {
  it("tells the preview it needs the desktop app", () => {
    expect(aimMessage(aimState(snapshot(), false), 16)).toBe(
      "arming only works in the desktop app",
    );
  });

  it("shows what is in the way of arming first, then of recording", () => {
    const arm = blocker("no_box", "connect a box to arm");
    const record = blocker("capture_stopped", "start monitoring to record");
    expect(aimMessage(aimState(snapshot({ arm, record }), true), 16)).toBe("connect a box to arm");
    expect(aimMessage(aimState(snapshot({ record }), true), 16)).toBe("start monitoring to record");
    expect(aimMessage(aimState(armed({ arm, record }), true), 16)).toBe(
      "start monitoring to record",
    );
  });

  it("names the aim button instead of the controller's hotkey", () => {
    expect(aimMessage(aimState(armed(), true), 16)).toBe("hold Side 2 to aim");
  });

  it("shows a waiting phase as it is and nothing while it moves", () => {
    expect(
      aimMessage(aimState(armed({}, { phase: "Waiting for the game to be in front" }), true), 16),
    ).toBe("Waiting for the game to be in front");
    expect(
      aimMessage(aimState(armed({}, { active: true, phase: "Tracking" }), true), 16),
    ).toBeNull();
  });
});

it("records a condition only while armed with nothing in the way", () => {
  expect(aimState(armed(), true).canRecord).toBe(true);
  expect(aimState(snapshot(), true).canRecord).toBe(false);
  const record = blocker("aim_failed", "the aim controller failed");
  expect(aimState(armed({ record }), true).canRecord).toBe(false);
});

it("links to Capture only for what Capture can fix", () => {
  expect(fixedInCapture(blocker("no_box", "connect"))).toBe(true);
  expect(fixedInCapture(blocker("box_busy", "waiting", true))).toBe(false);
  expect(fixedInCapture(blocker("aim_failed", "disarm and arm again"))).toBe(false);
  expect(fixedInCapture(blocker("recording", "finish recording"))).toBe(false);
  expect(fixedInCapture(blocker("broken", "restart the app"))).toBe(false);
  expect(fixedInCapture(null)).toBe(false);
});
