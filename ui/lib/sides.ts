import type { Case, Purpose, Run, Side, VisionConfig } from "@/types";

/** each comparison, and what it calls its two sides */
export const cases: Record<Case, { name: string; sides: Record<Side, string> }> = {
  assist: {
    name: "Native against an assist program",
    sides: { baseline: "Without program", candidate: "With program" },
  },
  box: {
    name: "Native against a box",
    sides: { baseline: "Direct", candidate: "Through the box" },
  },
  boxes: {
    name: "One box against another",
    sides: { baseline: "First box", candidate: "Second box" },
  },
  renderer: {
    name: "Medius with and without the renderer",
    sides: { baseline: "Renderer off", candidate: "Renderer on" },
  },
};

export const isCase = (purpose: Purpose | null | undefined): purpose is Case =>
  !!purpose && Object.hasOwn(cases, purpose);

/** the recordings tagged for `kind` that a comparison can read */
export function tagged(runs: Run[], kind: Case) {
  return runs.filter((run) => run.tag?.case === kind && !run.refusals.comparison);
}

export function sideCounts(tagged: Run[]): Record<Side, number> {
  const counts = { baseline: 0, candidate: 0 };
  for (const run of tagged) counts[run.tag!.side]++;
  return counts;
}

/** the lowest p a comparison with this many recordings a side can give, worked out the way native
 * does. below 2 a side there's none worth showing */
export function lowestP(a: number, b: number) {
  if (a < 2 || b < 2) return null;
  const n = a + b;
  const k = Math.min(a, b);
  let assignments = 1;
  for (let i = 0; i < k; i++) assignments = (assignments * (n - i)) / (i + 1);
  return (a === b ? 2 : 1) / assignments;
}

/** the side with fewer recordings, or the other one than last time when they're even */
export function nextSide(tagged: Run[]): Side {
  const counts = sideCounts(tagged);
  if (counts.baseline !== counts.candidate)
    return counts.baseline < counts.candidate ? "baseline" : "candidate";
  const last = tagged.reduce<Run | null>(
    (latest, run) => (!latest || (run.started_us ?? 0) > (latest.started_us ?? 0) ? run : latest),
    null,
  );
  return last?.tag?.side === "baseline" ? "candidate" : "baseline";
}

const compared = {
  yaw: "yaw",
  pitch: "pitch",
  fov: "FOV",
  fov_axis: "FOV axis",
  sensitivity: "sensitivity",
  scenario: "scenario",
} as const;

/** what's set differently now than in the tagged recordings, and how many of them had no target
 * capture to tell from */
export function setupDifferences(tagged: Run[], config: VisionConfig) {
  const differs = new Set<string>();
  let unknown = 0;
  for (const run of tagged) {
    if (!run.vision) {
      unknown++;
      continue;
    }
    for (const key of Object.keys(compared) as (keyof typeof compared)[]) {
      const [then, now] = [run.vision[key], config[key]];
      const same = typeof then === "string" ? then.trim() === String(now).trim() : then === now;
      if (!same) differs.add(compared[key]);
    }
  }
  return { differs: [...differs], unknown };
}

/** what differs between the tagged recordings themselves, against the first one with target capture */
export function recordedDifferences(tagged: Run[]) {
  const first = tagged.find((run) => run.vision);
  return first ? setupDifferences(tagged, first.vision!).differs : [];
}

/** "a, b and c" */
export const listed = (items: string[]) =>
  items.length < 2 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
