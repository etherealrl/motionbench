import { useState } from "react";
import { Items } from "@/components/inline";
import { Summary } from "@/components/summary";
import { Switch } from "@/components/ui/switch";
import { TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ResizableTable } from "@/components/resizable-table";
import { formatUs } from "@/lib/plots";
import type { Movement as MovementData, Plot, Speed } from "@/types";
import {
  axisLines,
  Canvas,
  Chart,
  EachRecording,
  Unavailable,
  colors,
  grid,
  timeAxis,
  useRecordingColor,
  ink,
} from "./chart";
import { percent, shares, sum } from "./marks";

const noDecoder = "unavailable, there's no decoder for this mouse so only its timing shows";

/** the largest count a lane draws, so a baseline's lane can share the candidate's scale */
function peak(movement: MovementData | null | undefined, signed: boolean) {
  return Math.max(
    1,
    ...(movement?.motion ?? []).flatMap((v) =>
      signed ? [Math.abs(v[1]), Math.abs(v[2])] : [v[3]],
    ),
  );
}

function CountsOverTime({
  plot,
  movement,
  signed,
  max,
  hover,
  onHover,
}: {
  plot: Plot;
  movement: MovementData;
  signed: boolean;
  max: number;
  hover: number | null;
  onHover: (v: number | null) => void;
}) {
  const duration = Math.max(0.001, (plot.to_us - plot.from_us) / 1e6);
  const color = useRecordingColor();
  return (
    <Canvas
      height={160}
      aria-label="Movement counts over time"
      onPointerMove={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        const scale = r.width / e.currentTarget.clientWidth;
        onHover(
          Math.max(0, Math.min(1, (e.clientX - r.left - 60 * scale) / (r.width - 72 * scale))) *
            duration,
        );
      }}
      onPointerLeave={() => onHover(null)}
      draw={(ctx, w, h) => {
        const lo = signed ? -max : 0;
        grid(ctx, w, h, lo, max, "");
        // signed, dx solid and dy dashed; otherwise the magnitude alone
        axisLines(ctx, color, signed ? [1, 2] : [3], (axis) => {
          let pause = 0;
          movement.motion.forEach((v, i) => {
            const x = 60 + (v[0] / duration) * (w - 72);
            const y = 40 + (1 - (v[axis] - lo) / (max - lo)) * (h - 65);
            // the line stops across a pause over 50 ms in the reports, nowhere else
            let broken = !i;
            while (pause < movement.pauses.length && movement.pauses[pause][1] <= v[0]) {
              broken ||= movement.pauses[pause][0] >= movement.motion[i - 1]?.[0];
              pause++;
            }
            if (broken) ctx.moveTo(x, y);
            else ctx.lineTo(x, y);
          });
        });
        timeAxis(ctx, w, h, duration);
        if (hover !== null) {
          const x = 60 + (hover / duration) * (w - 72);
          ctx.strokeStyle = ink(colors.cursor);
          ctx.beginPath();
          ctx.moveTo(x, 20);
          ctx.lineTo(x, h - 20);
          ctx.stroke();
        }
      }}
    />
  );
}

function CountsLane({
  plot,
  signed,
  max,
  hover,
  onHover,
}: {
  plot: Plot;
  signed: boolean;
  max: number;
  hover: number | null;
  onHover: (v: number | null) => void;
}) {
  const movement = plot.tabs.movement;
  return (
    <Chart
      className="chart-wide"
      title="Counts over time"
      about="the mouse's counts per report before Windows sensitivity, on the same time axis as the timeline, either the magnitude or dx solid and dy dashed. it's a preview that keeps the extremes of each stretch, though every number is still computed from all the reports. a baseline's lane uses this one's scale"
    >
      {movement ? (
        <CountsOverTime
          plot={plot}
          movement={movement}
          signed={signed}
          max={max}
          hover={hover}
          onHover={onHover}
        />
      ) : (
        !plot.movement_available && <Unavailable>{noDecoder}</Unavailable>
      )}
    </Chart>
  );
}

const countLabels = [...Array.from({ length: 11 }, (_, i) => `${i}`), ">10"];

function lag(value: number | null) {
  return value === null ? "-" : value.toFixed(2);
}

/** reports that moved, by the counts each axis carried, on a log share axis so the rare large
 * counts stay visible */
