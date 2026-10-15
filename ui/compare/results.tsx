import { ChevronRight } from "lucide-react";
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { native } from "@/lib/native";
import type { NotComplete, ResultRow } from "@/types";
import { Badge } from "@/components/ui/badge";
import { Chart } from "@/charts/chart";
import { Items } from "@/components/inline";
import { Notice } from "@/components/notice";

function histogram(values: number[], lo: number, hi: number, count = 24) {
  const bins = Array.from({ length: count }, () => 0);
  for (const v of values)
    bins[
      Math.max(0, Math.min(count - 1, Math.floor(((v - lo) / Math.max(1e-9, hi - lo)) * count)))
    ]++;
  return bins.map((v) => (v / Math.max(1, values.length)) * 100);
}
function FeatureDistribution({ result: r, column }: { result: ResultRow; column: number }) {
  const a = r.real_features.map((v) => v[column]),
    b = r.fake_features.map((v) => v[column]);
  const lo = [...a, ...b].reduce((m, v) => Math.min(m, v), Infinity),
    hi = [...a, ...b].reduce((m, v) => Math.max(m, v), -Infinity);
  const aa = histogram(a, lo, hi),
    bb = histogram(b, lo, hi);
  const max = Math.max(1, ...aa, ...bb);
  return (
    <svg
      role="img"
      aria-label={`Distributions for ${r.feature_names[column]}`}
      viewBox="0 0 500 130"
      className="feature-distribution"
    >
      {aa.map((v, i) => (
        <g key={i}>
          <rect
            x={20 + i * 19}
            y={100 - (v / max) * 90}
            height={(v / max) * 90}
            width="15"
            fill="var(--color-chart-baseline)"
            opacity=".6"
          />
          <rect
            x={25 + i * 19}
            y={100 - (bb[i] / max) * 90}
            height={(bb[i] / max) * 90}
            width="12"
            fill="var(--color-chart-candidate)"
            opacity=".7"
          />
        </g>
      ))}
      <text x="20" y="122">
        {lo.toPrecision(4)}
      </text>
      <text x="430" y="122">
        {hi.toPrecision(4)}
      </text>
    </svg>
  );
}
/** a paired result splits one recording into blocks; the others hold whole recordings */
const pairedResult = (r: ResultRow) => r.comparison.startsWith("paired");
const tooFewGroups = (r: ResultRow) =>
  !!r.judge_unavailable?.includes("at least two independent source groups");

/** what each side brought, which bounds what the AUC can say */
function sources(r: ResultRow) {
  if (pairedResult(r))
    return `one recording against its box input, ${r.judge?.n_groups ?? "-"} blocks`;
  const n = (list: string[]) => `${list.length} recording${list.length === 1 ? "" : "s"}`;
  return `${n(r.baseline)} against ${n(r.candidate)}`;
}

/** why the judge gave no AUC, in a few words */
/** the AUC's interval, or why there's none */
function interval(r: ResultRow) {
  const ci = r.judge?.auc_ci;
  if (ci) return `95% CI ${ci[0].toFixed(3)} to ${ci[1].toFixed(3)}`;
  if (r.judge) return "no interval, each held-out part has only one recording a side";
  return unavailable(r);
}

function unavailable(r: ResultRow) {
  if (!r.judge_unavailable) return "no judge result";
  if (!tooFewGroups(r)) return r.judge_unavailable;
  return pairedResult(r) ? "the recording is too short" : "needs two recordings on each side";
}

/** the same reason, with what to do about it */
function explain(r: ResultRow) {
  if (!tooFewGroups(r)) return `W1 is available. AUC is unavailable: ${r.judge_unavailable}`;
  return pairedResult(r)
    ? "the feature differences (W1) are there. AUC needs the recording split into at least two independent blocks so it can test on a block it didn't train on, and a longer recording gives you more blocks"
    : "the feature differences (W1) are there. AUC needs at least two separate recordings on each side so it can test on one it didn't train on, so add another recording to the side that only has one. recording for longer doesn't add independent sessions";
}

/** a recording the result read that didn't end complete, so the number is read with that in mind */
function notComplete(source: NotComplete) {
  const name = source.path.split(/[\\/]/).pop();
  return `${name} didn't end complete, ${source.message ?? source.kind}, so read this result with that in mind`;
}

