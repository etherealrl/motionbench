import { Items } from "@/components/inline";
import { TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ResizableTable } from "@/components/resizable-table";
import { formatMs, formatUs, niceTicks } from "@/lib/plots";
import type { Plot, Timing } from "@/types";
import { Canvas, Chart, EachRecording, Unavailable, colors, useRecordingColor, ink } from "./chart";
import { CentredHistogram, Grid, percent, shares, signed, sum } from "./marks";

const noMovement =
  "unavailable, there are no moving intervals so there's no period either. a recording needs a decoder and some movement";

function JitterChart({ timing, base }: { timing: Timing; base?: Timing }) {
  const { jitter } = timing;
  return (
    <Chart
      title="Jitter around the period"
      about="how far each 1x moving interval landed from the period estimate. it's what the host saw, so the host's own scheduling is in there too. these are shares of the 1x intervals, on the same bins as the baseline"
      foot={
        <Items>
          <span>{jitter.intervals.toLocaleString()} intervals at 1×</span>
          <span>{jitter.left_out.toLocaleString()} not at 1×</span>
          <span>
            median distance {jitter.mad_us === null ? "-" : `${jitter.mad_us.toFixed(1)} µs`}
          </span>
          <span>
            {/* under 100 intervals p01 and p99 are the extremes, so they are named as such */}
            {jitter.intervals < 100 ? "from" : "98% within"} {signed(jitter.p01_us)} to{" "}
            {signed(jitter.p99_us)}
          </span>
          {jitter.outside > 0 && (
            <span>
              {jitter.outside.toLocaleString()} beyond ±{jitter.span_us} µs
            </span>
          )}
        </Items>
      }
    >
      <CentredHistogram
        label="Jitter histogram"
        bins={jitter.bins}
        span={jitter.span_us}
        baseline={base?.jitter.bins}
      />
    </Chart>
  );
}

const groupLabels = ["Under 0.75×", "About 1×", "About 2×", "About 3×", "4× or more"];

function Groups({ timing, base }: { timing: Timing; base?: Timing }) {
  const own = shares(timing.groups);
  const theirs = base ? shares(base.groups) : null;
  return (
    <Chart
      title="Interval groups while moving"
      about="moving intervals grouped by how many periods they span. under 0.75 periods is a bunched report, rounding would have hidden it in 1x. a longer interval is a gap we saw, though what caused it isn't known from here"
      foot={<span>{sum(timing.groups).toLocaleString()} moving intervals</span>}
    >
      <div className={`delivery groups ${theirs ? "with-baseline" : ""}`}>
        <div className="delivery-row delivery-head" aria-hidden>
          <span />
          <span />
          <span>Share</span>
          <span>Count</span>
          {theirs && <span>Baseline</span>}
        </div>
        {own.map((share, i) => (
          <div className="delivery-row" key={i}>
            <span>{groupLabels[i]}</span>
            <div className="delivery-track">
              <div style={{ "--share": `${share * 100}%` }} />
              {theirs && <i style={{ "--base": `${theirs[i] * 100}%` }} />}
            </div>
            <strong>{percent(share)}</strong>
            <span>{timing.groups[i].toLocaleString()}</span>
            {theirs && <em>{percent(theirs[i])}</em>}
          </div>
        ))}
      </div>
    </Chart>
  );
}

/** the log axis spans what moves: from the longest length every interval exceeds to the
 * shortest none does */
function longerDomain(lines: [number, number][][]) {
  const all = lines.flat();
  const from = Math.max(50, ...all.filter(([, share]) => share >= 1).map(([us]) => us));
  const to = Math.min(50_000, ...all.filter(([, share]) => share <= 0).map(([us]) => us));
  return [from, Math.max(from * 10, to)];
}

function Longer({ timing, base }: { timing: Timing; base?: Timing }) {
  const total = sum(timing.groups);
  const decades = Math.max(1, Math.log10(Math.max(total, base ? sum(base.groups) : 0)));
  const [from, to] = longerDomain(base ? [timing.longer, base.longer] : [timing.longer]);
  const span = Math.log10(to / from);
  const x = (us: number) => 44 + (Math.log10(us / from) / span) * 541;
  const y = (share: number) => 20 + (-Math.log10(share) / decades) * 130;
  const line = (points: [number, number][]) =>
    points
      .filter(([us, share]) => share > 0 && us >= from && us <= to)
      .map(([us, share], i) => `${i ? "L" : "M"}${x(us)},${y(share)}`)
      .join("");
  const lengths = [100, 200, 500, 1000, 2000, 5000, 10_000, 20_000, 50_000].filter(
    (us) => us >= from && us <= to,
  );
  return (
    <Chart
      title="Long intervals while moving"
      about="the share of moving intervals longer than each length, with both axes logarithmic so a rare long gap stays visible. a pause over 50 ms doesn't count as a moving interval"
      foot={
        <Items>
          <span>{total.toLocaleString()} moving intervals</span>
          <span>across: longer than</span>
          <span>up: share of moving intervals</span>
        </Items>
      }
    >
      <svg
        role="img"
        aria-label="Share of moving intervals longer than each length"
        viewBox="0 0 600 180"
        className="summary-chart"
      >
        <Grid
          values={[1, 0.1, 0.01, 0.001, 0.0001].filter((s) => -Math.log10(s) <= decades)}
          y={y}
          label={percent}
        />
        {base && (
          <path d={line(base.longer)} fill="none" stroke={colors.baseline} strokeWidth="1.5" />
        )}
        <path d={line(timing.longer)} fill="none" stroke={colors.candidate} strokeWidth="2" />
        {lengths.map((us) => (
          <text key={us} x={x(us)} y="176" textAnchor="middle">
            {formatMs(us)}
          </text>
        ))}
      </svg>
    </Chart>
  );
}