function CountsPerReport({ movement, base }: { movement: MovementData; base?: MovementData }) {
  const total = sum(movement.dx);
  const series = [
    { name: "|dx|", own: shares(movement.dx), theirs: base && shares(base.dx) },
    { name: "|dy|", own: shares(movement.dy), theirs: base && shares(base.dy) },
  ];
  // one decade past the smallest share, so a bin holding a single report still has height
  const decades = Math.max(1, Math.floor(Math.log10(Math.max(total, base ? sum(base.dx) : 0))) + 1);
  const y = (share: number) => (share > 0 ? 20 + (-Math.log10(share) / decades) * 130 : 150);
  const slot = 541 / countLabels.length;
  return (
    <Chart
      title="Counts per report"
      about="the reports that carried movement, by how many counts each axis held, as a share of those reports on a logarithmic axis. this is where smoothing and quantization show, because smoothing spreads counts over more reports and a coarse pipeline piles them onto a few values. 'Next report alike' is how much one report's counts correlate with the next one's over the moving intervals' pairs, from 20 pairs. smoothing pushes it towards 1 but so does a hand that changes speed slowly, so it only compares conditions that were moved alike"
      foot={
        <Items>
          <span>{total.toLocaleString()} moving reports</span>
          <span>
            next report alike: dx {lag(movement.lag1_dx)}, dy {lag(movement.lag1_dy)} over{" "}
            {movement.lag1_pairs.toLocaleString()} pairs
          </span>
          {base && (
            <span>
              baseline {sum(base.dx).toLocaleString()} reports, dx {lag(base.lag1_dx)}, dy{" "}
              {lag(base.lag1_dy)} over {base.lag1_pairs.toLocaleString()} pairs
            </span>
          )}
        </Items>
      }
    >
      <svg
        role="img"
        aria-label="Reports by counts per axis"
        viewBox="0 0 600 180"
        className="summary-chart"
      >
        {[1, 0.1, 0.01, 0.001, 0.0001]
          .filter((s) => -Math.log10(s) <= decades)
          .map((share) => (
            <g key={share}>
              <line x1="44" x2="585" y1={y(share)} y2={y(share)} stroke={colors.grid} />
              <text x="0" y={y(share) + 4}>
                {percent(share)}
              </text>
            </g>
          ))}
        {countLabels.map((label, i) => {
          const left = 44 + i * slot + 4;
          const bar = (slot - 8) / 2;
          return (
            <g key={label}>
              {series.map((s, k) => (
                <g key={s.name}>
                  <rect
                    x={left + k * bar}
                    y={y(s.own[i])}
                    width={bar - 1}
                    height={150 - y(s.own[i])}
                    fill={colors.candidate}
                    opacity={k ? 0.55 : 1}
                  >
                    <title>
                      {s.name} {label}: {percent(s.own[i])}
                    </title>
                  </rect>
                  {s.theirs && s.theirs[i] > 0 && (
                    <rect
                      x={left + k * bar}
                      y={y(s.theirs[i])}
                      width={bar - 1}
                      height={150 - y(s.theirs[i])}
                      fill="none"
                      stroke={colors.baseline}
                      strokeWidth="1.2"
                    />
                  )}
                </g>
              ))}
              <text x={left + slot / 2 - 4} y="168" textAnchor="middle">
                {label}
              </text>
            </g>
          );
        })}
        <text x="585" y="12" textAnchor="end">
          |dx| solid, |dy| light
        </text>
      </svg>
    </Chart>
  );
}

function speedLabel(speed: Speed) {
  if (speed.from === null) return `under ${speed.to}`;
  if (speed.to === null) return `${speed.from} or more`;
  return `${speed.from} to ${speed.to}`;
}

const us = (value: number | null) => (value === null ? "-" : formatUs(value));

/** the moving intervals at each speed of their neighbours, the way to set two conditions side
 * by side when they were not moved alike */
