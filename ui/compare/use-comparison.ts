import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { sharedScale } from "@/lib/plots";
import { native } from "@/lib/native";
import type { Session } from "@/app/use-session";
import type { Config, Plot, ResultRow, ResultSource, Run, Tab } from "@/types";
import { runPlot } from "@/fixture";

type Range = [number, number] | null;

function savedSelection(key: string): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(value) ? value.filter((id): id is string => typeof id === "string") : [];
  } catch {
    return [];
  }
}
const defaults: Config = {
  seed: 7,
  folds: 5,
  repeats: 5,
  bootstrap: 200,
  permutations: 2000,
  width_ms: 160,
};

// the candidate's plot and, when there is one, the baseline's on shared axes; null
// once `active` turns false, so a superseded request neither asks further nor sets anything
async function loadPlots(
  id: string,
  base: { id: string; input: boolean } | null,
  range: Range,
  active: () => boolean,
): Promise<[Plot, Plot | undefined] | null> {
  const c = await invoke<Plot>("run_plot", { id, range, scale: null });
  if (!active()) return null;
  const b = base ? await invoke<Plot>("run_plot", { ...base, range, scale: null }) : null;
  if (!active()) return null;
  if (!b || !base) return [c, undefined];
  const scale = sharedScale(c, b);
  const [cc, bb] = await Promise.all([
    invoke<Plot>("run_plot", { id, range, scale }),
    invoke<Plot>("run_plot", { ...base, range, scale }),
  ]);
  return active() ? [cc, bb ?? undefined] : null;
}

// the recording a population's charts show: the one chosen if it is still in it, else its first
function chartRun(runs: Run[], ids: string[], chosen: string) {
  return runs.find((r) => r.id === (ids.includes(chosen) ? chosen : ids[0]));
}

function usePlots(
  { snapshot, libraryReady, setError }: Session,
  baseline: string[],
  candidate: string[],
  paired: boolean,
  range: Range,
) {
  const [plot, setPlot] = useState<Plot | null>(null),
    [basePlot, setBasePlot] = useState<Plot | undefined>();
  const [chartCandidate, setChartCandidate] = useState("");
  const [chartBaseline, setChartBaseline] = useState("");
  const selectedRun = chartRun(snapshot.runs, candidate, chartCandidate);
  const baselineRun = chartRun(snapshot.runs, baseline, chartBaseline);
  useEffect(() => {
    if (!libraryReady || !selectedRun) {
      setPlot(null);
      setBasePlot(undefined);
      return;
    }
    if (!native) {
      setPlot(runPlot);
      setBasePlot(baseline.length ? runPlot : undefined);
      return;
    }
    let active = true;
    setPlot(null);
    setBasePlot(undefined);
    const base = paired
      ? { id: selectedRun.id, input: true }
      : baselineRun
        ? { id: baselineRun.id, input: false }
        : null;
    loadPlots(selectedRun.id, base, range, () => active)
      .then((plots) => {
        if (!plots) return;
        setPlot(plots[0]);
        setBasePlot(plots[1]);
      })
      .catch((e) => {
        if (active) setError(String(e));
      });
    return () => {
      active = false;
    };
  }, [
    libraryReady,
    selectedRun?.id,
    selectedRun?.selected,
    selectedRun?.crop?.from_us,
    selectedRun?.crop?.to_us,
    baselineRun?.id,
    baselineRun?.selected,
    baselineRun?.crop?.from_us,
    baselineRun?.crop?.to_us,
    paired,
    range,
  ]);
  return {
    plot,
    basePlot,
    selectedRun,
    baselineRun,
    setChartCandidate,
    setChartBaseline,
  };
}

/** the brushed range, and the chosen recordings' crops as one value an effect can watch. a range
 *  counts from a recording's first kept report, so a new crop puts it away */
function useRange(runs: Run[], ids: string[]) {
  const crops = runs
    .filter((r) => r.crop && ids.includes(r.id))
    .map((r) => `${r.id}:${r.crop?.from_us}:${r.crop?.to_us}`)
    .join();
  // the range remembers the crops it was chosen under, so it is gone in the same render a crop
  // changes and no plot is asked for with it
  const [brushed, setBrushed] = useState<{ range: Range; crops: string }>({
    range: null,
    crops,
  });
  const range = brushed.crops === crops ? brushed.range : null;
  const setRange = (range: Range) => setBrushed({ range, crops });
  return [range, setRange, crops] as const;
}

function panelUsable(runs: Run[], ids: string[], panel: string) {
  return ids.every((id) => runs.find((r) => r.id === id)?.panels.includes(panel));
}