export function Results({ result: r }: { result: ResultRow }) {
  const [expanded, setExpanded] = useState<number | null>(null);
  const judge = r.judge;
  const order = r.w1
    .map((distance, i) => ({ distance, i }))
    .sort((a, b) => b.distance - a.distance);
  const max = Math.max(1e-6, ...r.w1);
  const mean = r.w1.reduce((a, b) => a + b, 0) / Math.max(1, r.w1.length);
  return (
    <div className="results">
      <div className="result-intro">
        <div>
          <Badge variant="outline">{r.panel}</Badge>
          <h2>What the judge could separate</h2>
          <p>
            <Items>
              <span>{r.comparison}</span>
              <span>{r.config.width_ms} ms windows</span>
              <span>source-grouped held-out evaluation</span>
            </Items>
          </p>
        </div>
        <JudgeRevision revision={r.judge_revision} />
      </div>
      <div className="metric-grid">
        <div>
          <span>Held-out AUC</span>
          <strong>{judge ? judge.auc.toFixed(3) : "Unavailable"}</strong>
          <small>
            <Items>
              <span>{sources(r)}</span>
              <span>{interval(r)}</span>
            </Items>
          </small>
        </div>
        <div>
          <span>Mean standardized W1</span>
          <strong>{mean.toFixed(3)}</strong>
          <small>lower means closer feature distributions</small>
        </div>
        <div>
          <span>Permutation p</span>
          <strong>
            {judge?.p_value === null || !judge
              ? "-"
              : judge.p_value < 0.001
                ? "< 0.001"
                : judge.p_value.toFixed(3)}
          </strong>
          <small>{judge?.permutation_auc.length ?? 0} grouped shuffles</small>
        </div>
        <div>
          <span>Independent groups</span>
          <strong>{judge?.n_groups ?? "-"}</strong>
          <small>
            <Items>
              <span>{r.real_features.length} baseline</span>
              <span>{r.fake_features.length} candidate rows</span>
            </Items>
          </small>
        </div>
      </div>
      {r.not_complete.map((source) => (
        <Notice key={source.path}>{notComplete(source)}</Notice>
      ))}
      {r.judge_unavailable && <Notice>{explain(r)}</Notice>}
      {judge?.permutation_error && <Notice>{judge.permutation_error}</Notice>}
      {judge?.smallest_p != null && judge.smallest_p > 0.05 && (
        <Notice>
          with {judge.n_groups} recordings the shuffles can't give a p under{" "}
          {judge.smallest_p.toFixed(2)}, so p can't show a difference here however big it is. 4
          recordings a side get it under 0.05
        </Notice>
      )}
      <p className="interpretation">
        AUC 0.5 is chance and 1.0 means the held out data separated completely. this describes these
        labeled populations, it's NOT a detection threshold for some unknown person
      </p>
      {judge && (
        <div className="chart-grid">
          <Chart
            title="Observed AUC against shuffled labels"
            about="two-sided grouped permutation null"
          >
            <svg
              role="img"
              aria-label="Permutation AUC histogram and observed AUC"
              viewBox="0 0 600 180"
              className="summary-chart"
            >
              {histogram(judge.permutation_auc, 0, 1, 50).map((v, i, bins) => (
                <rect
                  key={i}
                  x={40 + i * 10.5}
                  y={145 - (v / Math.max(1, ...bins)) * 110}
                  width="9"
                  height={(v / Math.max(1, ...bins)) * 110}
                  fill="var(--color-chart-null)"
                />
              ))}
              <line
                x1={40 + judge.auc * 525}
                x2={40 + judge.auc * 525}
                y1="15"
                y2="145"
                stroke="var(--color-chart-candidate)"
                strokeWidth="2"
              />
              <line
                x1="302.5"
                x2="302.5"
                y1="15"
                y2="145"
                stroke="var(--color-chart-baseline)"
                strokeDasharray="3 3"
              />
              <text x="40" y="172">
                0.0
              </text>
              <text x="290" y="172">
                0.5
              </text>
              <text x="555" y="172">
                1.0
              </text>
            </svg>
            <div className="chart-foot">
              {judge.null_p95 !== null
                ? `95% shuffled-distance boundary: ${judge.null_p95.toFixed(3)}`
                : "no permutation null was requested"}
            </div>
          </Chart>
          <Chart
            title="Fold and repeat resolution"
            about={`${r.config.repeats} repeats, train-only standardization`}
          >
            <div className="folds">
              {judge.fold_auc.map((folds, i) => (
                <div key={i}>
                  <span>repeat {i + 1}</span>
                  {folds.map((v, j) => (
                    <b
                      key={j}
                      title={`fold ${j + 1}`}
                      style={{ "--fold-opacity": 0.5 + Math.abs(v - 0.5) }}
                    >
                      {v.toFixed(3)}
                    </b>
                  ))}
                  <strong>{judge.repeat_auc[i].toFixed(3)}</strong>
                </div>
              ))}
            </div>
            <div className="chart-foot">
              keep source trials and recording variants together across folds
            </div>
          </Chart>
        </div>
      )}
      <Chart
        title="Features that changed most"
        about="W1 standardized by the baseline spread, expand to inspect the distributions"
      >
        <div className="feature-list">
          {order.map(({ distance, i }) => (
            <div key={i}>
              <button
                className="feature-row"
                onClick={() => setExpanded(expanded === i ? null : i)}
                aria-expanded={expanded === i}
              >
                <span>{r.feature_names[i]}</span>
                <div>
                  <i style={{ "--distance": `${(distance / max) * 100}%` }} />
                </div>
                <strong>{distance.toFixed(4)}</strong>
                <ChevronRight size={14} aria-hidden />
              </button>
              {expanded === i && <FeatureDistribution result={r} column={i} />}
            </div>
          ))}
        </div>
      </Chart>
      <div className="chart-foot">
        excluded idle windows: {r.idle_dropped[0]} baseline / {r.idle_dropped[1]} candidate
        {r.alignment_shift_us !== null && `, clock shift: ${r.alignment_shift_us} µs`}
      </div>
    </div>
  );
}

/** the judge's own commit, the code that made the result. native opens it in the browser */
function JudgeRevision({ revision }: { revision: string }) {
  return (
    <a
      className="revision"
      href={`https://github.com/optima-manent/ABCurves/commit/${revision}`}
      onClick={(e) => {
        if (!native) return;
        e.preventDefault();
        void invoke("open_judge_commit", { revision });
      }}
    >
      ABCurves {revision.slice(0, 7)}
    </a>
  );
}
