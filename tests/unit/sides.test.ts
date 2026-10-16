import { expect, test } from "vite-plus/test";
import {
  lowestP,
  nextSide,
  recordedDifferences,
  setupDifferences,
  sideCounts,
  tagged,
} from "@/lib/sides";
import { checklist, type Live } from "@/capture/checklist";
import { previewSettings } from "@/lib/preview-settings";
import type { BoxState, Case, Run, Side } from "@/types";

const config = { ...previewSettings.vision, scenario: "Gridshot" };
const run = (
  id: string,
  side: Side | null,
  started_us: number,
  extra: Partial<Run> = {},
  kind: Case = "assist",
) =>
  ({
    id,
    tag: side && { case: kind, side },
    started_us,
    refusals: { comparison: null, paired: null },
    vision: config,
    panels: [],
    metadata: {},
    ...extra,
  }) as unknown as Run;

test("the lowest p matches the table in the docs", () => {
  expect(lowestP(1, 5)).toBeNull();
  expect(lowestP(2, 2)).toBeCloseTo(1 / 3);
  expect(lowestP(3, 3)).toBeCloseTo(0.1);
  expect(lowestP(4, 4)).toBeCloseTo(0.0286, 3);
  expect(lowestP(5, 5)).toBeCloseTo(0.0079, 3);
  // uneven sides don't get the swapped labels for free
  expect(lowestP(2, 3)).toBeCloseTo(0.1);
});

test("only recordings tagged for this comparison that it can read count", () => {
  const runs = [
    run("a", "baseline", 1),
    run("b", "candidate", 2),
    run("c", null, 3),
    run("d", "candidate", 4, { refusals: { comparison: "it failed", paired: null } }),
    run("e", "candidate", 5, {}, "box"),
  ];
  expect(sideCounts(tagged(runs, "assist"))).toEqual({ baseline: 1, candidate: 1 });
  expect(sideCounts(tagged(runs, "box"))).toEqual({ baseline: 0, candidate: 1 });
});

test("the next side evens the counts out, then alternates", () => {
  expect(nextSide([])).toBe("baseline");
  expect(nextSide([run("a", "baseline", 1)])).toBe("candidate");
  expect(nextSide([run("a", "baseline", 1), run("b", "candidate", 2)])).toBe("baseline");
  expect(nextSide([run("a", "candidate", 1), run("b", "baseline", 2)])).toBe("candidate");
});

test("a setting that changed since a recording is named, one without target capture is unknown", () => {
  const runs = [
    run("a", "baseline", 1),
    run("b", "candidate", 2, { vision: { ...config, sensitivity: 2, scenario: " Gridshot " } }),
    run("e", "baseline", 5, { vision: { ...config, scenario: "Sixshot" } }),
    run("c", "candidate", 3, { vision: null }),
  ];
  expect(setupDifferences(runs, config)).toEqual({
    differs: ["sensitivity", "scenario"],
    unknown: 1,
  });
  expect(recordedDifferences(runs)).toEqual(["sensitivity", "scenario"]);
});

const noBox = { link: "none", medius: null } as unknown as BoxState;
const live = (runs: Run[], extra: Partial<Live> = {}): Live => ({
  runs,
  config,
  status: undefined,
  game: "Valorant",
  box: noBox,
  side: null,
  ...extra,
});
const medius = (render: string, spread = 0) => ({
  family: "Medius",
  key: "MAC 1",
  medius: {
    kind: "mouse",
    render: { effective: render, mode: "despiked", full: render === "rendered", ready: true },
    emit: { mode: "learned" },
    spread: { percent: spread },
  },
});
const find = (checks: ReturnType<typeof checklist>, label: string) =>
  checks.find((check) => check.label.startsWith(label))!;

test("an assist's box has to be connected the same way on both sides", () => {
  const runs = [run("a", "baseline", 1, { metadata: { box: medius("relay") } })];
  expect(find(checklist("assist", live(runs)), "Box connection").state).toBe("todo");
  expect(find(checklist("assist", live([run("a", "baseline", 1)])), "Box connection").state).toBe(
    "done",
  );
});

test("the renderer's side is checked against what each recording saved", () => {
  const right = run("a", "candidate", 1, { metadata: { box: medius("rendered") } }, "renderer");
  const wrong = run("b", "baseline", 2, { metadata: { box: medius("rendered") } }, "renderer");
  expect(find(checklist("renderer", live([right])), "Recorded renderer").state).toBe("done");
  const checks = checklist("renderer", live([right, wrong]));
  expect(find(checks, "Recorded renderer").hint).toMatch(/^1 of your recordings/);
  // and the box right now against the side picked for the next recording
  const box = { link: "connected", medius: medius("relay").medius } as unknown as BoxState;
  expect(
    find(checklist("renderer", live([], { box, side: "candidate" })), "Renderer for").state,
  ).toBe("todo");
  expect(
    find(checklist("renderer", live([], { box, side: "baseline" })), "Renderer for").state,
  ).toBe("done");
});

test("the renderer's other settings stay the same", () => {
  const runs = [
    run("a", "candidate", 1, { metadata: { box: medius("rendered") } }, "renderer"),
    run("b", "baseline", 2, { metadata: { box: medius("relay", 20) } }, "renderer"),
  ];
  expect(find(checklist("renderer", live(runs)), "Other settings").hint).toMatch(/^the spread/);
});

test("box against box wants one box a side and two different ones", () => {
  const through = (id: string, side: Side, key: string) =>
    run(id, side, 1, { metadata: { box: { ...medius("relay"), key } } }, "boxes");
  const check = (runs: Run[]) => find(checklist("boxes", live(runs)), "One box per side").state;
  expect(check([through("a", "baseline", "A"), through("b", "candidate", "B")])).toBe("done");
  expect(check([through("a", "baseline", "A"), through("b", "candidate", "A")])).toBe("todo");
  expect(check([through("a", "baseline", "A"), run("b", "candidate", 1, {}, "boxes")])).toBe(
    "note",
  );
});

test("a box that isn't a Medius doesn't save its settings, so the app says to check them by hand", () => {
  const makcu = { family: "MAKCU V4", key: "MAKCU 1" };
  const runs = [run("a", "candidate", 1, { metadata: { box: makcu } }, "box")];
  expect(find(checklist("box", live(runs)), "Box settings").hint).toMatch(/only saves a Medius/);
});
