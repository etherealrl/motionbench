import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Clock } from "lucide-react";
import { Notice } from "@/components/notice";
import type { Run } from "@/types";
import { RunDialog, type ShownRun } from "./run-dialog";

type Timing = {
  anchor_summary?: {
    samples: number;
    max_bracket_us: number;
    max_mapping_change_us: number;
    discontinuities: number;
  };
};
type Observers = {
  complete?: boolean;
  raw_input?: { max_batch: number };
};
type Paired = {
  receipt_alignment?: {
    observed_rate_ppm: number;
    residual_p05_us: number;
    residual_p95_us: number;
  };
};

const microseconds = (value: number | undefined) =>
  value === undefined ? "-" : `${value.toFixed(1)} µs`;

/** how a recording's clocks were tied together, from what it wrote beside its capture */
export function RecordingClocks({
  shown,
  onClose,
}: {
  shown: ShownRun | null;
  onClose: () => void;
}) {
  return (
    <RunDialog shown={shown} title="Recording clocks" onClose={onClose}>
      {(run) => <Clocks run={run} />}
    </RunDialog>
  );
}

function Clocks({ run }: { run: Run }) {
  const anchors = (run.metadata.timing as Timing | undefined)?.anchor_summary;
  const observers = run.metadata.observers as Observers | undefined;
  const alignment = (run.metadata.paired as Paired | undefined)?.receipt_alignment;
  const degraded = run.status.kind === "degraded" && run.status.message && (
    <Notice>{run.status.message}</Notice>
  );
  if (!anchors && !observers && !alignment)
    return (
      <>
        {degraded}
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Clock />
            </EmptyMedia>
            <EmptyTitle>No clock data</EmptyTitle>
            <EmptyDescription>this recording didn't save any clock data</EmptyDescription>
          </EmptyHeader>
        </Empty>
      </>
    );
  return (
    <>
      {degraded}
      <dl className="readout">
        <dt>Clock anchors</dt>
        <dd>{anchors?.samples ?? "-"}</dd>
        <dt>Largest anchor uncertainty</dt>
        <dd>{microseconds(anchors?.max_bracket_us)}</dd>
        <dt>Largest UTC/QPC change</dt>
        <dd>{microseconds(anchors?.max_mapping_change_us)}</dd>
        <dt>Clock discontinuities</dt>
        <dd>{anchors?.discontinuities ?? "-"}</dd>
        <dt>Raw Input largest batch</dt>
        <dd>{observers?.raw_input?.max_batch ?? "-"}</dd>
        <dt>Observer evidence</dt>
        <dd>{observers ? (observers.complete ? "Finalized" : "Incomplete") : "-"}</dd>
      </dl>
      {alignment && (
        <p className="field-description">
          serial receipt envelope: {alignment.observed_rate_ppm.toFixed(1)} ppm observed relative
          rate, residual p05 {alignment.residual_p05_us.toFixed(0)} µs, p95{" "}
          {alignment.residual_p95_us.toFixed(0)} µs. clock drift and a transport delay that changes
          can both feed into it, and it doesn't change any score
        </p>
      )}
      <p className="field-description">
        receipt clocks aren't physical USB timestamps, and the transport delay and the display's
        scanout aren't measured either
      </p>
    </>
  );
}
