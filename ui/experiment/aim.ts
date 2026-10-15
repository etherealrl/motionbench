import type { Blocker, Need, Snapshot } from "@/types";
import { buttonName } from "@/lib/mouse-buttons";

// what the Linear aim panel needs to know from one snapshot
export function aimState(snapshot: Snapshot, native: boolean) {
  const status = snapshot.experiment;
  const armed = !!status?.armed;
  const { arm, record } = snapshot.readiness;
  return {
    status,
    armed,
    native,
    arm,
    record,
    // the next thing in the way: arming first, then recording a condition
    waiting: armed ? record : (arm ?? record),
    learning: !!status?.learning,
    recording: !!snapshot.recording,
    canRecord: armed && record === null,
  };
}
export type Aim = ReturnType<typeof aimState>;

// what is in the way, or what the controller is waiting for; the badge already says it moves
export function aimMessage(aim: Aim, mask: number) {
  if (!aim.native) return "arming only works in the desktop app";
  if (aim.waiting) return aim.waiting.message;
  if (!aim.armed || aim.status?.active) return null;
  const phase = aim.status?.phase;
  if (!phase) return null;
  return phase === "Hold the hotkey" ? `hold ${buttonName(mask)} to aim` : phase;
}

// these are fixed in this panel, not in Capture setup
const fixedHere = new Set<Need>(["aim_failed", "recording", "broken"]);

// a wait passes by itself; most other blockers are fixed in Capture setup
export function fixedInCapture(blocker: Blocker | null) {
  return !!blocker && !blocker.waits && !fixedHere.has(blocker.need);
}