function IntervalBySpeed({ movement, base }: { movement: MovementData; base?: MovementData }) {
  const rows = movement.speed
    .map((own, i) => ({ own, theirs: base?.speed[i] }))
    .filter(({ own, theirs }) => own.intervals || theirs?.intervals);
  return (
    <Chart
      title="Interval by speed"
      about="moving intervals grouped by how fast the reports around them moved, in counts per millisecond, with the median and the longest interval in each, shown from 20 intervals. the longest one grows with the number of intervals, so compare two rows by their medians"
      foot={
        <Items>
          <span>the speed comes from the reports on either side of each interval</span>
          <span>median and longest from 20 intervals</span>
        </Items>
      }
    >
      <div className="burst-table">
        <ResizableTable id="interval-by-speed">
          <TableHeader>
            <TableRow>
              <TableHead>Counts per ms</TableHead>
              <TableHead>Intervals</TableHead>
              <TableHead>Median</TableHead>
              <TableHead>Longest</TableHead>
              {base && (
                <>
                  <TableHead>Baseline intervals</TableHead>
                  <TableHead>Baseline median</TableHead>
                  <TableHead>Baseline longest</TableHead>
                </>
              )}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map(({ own, theirs }) => (
              <TableRow key={speedLabel(own)}>
                <TableCell>{speedLabel(own)}</TableCell>
                <TableCell>{own.intervals.toLocaleString()}</TableCell>
                <TableCell>{us(own.median_us)}</TableCell>
                <TableCell>{us(own.longest_us)}</TableCell>
                {base && (
                  <>
                    <TableCell>{(theirs?.intervals ?? 0).toLocaleString()}</TableCell>
                    <TableCell>{us(theirs?.median_us ?? null)}</TableCell>
                    <TableCell>{us(theirs?.longest_us ?? null)}</TableCell>
                  </>
                )}
              </TableRow>
            ))}
          </TableBody>
        </ResizableTable>
      </div>
    </Chart>
  );
}

function PathView({ movement }: { movement: MovementData }) {
  const color = useRecordingColor();
  return (
    <Canvas
      height={220}
      aria-label="Count-space movement path"
      draw={(ctx, w, h) => {
        const xs = movement.path.map((v) => v[0]);
        const ys = movement.path.map((v) => v[1]);
        const loX = Math.min(0, ...xs),
          hiX = Math.max(1, ...xs),
          loY = Math.min(0, ...ys),
          hiY = Math.max(1, ...ys);
        const scale = Math.min((w - 40) / (hiX - loX), (h - 30) / (hiY - loY));
        ctx.strokeStyle = ink(color);
        ctx.beginPath();
        movement.path.forEach((v, i) => {
          const x = (w - (hiX - loX) * scale) / 2 + (v[0] - loX) * scale;
          const y = (h - (hiY - loY) * scale) / 2 + (v[1] - loY) * scale;
          if (!i) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        });
        ctx.stroke();
      }}
    />
  );
}

/** the Movement tab: what the decoded counts were, and how the intervals behave at each speed */
export function MovementTab({
  plot,
  baseline,
  baselineLabel,
  hover,
  onHover,
}: {
  plot: Plot;
  baseline?: Plot;
  baselineLabel: string;
  hover: number | null;
  onHover: (v: number | null) => void;
}) {
  const [signed, setSigned] = useState(false);
  if (!plot.movement_available) return <Unavailable>{noDecoder}</Unavailable>;
  const movement = plot.tabs.movement;
  const base = baseline?.tabs.movement ?? undefined;
  const max = Math.max(peak(movement, signed), peak(base, signed));
  return (
    <div className="chart-grid">
      <div className="chart-option">
        <Switch id="signed-counts" checked={signed} onCheckedChange={setSigned} />
        <label htmlFor="signed-counts">Signed dx and dy</label>
      </div>
      <EachRecording plot={plot} baseline={baseline} baselineLabel={baselineLabel}>
        {(shown) => (
          <CountsLane plot={shown} signed={signed} max={max} hover={hover} onHover={onHover} />
        )}
      </EachRecording>
      {movement && (
        <>
          <CountsPerReport movement={movement} base={base} />
          <IntervalBySpeed movement={movement} base={base} />
          <details className="advanced-settings chart-wide">
            <Summary>Count-space path</Summary>
            <EachRecording plot={plot} baseline={baseline} baselineLabel={baselineLabel}>
              {(shown) =>
                shown.tabs.movement && (
                  <Chart
                    title="Path"
                    about="the running sum of the counts from the start of the range, X and Y on one scale. it's one recording at a time, and the shape of a path alone doesn't tell you anything about target accuracy"
                  >
                    <PathView movement={shown.tabs.movement} />
                  </Chart>
                )
              }
            </EachRecording>
          </details>
        </>
      )}
    </div>
  );
}
