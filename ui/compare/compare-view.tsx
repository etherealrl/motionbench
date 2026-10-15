import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Items } from "@/components/inline";
import { AnalysisViews } from "@/charts/views";
import { HealthStrip } from "@/charts/readouts";
import { outcome } from "@/lib/outcome";
import type { Session } from "@/app/use-session";
import type { Plot, Run } from "@/types";
import type { Comparison } from "./use-comparison";
import { ComparisonSetup } from "./comparison-setup";
import { ChartToolbar } from "./chart-toolbar";
import { ComparisonTab, ExportResult } from "./comparison-tab";
import { useNumbers } from "./numbers-table";
import { Notice } from "@/components/notice";

/** how a recording ended and what it holds, before any number from it */
function health(role: string, run: Run, plot: Plot, paired: boolean) {
  const decoder = run.devices.find((d) => d.id === run.selected)?.decoder;
  const drops = (run.metadata.health as { serial_dropped?: number } | undefined)?.serial_dropped;
  const status = outcome[run.status.kind];
  return (
    <HealthStrip
      role={role}
      items={[
        ["Status", <Badge variant={status.tone}>{status.label}</Badge>],
        [
          "Decoder",
          decoder && decoder !== "unavailable"
            ? decoder.charAt(0).toUpperCase() + decoder.slice(1)
            : "None",
        ],
        ["Clock regressions", plot.regressions.toLocaleString()],
        ["Box drops", drops === undefined ? "-" : drops.toLocaleString()],
        ["Box input", paired ? "Recorded" : "Not recorded"],
      ]}
    />
  );
}

export function CompareView({ session, comparison }: { session: Session; comparison: Comparison }) {
  const { snapshot, call } = session;
  const { plot, basePlot, range, result, paired, baseline, candidate } = comparison;
  const { tab, setTab } = comparison;
  const columns = useNumbers(session, baseline, candidate, paired, range);
  return (
    <>
      <div className="section-intro">
        <h2>Compare</h2>
      </div>
      <ComparisonSetup session={session} comparison={comparison} />
      {snapshot.job?.status === "running" && (
        <div className="job-bar">
          <Items>
            <span>Judging recorded data</span>
            <span>{(snapshot.job.progress * 100).toFixed(0)}%</span>
          </Items>
          <Progress value={snapshot.job.progress * 100} />
          <Button
            variant="outline"
            size="sm"
            onClick={() => void call("cancel_analysis", { id: snapshot.job?.id })}
          >
            Cancel
          </Button>
        </div>
      )}
      {snapshot.job?.error && <Notice error>{snapshot.job.error}</Notice>}
      {plot && plot.reports > 0 && (
        <>
          <ChartToolbar snapshot={snapshot} comparison={comparison} />
          {(candidate.length > 1 || (!paired && baseline.length > 1)) && (
            <p className="field-description chart-scope">
              the charts show the recordings you chose above, though the analysis uses all{" "}
              {candidate.length + (paired ? 0 : baseline.length)} selected recordings
            </p>
          )}
          {comparison.selectedRun &&
            health("Candidate", comparison.selectedRun, plot, comparison.selectedRun.paired)}
          {!paired &&
            basePlot &&
            comparison.baselineRun &&
            health("Baseline", comparison.baselineRun, basePlot, comparison.baselineRun.paired)}
          <AnalysisViews
            plot={plot}
            baseline={basePlot}
            baselineLabel={paired ? "box input" : "baseline"}
            scope={
              range
                ? `Selected ${range[0].toFixed(3)} to ${range[1].toFixed(3)} s`
                : "Whole recording"
            }
            tab={tab}
            onTab={setTab}
            comparison={<ComparisonTab comparison={comparison} columns={columns} />}
            comparisonActions={<ExportResult comparison={comparison} />}
            onBrush={(r) => {
              if (snapshot.job?.status === "running") return;
              const origin = range?.[0] ?? 0;
              const selection: [number, number] = [origin + r[0], origin + r[1]];
              comparison.setRange(selection);
              comparison.setFromText(selection[0].toFixed(3));
              comparison.setToText(selection[1].toFixed(3));
            }}
          />
        </>
      )}
      {!(plot && plot.reports > 0) && result && (
        <ComparisonTab comparison={comparison} columns={columns} exporting />
      )}
    </>
  );
}
