import type { ReactNode } from "react";
import { InfoTip } from "@/components/info-tip";
import { formatUs } from "@/lib/plots";
import type { Plot } from "@/types";

type Values = Plot["readouts"];

const readouts: {
  label: string;
  about: string;
  value: (r: Values) => string;
  unit?: string;
  warning?: (r: Values) => string | null;
}[] = [
  {
    label: "Active report rate",
    about:
      "USB reports per second while the mouse moves, so one over the median time between back to back reports that both carry movement, over the latest second of movement or the whole selection once you choose one. pauses longer than 50 ms are left out, and this is NOT the sensor's sampling rate",
    value: (r) => (r.rate_hz > 0 ? Math.round(r.rate_hz).toLocaleString() : "-"),
    unit: "Hz",
    // halfway to 2 kHz, so a 1 kHz mouse's jitter never sets it off
    warning: (r) =>
      r.rate_hz > 1500
        ? "the judges were trained on 1 kHz and don't handle this rate yet, so a score from it doesn't mean much"
        : null,
  },
  {
    label: "Period estimate",
    about:
      "the median moving interval over the whole view, the interval groups and the jitter are both measured against it",
    value: (r) => us(r.period_us),
  },
  {
    label: "Median jitter",
    about:
      "how far a 1x moving interval usually lands from the period, as the median distance. it's what the host saw, so the host's own scheduling is in there too, and the Timing tab shows the spread",
    value: (r) => (r.jitter_us === null ? "-" : `${r.jitter_us.toFixed(1)} µs`),
  },
  {
    label: "Longest moving gap",
    about:
      "the longest interval between two reports that both carried movement. a pause over 50 ms splits the movement, so it isn't counted",
    value: (r) => us(r.longest_moving_us),
  },
];

const us = (value: number | null) => (value === null ? "-" : formatUs(value));

/** the numbers above the tabs, all from moving intervals; a baseline's sit under the
 * candidate's, named */
export function Readouts({ plot, baseline }: { plot: Plot; baseline?: Plot }) {
  return (
    <div className="metric-grid">
      {readouts.map(({ label, about, value, unit, warning }) => {
        const shown = value(plot.readouts);
        const warned = warning?.(plot.readouts);
        return (
          <div key={label}>
            <span>
              {label}
              <InfoTip label={`About ${label.toLowerCase()}`}>{about}</InfoTip>
            </span>
            <strong>
              {shown}
              {unit && shown !== "-" && <em>{unit}</em>}
            </strong>
            {warned && <small className="metric-warning">{warned}</small>}
            {baseline && (
              <small className="metric-baseline">
                baseline {value(baseline.readouts)}
                {unit && value(baseline.readouts) !== "-" && ` ${unit}`}
              </small>
            )}
          </div>
        );
      })}
    </div>
  );
}

/** what a saved recording is before any number: how it ended and what it holds */
export function HealthStrip({
  role,
  items,
}: {
  /** which recording this is, candidate or baseline */
  role: string;
  items: [label: string, value: ReactNode][];
}) {
  return (
    <dl className="health-strip" aria-label={`${role} recording health`}>
      <strong>{role}</strong>
      {items.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}