/** the two populations, kept across restarts */
function useSelection({ snapshot, libraryReady }: Session) {
  const [baseline, setBaseline] = useState<string[]>(() => savedSelection("bench-baseline")),
    [candidate, setCandidate] = useState<string[]>(() => savedSelection("bench-candidate"));
  // a selection saved by an earlier session may name recordings the library no longer has
  useEffect(() => {
    if (!native || !libraryReady) return;
    const ids = new Set(snapshot.runs.map((run) => run.id));
    setBaseline((previous) => previous.filter((id) => ids.has(id)));
    setCandidate((previous) => previous.filter((id) => ids.has(id)));
  }, [libraryReady]);
  useEffect(() => {
    localStorage.setItem("bench-baseline", JSON.stringify(baseline));
  }, [baseline]);
  useEffect(() => {
    localStorage.setItem("bench-candidate", JSON.stringify(candidate));
  }, [candidate]);
  return { baseline, setBaseline, candidate, setCandidate };
}

/** the result shown and where it came from, which an export names */
function useResult(call: Session["call"]) {
  const [result, setResult] = useState<ResultRow | null>(null);
  const [resultSource, setResultSource] = useState<ResultSource | null>(null);
  const showResult = (command: string, source: ResultSource) =>
    void call<ResultRow>(command, { id: source.id }).then((value) => {
      if (!value) return;
      setResult(value);
      setResultSource(source);
    });
  const exportResult = async () => {
    const path = await save({
      defaultPath: "comparison.results.json",
      filters: [{ name: "Analysis JSON", extensions: ["json"] }],
    });
    if (path && resultSource) await call("export_result", { path, source: resultSource });
  };
  return { result, setResult, showResult, exportResult };
}

export function useComparison(session: Session) {
  const { snapshot, call } = session;
  const { result, setResult, showResult, exportResult } = useResult(call);
  // the tab Compare shows lives here, so it survives a visit to another view; a new result
  // opens the Comparison tab, where it is
  const [tab, setTab] = useState<Tab>("timing");
  useEffect(() => {
    if (result) setTab("comparison");
  }, [result]);
  const { baseline, setBaseline, candidate, setCandidate } = useSelection(session);
  const [paired, setPaired] = useState(false),
    [panel, setPanel] = useState("texture19");
  const [matching, setMatching] = useState(false);
  const [config, setConfig] = useState(defaults);
  const [range, setRange, crops] = useRange(snapshot.runs, [...baseline, ...candidate]);
  const [fromText, setFromText] = useState(""),
    [toText, setToText] = useState(""),
    [question, setQuestion] = useState("firmware / injection comparison");
  useEffect(() => {
    const job = snapshot.job;
    if (job?.status === "complete") showResult("analysis_result", { kind: "job", id: job.id });
  }, [snapshot.job?.id, snapshot.job?.status, call]);
  const plots = usePlots(session, baseline, candidate, paired, range);
  const usablePanel = (value: string) =>
    panelUsable(snapshot.runs, paired ? candidate : [...baseline, ...candidate], value);
  // a result describes the recordings and settings it ran on; any change puts it away
  useEffect(() => {
    setResult(null);
  }, [range, panel, config, paired, matching, question, baseline.join(), candidate.join(), crops]);
  const analyze = async () => {
    setResult(null);
    await call("analyze", {
      request: {
        baseline,
        candidate,
        paired,
        matching,
        panel,
        config,
        range,
        question,
      },
    });
  };
  // a run from the library, shown on its own with its saved result
  const inspect = (run: Run) => {
    setCandidate([run.id]);
    setPaired(false);
    setRange(null);
    setResult(null);
    if (run.result) showResult("saved_result", { kind: "result", id: run.result.id });
  };
  const showImported = (id: string) => {
    setCandidate([id]);
    setRange(null);
  };
  const forget = (id: string) => {
    setBaseline((ids) => ids.filter((other) => other !== id));
    setCandidate((ids) => ids.filter((other) => other !== id));
    setResult(null);
    setRange(null);
  };
  return {
    ...plots,
    tab,
    setTab,
    result,
    setResult,
    baseline,
    setBaseline,
    candidate,
    setCandidate,
    paired,
    setPaired,
    panel,
    setPanel,
    matching,
    setMatching,
    config,
    setConfig,
    range,
    setRange,
    fromText,
    setFromText,
    toText,
    setToText,
    question,
    setQuestion,
    usablePanel,
    analyze,
    exportResult,
    inspect,
    showImported,
    forget,
  };
}

export type Comparison = ReturnType<typeof useComparison>;