const tickLabels = ["No report", "1 report", "2 reports", "3 or more"];

function Ticks({ timing, base }: { timing: Timing; base?: Timing }) {
  const own = shares(timing.ticks);
  const theirs = base ? shares(base.ticks) : null;
  const y = (share: number) => 150 - share * 120;
  return (
    <Chart
      title="Reports per 1 ms tick"
      about="the judge sums reports into 1 ms ticks, so this is what it actually sees. while moving, a tick can hold no report, one, or several, and ticks inside gaps up to 50 ms count too, so a slow or gappy stretch shows up as ticks with no report. a change in cadence moves these shares, which is why it moves Texture19"
      foot={<span>{sum(timing.ticks).toLocaleString()} ticks while moving</span>}
    >
      <svg
        role="img"
        aria-label="Share of 1 ms ticks by reports held"
        viewBox="0 0 600 180"
        className="summary-chart"
      >
        {own.map((share, i) => {
          const left = 60 + i * 135;
          return (
            <g key={i}>
              <rect
                x={left}
                y={y(share)}
                width="100"
                height={150 - y(share)}
                fill={colors.candidate}
              />
              {theirs && (
                <rect
                  x={left}
                  y={y(theirs[i])}
                  width="100"
                  height={150 - y(theirs[i])}
                  fill="none"
                  stroke={colors.baseline}
                  strokeWidth="1.5"
                />
              )}
              <text
                x={left + 50}
                y={Math.min(y(share), theirs ? y(theirs[i]) : 150) - 6}
                textAnchor="middle"
              >
                {percent(share)}
                {theirs && <tspan fill={colors.baseline}> {percent(theirs[i])}</tspan>}
              </text>
              <text x={left + 50} y="172" textAnchor="middle">
                {tickLabels[i]}
              </text>
            </g>
          );
        })}
      </svg>
    </Chart>
  );
}

function afterGap(timing: Timing) {
  const after = timing.density.after_gap;
  return after
    ? `after gaps of ${after.periods.toFixed(1)} periods a report held ${after.ratio.toFixed(2)}× its neighbours' counts per period (${after.intervals.toLocaleString()} gaps)`
    : "no gaps of 1.5 periods or more";
}

function DensityChart({ timing }: { timing: Timing }) {
  const { density } = timing;
  const color = useRecordingColor();
  const max = Math.max(1, ...density.cells);
  return (
    <Chart
      title="Displacement against interval"
      className="chart-wide"
      about="each moving interval placed by its neighbours' counts per period (across) and its own length in periods (up). after a gap, a report holding about as many periods' worth of counts as the gap was long looks like counts that were held and delivered late, while one holding about one period's worth looks like counts that weren't carried over. neither one proves the cause though"
      foot={
        <Items>
          <span>{density.intervals.toLocaleString()} intervals placed</span>
          {density.outside > 0 && (
            <span>{density.outside.toLocaleString()} at 4 periods or more</span>
          )}
          {density.alone > 0 && (
            <span>{density.alone.toLocaleString()} without a moving neighbour</span>
          )}
          <span>{afterGap(timing)}</span>
        </Items>
      }
    >
      <Canvas
        height={180}
        aria-label="Displacement against interval density"
        draw={(ctx, w, h) => {
          const left = 44,
            bottom = h - 20,
            cw = (w - left - 12) / density.columns,
            ch = (bottom - 16) / density.rows;
          ctx.fillStyle = ink(color);
          density.cells.forEach((n, i) => {
            if (!n) return;
            const column = i % density.columns,
              row = Math.floor(i / density.columns);
            ctx.globalAlpha = 0.25 + (0.75 * Math.log1p(n)) / Math.log1p(max);
            ctx.fillRect(left + column * cw, bottom - (row + 1) * ch, cw, ch);
          });
          ctx.globalAlpha = 1;
          ctx.font = "11px ui-monospace, monospace";
          ctx.strokeStyle = ink(colors.grid);
          ctx.fillStyle = ink(colors.text);
          for (let periods = 1; periods <= density.periods; periods++) {
            const y = bottom - (periods / density.periods) * (bottom - 16);
            ctx.beginPath();
            ctx.moveTo(left, y);
            ctx.lineTo(w - 12, y);
            ctx.stroke();
            ctx.fillText(`${periods}×`, 12, y + 4);
          }
          ctx.fillText("interval in periods", left, 10);
          const across = "neighbours' counts per period";
          ctx.fillText(across, w - 12 - ctx.measureText(across).width, 10);
          ctx.textAlign = "center";
          for (const counts of niceTicks(0, density.x_max, 8)) {
            const x = left + (counts / density.x_max) * (w - left - 12);
            ctx.fillText(`${counts}`, Math.min(x, w - 20), h - 4);
          }
          ctx.textAlign = "start";
        }}
      />
    </Chart>
  );
}

