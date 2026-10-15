import type { Blocker } from "@/types";

// what the user has to do before a control works; a wait passes by itself and says nothing here
export function fixHint(blocker: Blocker | null) {
  return blocker && !blocker.waits ? blocker.message : null;
}

// why a disabled control waits or what fixes it; the busy gate is the user's own action finishing
export function blockerHint(blocker: Blocker | null) {
  return blocker && blocker.need !== "busy" ? blocker.message : null;
}
