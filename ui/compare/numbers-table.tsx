import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ResizableTable } from "@/components/resizable-table";
import { Chart } from "@/charts/chart";
import { formatUs } from "@/lib/plots";
import { difference, median, present } from "@/lib/numbers";
import { native } from "@/lib/native";
import type { Session } from "@/app/use-session";
import type { Numbers, Run } from "@/types";
import { numbers as previewNumbers } from "@/fixture";

/** one source recording's figures, and which recording they came from */
interface Column {
  name: string;
  numbers: Numbers;
}
export interface Columns {
  baseline: Column[];
  candidate: Column[];
  /** the baseline is each candidate's own box input */
  paired: boolean;
}

type Key = Exclude<keyof Numbers, "moving">;

const share = (v: number) => `${(v * 100).toFixed(1)}%`;
/** a difference of two shares is in percentage points, not a share of anything */
const points = (v: number) => `${(v * 100).toFixed(1)} pp`;
const rows: {
  label: string;
  key: Key;
  show: (v: number) => string;
  apart?: (v: number) => string;
}[] = [
  {
    label: "Active report rate",
    key: "rate_hz",
    show: (v) => `${Math.round(v).toLocaleString()} Hz`,
  },
  { label: "Period estimate", key: "period_us", show: formatUs },
  {
    label: "Median jitter",
    key: "jitter_us",
    show: (v) => `${v.toFixed(1)} µs`,
  },
  { label: "Longest moving gap", key: "longest_moving_us", show: formatUs },
  { label: "Bunched, under 0.75×", key: "bunched", show: share, apart: points },
  { label: "About 2× or longer", key: "beyond_1x", show: share, apart: points },
  {
    label: "Ticks with no report",
    key: "empty_ticks",
    show: share,
    apart: points,
  },
  {
    label: "Counts after a gap",
    key: "after_gap",
    show: (v) => `${v.toFixed(2)}×`,
  },
  { label: "Next report alike, dx", key: "lag1_dx", show: (v) => v.toFixed(2) },
];

/** every source recording's figures for the selected range, one recording after another; the
 * table empties while they load, so it never shows another selection's figures */
export function useNumbers(
  { snapshot, libraryReady, setError }: Session,
  baseline: string[],
  candidate: string[],
  paired: boolean,
  range: [number, number] | null,
) {
  const [columns, setColumns] = useState<Columns | null>(null);
  const runs = (ids: string[]) =>
    ids.map((id) => snapshot.runs.find((run) => run.id === id)).filter((run): run is Run => !!run);
  const own = runs(candidate);
  // paired, the baseline is the box's own input of each candidate
  const base = paired ? own : runs(baseline);
  // a recording read with another endpoint, decoder or crop has other figures
  const reading = [...base, ...own]
    .map(
      (run) =>
        `${run.id}:${run.selected}:${JSON.stringify(run.decoder)}:${JSON.stringify(run.crop)}`,
    )
    .join();
  useEffect(() => {
    setColumns(null);
    if (!libraryReady || !own.length) return;
    let active = true;
    const load = async (list: Run[], input: boolean) => {
      const out: Column[] = [];
      for (const run of list) {
        if (!active) break;
        out.push({
          name: input ? `${run.name}, box input` : run.name,
          numbers: native
            ? await invoke<Numbers>("run_numbers", {
                id: run.id,
                range,
                input,
              })
            : previewNumbers,
        });
      }
      return out;
    };
    (async () => {
      const b = await load(base, paired);
      const c = await load(own, false);
      if (active) setColumns({ baseline: b, candidate: c, paired });
    })().catch((e) => {
      if (active) setError(String(e));
    });
    return () => {
      active = false;
    };
  }, [libraryReady, reading, paired, range]);
  return columns;
}

/** one column per source recording grouped by condition, then each condition's median and,
 * where both exist, their difference. no colouring: a difference is not a significance */
export function NumbersTable({ columns }: { columns: Columns }) {
  const groups: [string, Column[]][] = [
    [columns.paired ? "Box input" : "Baseline", columns.baseline],
    ["Candidate", columns.candidate],
  ];
  const recordings = (n: number) => `${n} recording${n === 1 ? "" : "s"}`;
  return (
    <Chart
      title="Numbers by recording"
      about="each source recording's figures over the selected range, taken from its moving intervals (two back to back reports that both moved, at most 50 ms apart), then each condition's median over the recordings that have that figure. the difference is the candidate's median minus the baseline's, and a row with one value on each side is just one recording against one, not a population"
      foot={
        <span>
          {columns.paired
            ? `${recordings(columns.candidate.length)}, each against its own box input`
            : `${recordings(columns.baseline.length)} against ${recordings(columns.candidate.length)}`}
        </span>
      }
    >
      <div className="numbers-table">
        <ResizableTable id="comparison-numbers">
          <TableHeader>
            <TableRow>
              <TableHead />
              {groups.map(([group, list]) => (
                <TableHead key={group} colSpan={list.length + 1}>
                  {group}
                </TableHead>
              ))}
              <TableHead />
            </TableRow>
            <TableRow>
              <TableHead />
              {groups.flatMap(([group, list]) => [
                ...list.map((c, i) => <TableHead key={`${group}-${i}`}>{c.name}</TableHead>),
                <TableHead key={`${group}-median`}>Median</TableHead>,
              ])}
              <TableHead>Difference</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map(({ label, key, show, apart = show }) => {
              const values = groups.map(([, list]) => present(list.map((c) => c.numbers[key])));
              const medians = values.map(median);
              const apartText = difference(medians[0], medians[1], apart);
              const single = values.every((v) => v.length === 1);
              return (
                <TableRow key={key}>
                  <TableCell>{label}</TableCell>
                  {groups.flatMap(([group, list], g) => [
                    ...list.map((col, i) => {
                      const v = col.numbers[key];
                      return (
                        <TableCell key={`${group}-${i}`}>{v === null ? "-" : show(v)}</TableCell>
                      );
                    }),
                    <TableCell key={`${group}-median`}>
                      {medians[g] === null ? "-" : show(medians[g])}
                    </TableCell>,
                  ])}
                  <TableCell>
                    {apartText === null
                      ? "-"
                      : single
                        ? `${apartText}, 1 recording each`
                        : apartText}
                  </TableCell>
                </TableRow>
              );
            })}
            <TableRow>
              <TableCell>Moving intervals</TableCell>
              {groups.flatMap(([group, list]) => [
                ...list.map((col, i) => (
                  <TableCell key={`${group}-${i}`}>{col.numbers.moving.toLocaleString()}</TableCell>
                )),
                <TableCell key={`${group}-median`} />,
              ])}
              <TableCell />
            </TableRow>
          </TableBody>
        </ResizableTable>
      </div>
    </Chart>
  );
}
