import { RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Input } from "@/components/ui/input";
import type { Snapshot } from "@/types";
import type { Comparison } from "./use-comparison";

export function ChartToolbar({
  snapshot,
  comparison,
}: {
  snapshot: Snapshot;
  comparison: Comparison;
}) {
  const {
    paired,
    baseline,
    candidate,
    selectedRun,
    baselineRun,
    fromText,
    setFromText,
    toText,
    setToText,
    setRange,
  } = comparison;
  const busy = snapshot.job?.status === "running";
  // a new recording clears the range picked on the one before
  const pick = (choose: (id: string) => void) => (id: string) => {
    choose(id);
    setRange(null);
    setFromText("");
    setToText("");
  };
  return (
    <div className="range-toolbar">
      <div className="chart-recordings">
        <ChartRecording
          label="Candidate chart"
          ids={candidate}
          value={selectedRun?.id}
          runs={snapshot.runs}
          disabled={busy}
          onPick={pick(comparison.setChartCandidate)}
        />
        {paired ? (
          <span>Baseline: physical input</span>
        ) : (
          baseline.length > 0 && (
            <ChartRecording
              label="Baseline chart"
              ids={baseline}
              value={baselineRun?.id}
              runs={snapshot.runs}
              disabled={busy}
              onPick={pick(comparison.setChartBaseline)}
            />
          )
        )}
      </div>
      <div className="range-inputs">
        <Input
          className="h-8"
          aria-label="Range start in seconds"
          placeholder="from s"
          type="number"
          value={fromText}
          onChange={(e) => setFromText(e.target.value)}
        />
        <Input
          className="h-8"
          aria-label="Range end in seconds"
          placeholder="to s"
          type="number"
          value={toText}
          onChange={(e) => setToText(e.target.value)}
        />
        <Button
          variant="outline"
          size="sm"
          disabled={snapshot.job?.status === "running"}
          onClick={() => {
            if (Number(toText) > Number(fromText)) setRange([Number(fromText), Number(toText)]);
          }}
        >
          Apply range
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={snapshot.job?.status === "running"}
          onClick={() => {
            setRange(null);
            setFromText("");
            setToText("");
          }}
        >
          <RotateCcw /> Reset
        </Button>
      </div>
    </div>
  );
}

/** which of one side's recordings the charts show */
function ChartRecording({
  label,
  ids,
  value,
  runs,
  disabled,
  onPick,
}: {
  label: string;
  ids: string[];
  value: string | undefined;
  runs: Snapshot["runs"];
  disabled: boolean;
  onPick: (id: string) => void;
}) {
  return (
    <label>
      {label}
      <Dropdown
        aria-label={`${label} recording`}
        value={value ?? ""}
        disabled={disabled}
        onValueChange={onPick}
      >
        {ids.map((id) => (
          <DropdownOption key={id} value={id}>
            {runs.find((run) => run.id === id)?.name}
          </DropdownOption>
        ))}
      </Dropdown>
    </label>
  );
}
