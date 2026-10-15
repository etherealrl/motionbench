import { createContext, useContext, useEffect, useRef } from "react";
import { InfoTip } from "@/components/info-tip";
import { niceTicks } from "@/lib/plots";
import type { Plot } from "@/types";

/** the chart colors as theme variables. svg takes them as they are, a canvas through `ink` */
export const colors = {
  candidate: "var(--color-chart-candidate)",
  baseline: "var(--color-chart-baseline)",
  grid: "var(--color-chart-grid)",
  text: "var(--color-chart-text)",
  outlier: "var(--color-chart-outlier)",
  moving: "var(--color-chart-moving)",
  gap: "var(--color-chart-gap)",
  median: "var(--color-chart-median)",
  cursor: "var(--color-chart-cursor)",
  selection: "var(--color-chart-selection)",
};

/** a color a canvas can take, a theme variable read as the current theme has it */
export function ink(color: string) {
  const name = /^var\((--[\w-]+)\)$/.exec(color)?.[1];
  return name ? getComputedStyle(document.documentElement).getPropertyValue(name).trim() : color;
}

/** which recording a chart of one recording shows, when there's a baseline to tell it from */
type Shown = { label: string; side: "candidate" | "baseline" };
const ShownContext = createContext<Shown | null>(null);

/** a view of one recording, drawn for the candidate and again for the baseline when there is
 * one. every chart in it says which recording it is and draws in that recording's color */
export function EachRecording({
  plot,
  baseline,
  baselineLabel = "baseline",
  children,
}: {
  plot: Plot;
  baseline?: Plot;
  baselineLabel?: string;
  children: (plot: Plot) => React.ReactNode;
}) {
  if (!baseline) return children(plot);
  return (
    <>
      <ShownContext.Provider value={{ label: "candidate", side: "candidate" }}>
        {children(plot)}
      </ShownContext.Provider>
      <ShownContext.Provider value={{ label: baselineLabel, side: "baseline" }}>
        {children(baseline)}
      </ShownContext.Provider>
    </>
  );
}

/** the color of the recording a chart shows */
export function useRecordingColor() {
  return useContext(ShownContext)?.side === "baseline" ? colors.baseline : colors.candidate;
}

/** a chart card: its title, what it means behind an info button, and a foot that says what it
 * counts. inside `EachRecording` the title says which recording it shows */
export function Chart({
  title,
  about,
  foot,
  children,
  className = "",
}: {
  title: string;
  about?: React.ReactNode;
  foot?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  const shown = useContext(ShownContext);
  if (shown) title = `${title}, ${shown.label}`;
  return (
    <section className={`panel chart ${className}`} data-recording={shown?.side}>
      <div className="panel-heading">
        <div className="flex items-center gap-1">
          <h3>{title}</h3>
          {about && <InfoTip label={`About ${title}`}>{about}</InfoTip>}
        </div>
      </div>
      {children}
      {foot && <div className="chart-foot">{foot}</div>}
    </section>
  );
}

/** a view that has nothing to draw, and why */
export function Unavailable({ children }: { children: React.ReactNode }) {
  return <div className="chart-empty">{children}</div>;
}

export function Canvas({
  draw,
  height = 200,
  ...props
}: {
  draw: (ctx: CanvasRenderingContext2D, width: number, height: number) => void;
  height?: number;
} & React.CanvasHTMLAttributes<HTMLCanvasElement>) {
  const ref = useRef<HTMLCanvasElement>(null);
  const currentDraw = useRef(draw);
  currentDraw.current = draw;
  const schedule = useRef<() => void>(() => {});
  useEffect(() => {
    const canvas = ref.current!;
    const ctx = canvas.getContext("2d")!;
    let frame: number | null = null;
    const render = () => {
      frame = null;
      const width = canvas.clientWidth;
      const ratio =
        (window.devicePixelRatio || 1) * (Number(document.documentElement.style.zoom) || 1);
      const pixelWidth = Math.ceil(width * ratio);
      const pixelHeight = Math.ceil(height * ratio);
      if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
      if (canvas.height !== pixelHeight) canvas.height = pixelHeight;
      ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
      ctx.clearRect(0, 0, width, height);
      currentDraw.current(ctx, width, height);
    };
    const queue = () => {
      if (frame === null) frame = requestAnimationFrame(render);
    };
    schedule.current = queue;
    const observer = new ResizeObserver(queue);
    observer.observe(canvas);
    // a theme switch changes every color, so the chart draws again
    const theme = new MutationObserver(queue);
    theme.observe(document.documentElement, { attributeFilter: ["class"] });
    queue();
    return () => {
      observer.disconnect();
      theme.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
      schedule.current = () => {};
    };
  }, [height]);
  useEffect(() => schedule.current(), [draw]);
  return <canvas ref={ref} style={{ "--canvas-height": `${height}px` }} {...props} />;
}

export function grid(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  lo: number,
  hi: number,
  unit = "µs",
) {
  ctx.font = "11px ui-monospace, monospace";
  ctx.lineWidth = 1;
  // counts are whole numbers, so their axis is too
  const ticks = niceTicks(lo, hi).filter((v) => unit || Number.isInteger(v));
  for (const value of ticks) {
    const y = 40 + (1 - (value - lo) / (hi - lo || 1)) * (height - 65);
    ctx.strokeStyle = ink(colors.grid);
    ctx.beginPath();
    ctx.moveTo(60, y);
    ctx.lineTo(width - 12, y);
    ctx.stroke();
    ctx.fillStyle = ink(colors.text);
    ctx.fillText(`${value} ${unit}`, 0, y + 4);
  }
}

/** one line per axis in `color`, the second axis (dy) dashed. `trace` draws one axis's path */
export function axisLines(
  ctx: CanvasRenderingContext2D,
  color: string,
  axes: number[],
  trace: (axis: number) => void,
) {
  for (const axis of axes) {
    ctx.strokeStyle = ink(color);
    ctx.setLineDash(axis === 2 ? [3, 3] : []);
    ctx.beginPath();
    trace(axis);
    ctx.stroke();
  }
  ctx.setLineDash([]);
}

/** seconds along the bottom of a time chart whose plot area runs from x 60 to 12 short of the
 * right edge, at round values */
export function timeAxis(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  duration: number,
) {
  ctx.font = "11px ui-monospace, monospace";
  ctx.fillStyle = ink(colors.text);
  ctx.textAlign = "center";
  for (const t of niceTicks(0, duration, 8)) {
    const x = 60 + (t / duration) * (width - 72);
    ctx.fillText(`${t} s`, Math.min(x, width - 24), height - 4);
  }
  ctx.textAlign = "start";
}
