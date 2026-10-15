import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Items } from "@/components/inline";
import { Notice } from "@/components/notice";
import { currentFrame } from "@/lib/vision";
import type { Snapshot } from "@/types";
import { boxLosses, boxName } from "./setup/box";
import { MediusDetails } from "./setup/medius";

/** what in the popover needs a look, named as its rows are, so the header can say it */
export function diagnosticsWarnings(snapshot: Snapshot): string[] {
  const { health, observers, box, vision } = snapshot;
  return [
    health.serial_dropped > 0 && "Serial losses",
    health.timestamp_regressions > 0 && "Clock regressions",
    !!observers?.error && "Observer error",
    (observers?.queue_losses ?? 0) > 0 && "Observer queue losses",
    box.link === "connected" && boxLosses(box) > 0 && "Box input losses",
    !!vision?.optimization_warning && "Target capture warning",
  ].filter((reason): reason is string => !!reason);
}

/** everything that explains a capture's health, in one place */
export function Diagnostics({ snapshot }: { snapshot: Snapshot }) {
  const decoder = snapshot.devices.find((d) => d.id === snapshot.selected)?.decoder;
  const vision = snapshot.vision;
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm">
          Diagnostics
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="status-diagnostics">
        <Group title="Capture">
          <dl className="readout">
            <dt>Decoder</dt>
            <dd>
              {decoder ? decoder.charAt(0).toUpperCase() + decoder.slice(1) : "No mouse selected"}
            </dd>
            <dt>Reports</dt>
            <dd>{snapshot.health.reports.toLocaleString()}</dd>
            <dt>Serial losses</dt>
            <dd>{snapshot.health.serial_dropped}</dd>
            <dt>Clock regressions</dt>
            <dd>{snapshot.health.timestamp_regressions}</dd>
          </dl>
        </Group>
        <Observers snapshot={snapshot} />
        {snapshot.box.link === "connected" && <Box box={snapshot.box} />}
        {(vision?.running || vision?.error) && <Targets vision={vision} />}
      </PopoverContent>
    </Popover>
  );
}

function Box({ box }: { box: Snapshot["box"] }) {
  const h = box.medius?.health;
  return (
    <Group title="Box">
      <dl className="readout">
        <dt>Box</dt>
        <dd>{boxName(box)}</dd>
        <dt>Input losses</dt>
        <dd>
          {h ? (
            <Items>
              <span>{boxLosses(box)}</span>
              <span>input {box.losses}</span>
              <span>link {h.link_drops}</span>
              <span>host {h.host_drops}</span>
              <span>TX {h.tx_drops}</span>
            </Items>
          ) : (
            box.losses
          )}
        </dd>
      </dl>
      {box.medius && <MediusDetails medius={box.medius} />}
    </Group>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="diagnostics-group" aria-label={title}>
      <h4>{title}</h4>
      {children}
    </section>
  );
}

// the host-side observers that see the same mouse; the driver one only shows when installed
function Observers({ snapshot }: { snapshot: Snapshot }) {
  const status = snapshot.observers;
  const showDriver = (snapshot.dev_mode ?? import.meta.env.DEV) || status?.kernel_ready;
  const raw = !snapshot.live
    ? ({ label: "Stopped", tone: "muted" } as const)
    : status?.raw_ready
      ? ({ label: "Ready", tone: "success" } as const)
      : ({ label: "Starting…", tone: "warning" } as const);
  return (
    <Group title="Observers">
      <dl className="readout">
        <dt>Raw Input</dt>
        <dd>
          <Badge variant={raw.tone}>{raw.label}</Badge>
        </dd>
        {showDriver && (
          <>
            <dt>Mouse-class observer</dt>
            <dd>
              {status?.kernel_ready ? (
                <Badge variant="success">Ready</Badge>
              ) : status?.kernel_status ? (
                status.kernel_status
              ) : (
                <Badge variant="muted">Unavailable</Badge>
              )}
            </dd>
          </>
        )}
        <dt>Queue losses</dt>
        <dd>{(status?.queue_losses ?? 0).toLocaleString()}</dd>
      </dl>
      {status?.error && <Notice error>{status.error}</Notice>}
    </Group>
  );
}

// timings of a frame the worker no longer has are not current, so they read "-" like the rates
function Targets({ vision }: { vision: NonNullable<Snapshot["vision"]> }) {
  const last = currentFrame(vision) ? vision.last : null;
  const ms = (value: number | null | undefined) => (value == null ? "-" : `${value.toFixed(1)} ms`);
  return (
    <Group title="Target capture">
      <dl className="readout">
        {vision.transport && (
          <>
            <dt>Transport</dt>
            <dd>{vision.transport}</dd>
          </>
        )}
        <dt>Analyzed frames</dt>
        <dd>{vision.frames.toLocaleString()}</dd>
        <dt>Skipped by DXGI</dt>
        <dd>{vision.skipped.toLocaleString()}</dd>
        <dt>Rate-limited</dt>
        <dd>{(vision.throttled ?? 0).toLocaleString()}</dd>
        <dt>Preprocess</dt>
        <dd>{ms(last?.preprocess_ms)}</dd>
        <dt>Inference</dt>
        <dd>{ms(last?.inference_ms)}</dd>
      </dl>
      {vision.optimization_warning && <Notice>{vision.optimization_warning}</Notice>}
    </Group>
  );
}