function Bursts({
  plot,
  timing,
  onSelect,
}: {
  plot: Plot;
  timing: Timing;
  onSelect: (range: [number, number]) => void;
}) {
  const bursts = timing.bursts;
  const seconds = (us: number) => ((us - plot.from_us) / 1e6).toFixed(2);
  const select = (from: number, to: number) =>
    onSelect([(from - plot.from_us) / 1e6, (to - plot.from_us) / 1e6]);
  return (
    <Chart
      title="Bursts"
      className="chart-wide"
      about="stretches of movement split at pauses over 50 ms, and picking one selects its range for every tab. a burst is part of one recording, so it isn't an independent sample"
      foot={
        bursts && (
          <Items>
            <span>split at pauses over 50 ms</span>
            <span>a rate needs 20 intervals</span>
            {bursts.more > 0 && <span>{bursts.more.toLocaleString()} more not listed</span>}
          </Items>
        )
      }
    >
      {!bursts ? (
        <Unavailable>these only get listed for a saved recording or a paused range</Unavailable>
      ) : !bursts.rows.length ? (
        <Unavailable>no movement in this range</Unavailable>
      ) : (
        <div className="burst-table">
          <ResizableTable id="bursts">
            <TableHeader>
              <TableRow>
                <TableHead>Start</TableHead>
                <TableHead>Length</TableHead>
                <TableHead>Reports</TableHead>
                <TableHead>Rate</TableHead>
                <TableHead>Median</TableHead>
                <TableHead>Longest</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {bursts.rows.map((burst) => (
                <TableRow
                  key={burst.from_us}
                  tabIndex={0}
                  aria-label={`Select the burst at ${seconds(burst.from_us)} s`}
                  onClick={() => select(burst.from_us, burst.to_us)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") select(burst.from_us, burst.to_us);
                  }}
                >
                  <TableCell>{seconds(burst.from_us)} s</TableCell>
                  <TableCell>{((burst.to_us - burst.from_us) / 1e6).toFixed(2)} s</TableCell>
                  <TableCell>{burst.reports.toLocaleString()}</TableCell>
                  <TableCell>
                    {burst.rate_hz === null
                      ? "-"
                      : `${Math.round(burst.rate_hz).toLocaleString()} Hz`}
                  </TableCell>
                  <TableCell>
                    {burst.median_us === null ? "-" : formatUs(burst.median_us)}
                  </TableCell>
                  <TableCell>{formatUs(burst.longest_us)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </ResizableTable>
        </div>
      )}
    </Chart>
  );
}

/** the Timing tab: how the host observed the moving intervals */
export function TimingTab({
  plot,
  baseline,
  baselineLabel,
  onSelect,
}: {
  plot: Plot;
  baseline?: Plot;
  baselineLabel?: string;
  onSelect: (range: [number, number]) => void;
}) {
  const timing = plot.tabs.timing;
  if (!timing) return null;
  if (plot.readouts.period_us === null) return <Unavailable>{noMovement}</Unavailable>;
  const base = baseline?.tabs.timing ?? undefined;
  return (
    <div className="chart-grid">
      <JitterChart timing={timing} base={base} />
      <Groups timing={timing} base={base} />
      <Longer timing={timing} base={base} />
      <Ticks timing={timing} base={base} />
      <EachRecording plot={plot} baseline={baseline} baselineLabel={baselineLabel}>
        {(shown) => shown.tabs.timing && <DensityChart timing={shown.tabs.timing} />}
      </EachRecording>
      <EachRecording plot={plot} baseline={baseline} baselineLabel={baselineLabel}>
        {(shown) =>
          shown.tabs.timing && (
            <Bursts plot={shown} timing={shown.tabs.timing} onSelect={onSelect} />
          )
        }
      </EachRecording>
    </div>
  );
}
