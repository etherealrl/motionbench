import { niceTicks } from "@/lib/plots";
import { colors, useRecordingColor } from "./chart";

/** the svg charts draw in a 600 by 180 box: values from x 44 to 585, bars down to y 150 */
export const percent = (share: number) =>
  `${Number((share * 100).toFixed(share > 0 && share < 0.001 ? 3 : 1))}%`;
export const signed = (us: number | null) =>
  us === null ? "-" : `${us > 0 ? "+" : ""}${us.toFixed(0)} µs`;
export const sum = (values: number[]) => values.reduce((a, b) => a + b, 0);

/** shares of a total, zero when there is none */
export function shares(counts: number[]) {
  const total = sum(counts);
  return counts.map((n) => (total ? n / total : 0));
}

/** a step outline over bins of equal width, how a baseline sits on a candidate's bars */
function outline(values: number[], x: (i: number) => number, y: (v: number) => number) {
  return values.map((v, i) => `${i ? "L" : "M"}${x(i)},${y(v)}L${x(i + 1)},${y(v)}`).join("");
}

/** horizontal grid lines with their labels, at round values */
export function Grid({
  values,
  y,
  label,
}: {
  values: number[];
  y: (v: number) => number;
  label: (v: number) => string;
}) {
  return values.map((v) => (
    <g key={v}>
      <line x1="44" x2="585" y1={y(v)} y2={y(v)} stroke={colors.grid} />
      <text x="0" y={y(v) + 4}>
        {label(v)}
      </text>
    </g>
  ));
}

/** shares of centred whole microsecond bins either side of zero, labelled in microseconds, with
 * a baseline's shares as a step outline on the same bins */
export function CentredHistogram({
  label,
  bins,
  span,
  baseline,
}: {
  label: string;
  bins: number[];
  span: number;
  baseline?: number[];
}) {
  const own = shares(bins);
  const color = useRecordingColor();
  const theirs = baseline && baseline.length === bins.length ? shares(baseline) : null;
  const max = Math.max(0.001, ...own, ...(theirs ?? []));
  const width = 541 / own.length;
  const binWidth = (2 * span) / Math.max(1, own.length - 1);
  const x = (i: number) => 44 + i * width;
  const y = (v: number) => 150 - (v / max) * 130;
  return (
    <svg role="img" aria-label={label} viewBox="0 0 600 180" className="summary-chart">
      <Grid values={niceTicks(0, max)} y={y} label={percent} />
      {own.map((v, i) => (
        <rect
          key={i}
          x={x(i) + 0.5}
          y={y(v)}
          width={Math.max(0.5, width - 1)}
          height={150 - y(v)}
          fill={color}
        >
          <title>
            {signed(-span + i * binWidth)}: {percent(v)}
          </title>
        </rect>
      ))}
      {theirs && (
        <path d={outline(theirs, x, y)} fill="none" stroke={colors.baseline} strokeWidth="1.5" />
      )}
      {niceTicks(-span, span, 8)
        .filter(Number.isInteger)
        .map((us) => {
          // a bin is centred on its value, the middle bin on zero
          const at = x((us + span) / binWidth + 0.5);
          return (
            <g key={us}>
              <line x1={at} x2={at} y1="150" y2="155" stroke={colors.text} />
              <text x={at} y="168" textAnchor="middle">
                {us > 0 ? `+${us}` : us}
                {us === 0 && " µs"}
              </text>
            </g>
          );
        })}
    </svg>
  );
}
