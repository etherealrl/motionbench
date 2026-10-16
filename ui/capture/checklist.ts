import {
  cases,
  listed,
  lowestP,
  nextSide,
  setupDifferences,
  sideCounts,
  tagged,
} from "@/lib/sides";
import { recordedBox } from "@/runs/sort";
import type { BoxState, Case, MediusState, Run, Side, VisionConfig, VisionStatus } from "@/types";

export type Check = { label: string; state: "done" | "todo" | "note"; hint?: string };

/** what the app knows right now, that the checks read */
export type Live = {
  runs: Run[];
  config: VisionConfig;
  status: VisionStatus | undefined;
  /** the game the projection matches, by name */
  game: string | undefined;
  box: BoxState;
  /** the side the next recording gets */
  side: Side | null;
};

const shownP = (p: number) => Number(p.toPrecision(p >= 0.2 ? 2 : 1));

/** what `kind` needs before each recording, from what the app knows and what the tagged recordings
 * saved */
export function checklist(kind: Case, live: Live): Check[] {
  const sides = tagged(live.runs, kind);
  const specific = {
    assist: assistChecks,
    box: boxChecks,
    boxes: boxesChecks,
    renderer: rendererChecks,
  }[kind](sides, live);
  return [...specific, nextCheck(kind, sides), countCheck(kind, sides)];
}

function nextCheck(kind: Case, sides: Run[]): Check {
  return {
    label: `${cases[kind].sides[nextSide(sides)]} next`,
    state: "note",
    hint:
      "the side with fewer recordings, or the other one than last time when they're even" +
      (kind === "box" ? ". replug the mouse for it, even if it's annoying" : ""),
  };
}

function countCheck(kind: Case, sides: Run[]): Check {
  const counts = sideCounts(sides);
  const names = cases[kind].sides;
  const p = lowestP(counts.baseline, counts.candidate);
  const enough = counts.baseline >= 5 && counts.candidate >= 5;
  return {
    label: "Recordings",
    state: enough ? "done" : "todo",
    hint:
      `${counts.baseline} ${names.baseline.toLowerCase()} and ${counts.candidate} ${names.candidate.toLowerCase()}, ` +
      (p === null
        ? "you need 2 a side before there's any p at all"
        : `so the lowest p you can get is about ${shownP(p)}`) +
      (enough ? "" : ", and 5 a side is a good start"),
  };
}

function targetCheck({ status }: Live): Check {
  const label = "Target capture";
  if (status?.running && status.ready) return { label, state: "done" };
  return {
    label,
    state: "todo",
    hint: "start it in Target capture below, an assist acts on targets so the comparison needs to see them",
  };
}

function scenarioCheck({ config }: Live): Check {
  const scenario = config.scenario.trim();
  if (scenario) return { label: "Scenario", state: "done", hint: scenario };
  return {
    label: "Scenario",
    state: "todo",
    hint: "name the scenario you play, it has to be the same one on both sides",
  };
}

function gameCheck({ config, game }: Live): Check {
  const label = "Game and sensitivity";
  if (game && config.sensitivity > 0)
    return { label, state: "done", hint: `${game}, ${config.sensitivity}` };
  return {
    label,
    state: "todo",
    hint: "pick your game in Projection and type in your sensitivity, exactly what's in the game",
  };
}

function setupCheck(sides: Run[], { config }: Live): Check {
  const label = "Same setup on both sides";
  if (!sides.length) return { label, state: "note", hint: "nothing tagged yet" };
  const { differs, unknown } = setupDifferences(sides, config);
  if (differs.length)
    return {
      label,
      state: "todo",
      hint: `the ${listed(differs)} isn't what some of your recordings had, so set it back or the judges might just see that`,
    };
  if (unknown)
    return {
      label,
      state: "note",
      hint: `${unknown} of your recordings had no target capture, so there's no way to tell for those`,
    };
  return { label, state: "done" };
}

// being connected to the box can't change between the sides, or the judges might see that
function connectionCheck(sides: Run[], { box }: Live): Check {
  const label = "Box connection";
  const now = box.link === "connected";
  if (sides.some((run) => !!recordedBox(run)?.family !== now))
    return {
      label,
      state: "todo",
      hint: `the box is ${now ? "connected" : "disconnected"} now but it wasn't in some of your recordings, so keep it the same on both sides`,
    };
  if (sides.length)
    return {
      label,
      state: "done",
      hint: `${now ? "connected" : "disconnected"} in every recording`,
    };
  return {
    label,
    state: "note",
    hint: "if the program runs on this PC it needs the box's COM port, so leave the box disconnected here on both sides",
  };
}

function assistChecks(sides: Run[], live: Live): Check[] {
  return [
    targetCheck(live),
    scenarioCheck(live),
    gameCheck(live),
    setupCheck(sides, live),
    connectionCheck(sides, live),
  ];
}

type Setting = [name: string, read: (m: Partial<MediusState>) => unknown];
const renderSettings: Setting[] = [
  ["render mode", (m) => m.render?.mode],
  ["Render physical movement", (m) => m.render?.full],
];
const otherSettings: Setting[] = [
  ["emit pace", (m) => m.emit?.mode],
  ["fixed rate", (m) => m.emit?.fixed_hz],
  ["forced rate", (m) => m.emit?.force_hz],
  ["spread", (m) => m.spread?.percent],
  ["riding", (m) => m.riding_ms],
];

