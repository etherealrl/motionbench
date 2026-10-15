import { Items } from "@/components/inline";
import type { Plot, Relay } from "@/types";
import {
  axisLines,
  Canvas,
  Chart,
  EachRecording,
  Unavailable,
  grid,
  timeAxis,
  useRecordingColor,
  ink,
} from "./chart";
import { CentredHistogram, percent, signed } from "./marks";

const noInput = "unavailable, this view needs a saved recording that has the box's own input";

const counts = (c: [number, number]) => `dx ${c[0].toLocaleString()}, dy ${c[1].toLocaleString()}`;

function Conserved({ plot, relay }: { plot: Plot; relay: Relay }) {
  const duration = Math.max(0.001, (plot.to_us - plot.from_us) / 1e6);
  const color = useRecordingColor();
  return (
    <Chart
      className="chart-wide"
      title="Counts conserved"
      about="the box's counts minus the wire's, summed from the start of the range, dx solid and dy dashed. a lossless relay stays at zero, so any drift means counts got merged, split, dropped, scaled or added on the way. a count that's still in flight shows up as a brief step"
      foot={
        <Items>
          <span>box {counts(relay.box_counts)}</span>
          <span>wire {counts(relay.wire_counts)}</span>
        </Items>
      }
    >
      <Canvas
        height={160}
        aria-label="Box counts minus wire counts over time"
        draw={(ctx, w, h) => {
          const max = Math.max(
            1,
            ...relay.conserved.flatMap((c) => [Math.abs(c[1]), Math.abs(c[2])]),
          );
          grid(ctx, w, h, -max, max, "");
          axisLines(ctx, color, [1, 2], (axis) =>
            relay.conserved.forEach((c, i) => {
              const x = 60 + (c[0] / duration) * (w - 72);
              const y = 40 + (1 - (c[axis] + max) / (2 * max)) * (h - 65);
              if (!i) ctx.moveTo(x, y);
              else ctx.lineTo(x, y);
            }),
          );
          timeAxis(ctx, w, h, duration);
        }}
      />
    </Chart>
  );
}

function Windows({ plot, relay }: { plot: Plot; relay: Relay }) {
  const duration = Math.max(0.001, (plot.to_us - plot.from_us) / 1e6);
  const off = relay.windows.filter((w) => w[0] || w[1]).length;
  const color = useRecordingColor();
  return (
    <Chart
      className="chart-wide"
      title="Difference per 100 ms"
      about="the box's counts minus the wire's in each 100 ms window, with the box's reports placed where they landed on the wire's clock. dx is the left half of each bar and dy the lighter right half, and a bar goes above the line when the box had more"
      foot={
        <span>
          {off.toLocaleString()} of {relay.windows.length.toLocaleString()} windows differ
        </span>
      }
    >
      <Canvas
        height={120}
        aria-label="Box minus wire counts per 100 ms"
        draw={(ctx, w, h) => {
          const max = Math.max(
            1,
            ...relay.windows.flatMap((c) => [Math.abs(c[0]), Math.abs(c[1])]),
          );
          grid(ctx, w, h, -max, max, "");
          const zero = 40 + 0.5 * (h - 65);
          const width = Math.max(1, (0.1 / duration) * (w - 72) - 1);
          relay.windows.forEach((c, i) => {
            const x = 60 + ((i * 0.1) / duration) * (w - 72);
            c.forEach((v, axis) => {
              if (!v) return;
              ctx.fillStyle = axis ? ink(color) + "88" : ink(color);
              const top = zero - (v / max) * 0.5 * (h - 65);
              ctx.fillRect(
                x + (axis * width) / 2,
                Math.min(top, zero),
                width / 2,
                Math.abs(top - zero),
              );
            });
          });
          timeAxis(ctx, w, h, duration);
        }}
      />
    </Chart>
  );
}

function Matching({ relay }: { relay: Relay }) {
  const rows: [string, number][] = [
    ["Paired", relay.matched],
    ["Box only", relay.box_only],
    ["Wire only", relay.wire_only],
  ];
  const total = relay.matched + relay.box_only + relay.wire_only;
  return (
    <Chart
      title="Matching"
      about="each moving box report gets paired with the nearest unpaired wire report that has the same counts, within one period of where it landed on the wire's clock. a merged or split report pairs with nothing on either side, so a lossless relay that reshapes reports still keeps its counts conserved"
      foot={<span>pairs, and moving reports left alone on either side</span>}
    >
      <div className="delivery groups">
        {rows.map(([label, n]) => (
          <div className="delivery-row" key={label}>
            <span>{label}</span>
            <div className="delivery-track">
              <div style={{ "--share": `${total ? (n / total) * 100 : 0}%` }} />
            </div>
            <strong>{percent(total ? n / total : 0)}</strong>
            <span>{n.toLocaleString()}</span>
          </div>
        ))}
      </div>
    </Chart>
  );
}

function OffsetVariation({ relay }: { relay: Relay }) {
  const { offset } = relay;
  return (
    <Chart
      title="Offset variation"
      about="how far each pair's wire time minus box time sat from their median, after the recording's fixed shift. pairs are only looked for within one period, so a variation wider than that shows up as reports left alone instead. this is only the variation of the offset, the absolute latency isn't measured here"
      foot={
        <Items>
          <span>{relay.matched.toLocaleString()} pairs</span>
          <span>the absolute latency isn't measured here</span>
          {relay.matched > 0 && (
            <span>
              {relay.matched < 100 ? "from" : "98% within"} {signed(offset.p01_us)} to{" "}
              {signed(offset.p99_us)}
            </span>
          )}
          {offset.outside > 0 && (
            <span>
              {offset.outside.toLocaleString()} beyond ±{offset.span_us} µs
            </span>
          )}
        </Items>
      }
    >
      {relay.matched ? (
        <CentredHistogram
          label="Offset variation histogram"
          bins={offset.bins}
          span={offset.span_us}
        />
      ) : (
        <Unavailable>no pairs, so there's no offset</Unavailable>
      )}
    </Chart>
  );
}

/** the Relay tab: what the box saw against what reached the wire */
export function RelayTab({
  plot,
  baseline,
  baselineLabel,
}: {
  plot: Plot;
  baseline?: Plot;
  baselineLabel?: string;
}) {
  return (
    <div className="chart-grid">
      <EachRecording
        plot={plot}
        baseline={baseline?.tabs.relay ? baseline : undefined}
        baselineLabel={baselineLabel}
      >
        {(shown) => <RelayCharts plot={shown} />}
      </EachRecording>
    </div>
  );
}

/** one recording's relay */
function RelayCharts({ plot }: { plot: Plot }) {
  const relay = plot.tabs.relay;
  if (!relay) return <Unavailable>{noInput}</Unavailable>;
  if (!plot.movement_available)
    return (
      <Unavailable>
        unavailable, there's no decoder for the wire so its counts can't be set against the box's
      </Unavailable>
    );
  return (
    <>
      <Conserved plot={plot} relay={relay} />
      <Windows plot={plot} relay={relay} />
      <Matching relay={relay} />
      <OffsetVariation relay={relay} />
    </>
  );
}
