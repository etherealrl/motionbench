import { useState } from "react";
import { Items } from "@/components/inline";
import { brushRange, formatUs } from "@/lib/plots";
import type { Plot } from "@/types";
import { Canvas, Chart, colors, grid, timeAxis, ink } from "./chart";

// the timeline's columns sit on a fixed grid of absolute time; an empty plot has no step and
// spreads its columns evenly over the view
function columnStep(p: Plot) {
  return p.timeline_step_us || (p.to_us - p.from_us) / (p.timeline.length || 1);
}
function columnStart(p: Plot) {
  return p.timeline_step_us ? p.timeline_start_us : p.from_us;
}

function draw(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  plot: Plot,
  hover: number | null,
  start: number | null,
) {
  const duration = Math.max(0.001, (plot.to_us - plot.from_us) / 1e6);
  const [lo, hi] = plot.scale.band;
  grid(ctx, w, h, lo, hi);
  const y = (v: number) => 40 + (1 - (Math.min(hi, Math.max(lo, v)) - lo) / (hi - lo)) * (h - 65);
  // whole device pixels: a thin line at a fractional position fades in and out as the view
  // slides
  const ratio = ctx.getTransform().a || 1;
  const at = (seconds: number) => 60 + (seconds / duration) * (w - 72);
  const span = Math.max(1, plot.to_us - plot.from_us);
  const step = columnStep(plot);
  const first = columnStart(plot);
  // each column fills the whole device pixels up to the next one: thin lines with gaps between
  // them shimmer in waves as the view slides
  const edge = (i: number) => {
    const x = 60 + ((first + i * step - plot.from_us) / span) * (w - 72);
    return Math.round(Math.max(60, x) * ratio) / ratio;
  };
  const median: [number, number][][] = [[]];
  plot.timeline.forEach((v, i) => {
    const left = edge(i);
    const width = Math.max(1 / ratio, edge(i + 1) - left);
    if (!v) {
      ctx.fillStyle = ink(colors.gap);
      ctx.fillRect(left, 40, width, h - 65);
      if (median[median.length - 1].length) median.push([]);
      return;
    }
    if (v[3]) {
      ctx.fillStyle = ink(colors.moving);
      ctx.fillRect(left, 40, width, h - 65);
    }
    const top = y(v[1]);
    const bottom = y(v[0]) + 1;
    ctx.fillStyle = ink(colors.candidate);
    ctx.fillRect(left, top, width, Math.max(1 / ratio, bottom - top));
    median[median.length - 1].push([left + width / 2, y(v[2])]);
    if (v[0] < lo || v[1] > hi) {
      const mid = left + width / 2;
      ctx.fillStyle = ink(colors.outlier);
      ctx.beginPath();
      ctx.moveTo(mid - 2, 25);
      ctx.lineTo(mid + 2, 25);
      ctx.lineTo(mid, 29);
      ctx.fill();
    }
  });
  ctx.strokeStyle = ink(colors.median);
  for (const run of median) {
    ctx.beginPath();
    run.forEach(([x, yy], i) => (i ? ctx.lineTo(x, yy) : ctx.moveTo(x, yy)));
    ctx.stroke();
  }
  if (hover !== null) {
    ctx.strokeStyle = ink(colors.cursor);
    ctx.beginPath();
    ctx.moveTo(at(hover), 12);
    ctx.lineTo(at(hover), h - 20);
    ctx.stroke();
  }
  if (start !== null && hover !== null) {
    ctx.fillStyle = ink(colors.selection);
    ctx.fillRect(
      at(Math.min(start, hover)),
      12,
      (Math.abs(start - hover) / duration) * (w - 72),
      h - 32,
    );
  }
  timeAxis(ctx, w, h, duration);
}

/** report intervals over time: the range picker for every tab. a baseline gets its own lane,
 * never the candidate's time axis */
export function Timeline({
  plot,
  label,
  hover,
  onHover,
  onBrush,
}: {
  plot: Plot;
  label?: string;
  hover: number | null;
  onHover: (value: number | null) => void;
  onBrush: (range: [number, number]) => void;
}) {
  const [start, setStart] = useState<number | null>(null);
  const duration = Math.max(0.001, (plot.to_us - plot.from_us) / 1e6);
  const position = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const scale = rect.width / event.currentTarget.clientWidth;
    return (
      Math.max(
        0,
        Math.min(1, (event.clientX - rect.left - 60 * scale) / (rect.width - 72 * scale)),
      ) * duration
    );
  };
  const bucket =
    hover === null
      ? null
      : plot.timeline[
          Math.max(
            0,
            Math.min(
              plot.timeline.length - 1,
              Math.floor((plot.from_us + hover * 1e6 - columnStart(plot)) / columnStep(plot)),
            ),
          )
        ];
  return (
    <Chart
      className="timeline-lane"
      title={label ? `Report intervals, ${label}` : "Report intervals over time"}
      about="each column holds its shortest to longest interval and the line is its median. shaded columns held a report that carried movement. drag to choose the range every tab shows, and the triangles mark intervals outside the band"
      foot={
        <>
          <Items>
            <span>
              median of all gaps{" "}
              {plot.readouts.median_gap_us === null ? "-" : formatUs(plot.readouts.median_gap_us)}
            </span>
            <span>shaded: observed movement</span>
            <span>blank means no completed reports</span>
          </Items>
          <span>
            {hover === null
              ? ""
              : `${hover.toFixed(3)} s, ${bucket ? `${formatUs(bucket[0])} to ${formatUs(bucket[1])}` : "silence"}`}
          </span>
        </>
      }
    >
      <Canvas
        aria-label={`Report interval timeline${label ? `, ${label}` : ""}. Drag to select a range.`}
        tabIndex={0}
        onPointerMove={(e) => onHover(position(e))}
        onPointerLeave={() => onHover(null)}
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          setStart(position(e));
          e.currentTarget.setPointerCapture(e.pointerId);
        }}
        onPointerUp={(e) => {
          if (e.button !== 0) return;
          if (start !== null) {
            const r = brushRange(start, position(e), duration);
            if (r) onBrush(r);
          }
          setStart(null);
        }}
        onPointerCancel={() => setStart(null)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onBrush([duration * 0.25, duration * 0.75]);
        }}
        draw={(ctx, w, h) => draw(ctx, w, h, plot, hover, start)}
      />
    </Chart>
  );
}