/** the Medius settings that aren't the same in all of `runs`, from what each one saved */
function settingsDiffer(runs: Run[], settings: Setting[]) {
  const saved = runs.flatMap((run) => recordedBox(run)?.medius ?? []);
  return settings
    .filter(([, read]) => new Set(saved.map((m) => JSON.stringify(read(m) ?? null))).size > 1)
    .map(([name]) => name);
}

function settingsCheck(runs: Run[], settings: Setting[], label = "Box settings"): Check {
  if (!runs.length) return { label, state: "note", hint: "nothing tagged yet" };
  if (!runs.some((run) => recordedBox(run)?.family))
    return {
      label,
      state: "note",
      hint: "connect the box's COM port while you record so the app can save its settings and check them",
    };
  if (!runs.some((run) => recordedBox(run)?.medius))
    return {
      label,
      state: "note",
      hint: "the app only saves a Medius's settings, so check this box's settings stay the same yourself",
    };
  const differs = settingsDiffer(runs, settings);
  return differs.length
    ? {
        label,
        state: "todo",
        hint: `the ${listed(differs)} isn't the same in all of your recordings, so set it back`,
      }
    : { label, state: "done", hint: "the same in every recording" };
}

function boxChecks(sides: Run[]): Check[] {
  return [
    {
      label: "Mouse endpoint",
      state: "note",
      hint: "for Through the box pick the box's output in Mouse endpoint (moving the mouse helps you find it), and the mouse itself for Direct",
    },
    settingsCheck(
      sides.filter((run) => run.tag?.side === "candidate"),
      [...renderSettings, ...otherSettings],
    ),
  ];
}

function boxPerSideCheck(sides: Run[]): Check {
  const label = "One box per side";
  if (!sides.length) return { label, state: "note", hint: "nothing tagged yet" };
  const keys = (side: Side) =>
    new Set(
      sides
        .filter((run) => run.tag?.side === side)
        .map((run) => recordedBox(run)?.key ?? undefined),
    );
  const [first, second] = [keys("baseline"), keys("candidate")];
  if (first.has(undefined) || second.has(undefined))
    return {
      label,
      state: "note",
      hint: "connect each box's COM port while you record so the app can tell the boxes apart",
    };
  if (first.size > 1 || second.size > 1)
    return {
      label,
      state: "todo",
      hint: "a side has recordings from more than one box, so fix their side in Runs",
    };
  if ([...first].some((key) => second.has(key)))
    return { label, state: "todo", hint: "both sides went through the same box" };
  return { label, state: "done" };
}

function boxesChecks(sides: Run[]): Check[] {
  return [
    boxPerSideCheck(sides),
    {
      label: "Same mouse",
      state: "note",
      hint: "keep the mouse, its cable and the USB port the same, and if the boxes run different firmware say so in the recording names",
    },
    settingsCheck(sides, [...renderSettings, ...otherSettings]),
  ];
}

const rendering = (render: { effective?: string } | undefined) => render?.effective === "rendered";

function mediusCheck(medius: MediusState | null): Check {
  if (!medius)
    return {
      label: "Medius",
      state: "todo",
      hint: "connect the box's control port in Box so the app can see the renderer",
    };
  if (medius.kind !== "mouse")
    return {
      label: "Medius",
      state: "todo",
      hint: `the box cloned your ${medius.kind}, not your mouse, so plug the mouse into it and connect again`,
    };
  return { label: "Medius", state: "done", hint: medius.clone };
}

// the box right now against the side picked for the next recording
function nextRenderCheck(medius: MediusState | null, side: Side | null): Check {
  const label = "Renderer for this recording";
  if (!medius) return { label, state: "note", hint: "once the Medius is connected" };
  if (!side)
    return {
      label,
      state: "todo",
      hint: "pick a side next to Record, then the app checks the renderer matches it",
    };
  const want = side === "candidate";
  if (rendering(medius.render) === want)
    return {
      label,
      state: "done",
      hint: want ? "the box renders your movement" : "the box relays your movement",
    };
  return {
    label,
    state: "todo",
    hint: want
      ? "the next recording is tagged Renderer on but the box relays your movement, so turn on Render physical movement, apply, and move the mouse until Relay only goes away"
      : "the next recording is tagged Renderer off but the box renders your movement, so turn the renderer off and keep every other setting",
  };
}

// what each tagged recording saved against its side
function recordedRenderCheck(sides: Run[]): Check {
  const label = "Recorded renderer";
  const recorded = sides.filter((run) => recordedBox(run)?.medius);
  if (!recorded.length)
    return { label, state: "note", hint: "nothing tagged with the box's settings yet" };
  const wrong = recorded.filter(
    (run) => rendering(recordedBox(run)?.medius?.render) !== (run.tag?.side === "candidate"),
  );
  if (wrong.length)
    return {
      label,
      state: "todo",
      hint: `${wrong.length} of your recordings had the renderer the other way than their side says, so fix their side in Runs or record them again`,
    };
  return { label, state: "done", hint: "every recording matches its side" };
}

function rendererChecks(sides: Run[], live: Live): Check[] {
  const medius = live.box.link === "connected" ? live.box.medius : null;
  return [
    mediusCheck(medius),
    nextRenderCheck(medius, live.side),
    recordedRenderCheck(sides),
    settingsCheck(sides, otherSettings, "Other settings"),
  ];
}
