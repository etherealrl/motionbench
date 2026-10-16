import { flushSettings, useSettings, updateSettings } from "@/lib/settings";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Crosshair, Maximize2, Minimize2, Play, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Switch } from "@/components/ui/switch";
import { Badge, type BadgeTone } from "@/components/ui/badge";
import { currentFrame, targetCircle } from "@/lib/vision";
import { TargetStage } from "@/components/target-stage";
import { native } from "@/lib/native";
import { blockerHint } from "@/lib/text";
import type { Blocker, Run, VisionConfig, VisionStatus } from "@/types";
import { Items } from "@/components/inline";
import { Summary } from "@/components/summary";
import { Notice } from "@/components/notice";
import { SettingRow, SettingSection } from "@/components/setting-row";
import { SetupSection } from "@/components/setup-section";
import { ResponseDelay, useDelayEstimate } from "./response-delay";
import { NumberRow, type NumberSetting, type Update } from "./number-row";
import { ProjectionSettings } from "./projection";

const targetClass: NumberSetting = ["target_class", "Target class", null, 0, "-1 takes any class"];
const detector: NumberSetting[] = [["confidence", "Minimum confidence", null, 2]];
const hardware: NumberSetting[] = [["cuda_device", "CUDA device", null, 0]];

// why Start can't be pressed, if it can't
function startHint(pending: boolean, blocker: Blocker | null, unnamed: boolean) {
  if (pending) return undefined;
  if (blocker) return blockerHint(blocker) || undefined;
  return unnamed
    ? "name the scenario first, a recording without one gets no target events"
    : undefined;
}

// what target capture is doing, in a word and a tone; off says nothing
function captureState(
  status: VisionStatus | undefined,
  pending: boolean,
  provider: string,
  fresh: boolean,
): { label: string; tone: BadgeTone } | null {
  const running = status?.running ?? false;
  if (status?.error) return { label: "Needs attention", tone: "danger" };
  if (pending || (running && !status?.ready))
    return {
      label: provider === "TensorRT" ? "Preparing TensorRT engine…" : "Warming up…",
      tone: "warning",
    };
  if (status?.recording) return { label: "Recording targets", tone: "success" };
  if (!running) return null;
  return fresh
    ? { label: "Running", tone: "success" }
    : { label: "Waiting for frames", tone: "warning" };
}

type Command = (name: string, args?: Record<string, unknown>) => Promise<unknown>;

// the saved target settings, and starting or stopping the worker with them
function useTargetCapture(status: VisionStatus | undefined, command: Command) {
  const config = useSettings().vision;
  const [pending, setPending] = useState(false);
  const running = status?.running ?? false;
  const active = running || !!status?.error;
  const fresh = currentFrame(status);
  const update: Update = (key, value) => updateSettings({ vision: { ...config, [key]: value } });
  const toggle = async () => {
    if (pending) return;
    setPending(true);
    try {
      // target capture starts with the saved settings, so a change not saved yet stops it
      if (active) await command("screen_stop");
      else if (await flushSettings()) await command("screen_start");
    } finally {
      setPending(false);
    }
  };
  return { config, update, pending, running, active, fresh, toggle };
}

/** target capture: its settings, with the live detections beside them while it runs */
export function TargetSection({
  status,
  recording,
  blocker,
  native,
  command,
  runs,
  open: expanded,
  onOpenChange,
}: {
  status?: VisionStatus;
  recording: boolean;
  /** what stops Start, or Stop while it runs */
  blocker: Blocker | null;
  native: boolean;
  command: Command;
  runs: Run[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { config, update, pending, running, active, fresh, toggle } = useTargetCapture(
    status,
    command,
  );
  const estimate = useDelayEstimate(command);
  const locked = running || recording || pending;
  const state = captureState(status, pending, config.provider, fresh);
  // a recording without a scenario gets no target events, so it doesn't start without one
  const unnamed = !active && !config.scenario.trim();
  return (
    <SetupSection
      title="Target capture"
      open={expanded}
      onOpenChange={onOpenChange}
      status={state && <Badge variant={state.tone}>{state.label}</Badge>}
      hint={startHint(pending, blocker, unnamed)}
      action={
        <StartStop
          active={active}
          pending={pending}
          disabled={!native || pending || blocker !== null || unnamed}
          onClick={() => void toggle()}
        />
      }
      aside={active && <TargetLive status={status} fresh={fresh} />}
    >
      {status?.error && <Notice error>{status.error}</Notice>}
      {running && !recording && (
        <p className="field-description">
          stop target capture first if you want to change its settings
        </p>
      )}
      <TargetSettings config={config} update={update} locked={locked} native={native} />
      <ProjectionSettings config={config} status={status} locked={locked} />
      <StartOnLaunch />
      <details className="advanced-settings">
        <Summary>Detector and hardware</Summary>
        <DetectorAndHardware config={config} update={update} locked={locked} />
      </details>
      <details className="advanced-settings">
        <Summary>Response delay</Summary>
        <ResponseDelay
          estimate={estimate}
          runs={runs}
          locked={locked}
          native={native}
          profile={config}
          apply={(delay) => update("display_delay_ms", delay)}
        />
      </details>
    </SetupSection>
  );
}

// a preference, not one of target capture's own settings, so it can change while it runs
function StartOnLaunch() {
  const targets = useSettings().targets;
  return (
    <div className="setting-group">
      <SettingRow
        id="targets-on-launch"
        label="Start with the app"
        description="starts target capture whenever Motion Bench opens, so it's ready by the time you are"
      >
        <Switch
          id="targets-on-launch"
          checked={targets.start_on_launch}
          onCheckedChange={(start) =>
            updateSettings({ targets: { ...targets, start_on_launch: start } })
          }
        />
      </SettingRow>
    </div>
  );
}

function StartStop({
  active,
  pending,
  disabled,
  onClick,
}: {
  active: boolean;
  pending: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  const verb = active ? "Stop" : "Start";
  return (
    <Button
      size="sm"
      variant="outline"
      aria-label={`${verb} target capture`}
      disabled={disabled}
      onClick={onClick}
    >
      {active ? <Square /> : <Play />}
      {pending ? (active ? "Stopping…" : "Starting…") : verb}
    </Button>
  );
}

type Display = {
  adapter: number;
  output: number;
  number: number | null;
  gpu: string;
  size: [number, number];
  main: boolean;
};

/** the display to capture, by the number Windows gives it, which also picks the GPU it hangs off */
function DisplayPick({ config, native }: { config: VisionConfig; native: boolean }) {
  const [displays, setDisplays] = useState<Display[]>([]);
  useEffect(() => {
    if (native)
      invoke<Display[]>("displays")
        .then((found) => setDisplays(found ?? []))
        .catch(() => setDisplays([]));
  }, [native]);
  const gpus = new Set(displays.map((d) => d.gpu)).size;
  const name = (d: Display) =>
    [
      // without Windows' number, it's only the GPU's own count of its outputs
      d.number === null ? `Output ${d.output + 1} of GPU ${d.adapter + 1}` : `Display ${d.number}`,
      `${d.size[0]} × ${d.size[1]}`,
      d.main ? "main" : null,
      // the GPU only matters when there's more than one
      gpus > 1 ? d.gpu : null,
    ]
      .filter(Boolean)
      .join(", ");
  return (
    <SettingRow id="target-display" label="Display" description="the screen the game is on">
      <Dropdown
        id="target-display"
        aria-label="Display"
        // a saved display that's gone, unplugged say, shows as none picked
        value={
          displays.some((d) => d.adapter === config.adapter && d.output === config.output)
            ? `${config.adapter}:${config.output}`
            : ""
        }
        placeholder="Pick a display"
        onValueChange={(value) => {
          const [adapter, output] = value.split(":").map(Number);
          updateSettings((shown) => ({ vision: { ...shown.vision, adapter, output } }));
        }}
      >
        {displays.map((d) => (
          <DropdownOption key={`${d.adapter}:${d.output}`} value={`${d.adapter}:${d.output}`}>
            {name(d)}
          </DropdownOption>
        ))}
      </Dropdown>
    </SettingRow>
  );
}

function TargetSettings({
  config,
  update,
  locked,
  native,
}: {
  config: VisionConfig;
  update: Update;
  locked: boolean;
  native: boolean;
}) {
  return (
    <fieldset disabled={locked} className="setting-group">
      <SettingRow
        id="target-model"
        label="Target model"
        description="the ONNX model that finds the target on screen"
      >
        <div className="input-with-action">
          <Input
            id="target-model"
            value={config.model}
            placeholder="Choose a model file"
            onChange={(e) => update("model", e.target.value)}
          />
          <Button
            variant="outline"
            disabled={!native}
            onClick={async () => {
              const path = await open({
                multiple: false,
                filters: [{ name: "ONNX model", extensions: ["onnx"] }],
              });
              if (typeof path === "string") update("model", path);
            }}
          >
            Browse
          </Button>
        </div>
      </SettingRow>
      <SettingRow
        id="target-scenario"
        label="Scenario"
        description={
          config.scenario.trim()
            ? "which task the target events belong to"
            : "name the scenario, because a recording without one gets no target events"
        }
      >
        <Input
          id="target-scenario"
          value={config.scenario}
          placeholder="task / variant"
          onChange={(e) => update("scenario", e.target.value)}
        />
      </SettingRow>
      <DisplayPick config={config} native={native} />
      <SettingRow
        id="target-provider"
        label="Inference provider"
        description={
          config.provider === "TensorRT"
            ? "the first start builds the engine and that can take several minutes, though it only happens once. switching to or from DirectML needs an app restart"
            : "switching to or from DirectML needs an app restart"
        }
      >
        <Dropdown
          id="target-provider"
          aria-label="Inference provider"
          value={config.provider}
          onValueChange={(value) => update("provider", value)}
        >
          <DropdownOption value="CUDA">CUDA (NVIDIA)</DropdownOption>
          <DropdownOption value="TensorRT">TensorRT (NVIDIA)</DropdownOption>
          <DropdownOption value="DirectML">DirectML (AMD, Intel, NVIDIA)</DropdownOption>
          <DropdownOption value="CPU">CPU</DropdownOption>
        </Dropdown>
      </SettingRow>
      <SettingRow
        id="target-fps"
        label="Frame rate limit"
        description="leave it empty to analyze every frame"
      >
        <InputGroup>
          <InputGroupInput
            id="target-fps"
            type="number"
            min={0}
            step={1}
            value={config.fps || ""}
            onChange={(e) => update("fps", Number(e.target.value))}
          />
          <InputGroupAddon align="inline-end">fps</InputGroupAddon>
        </InputGroup>
      </SettingRow>
    </fieldset>
  );
}

function DetectorAndHardware({
  config,
  update,
  locked,
}: {
  config: VisionConfig;
  update: Update;
  locked: boolean;
}) {
  const rows = (settings: NumberSetting[]) =>
    settings.map((setting) => (
      <NumberRow key={setting[0]} setting={setting} config={config} update={update} />
    ));
  return (
    <fieldset disabled={locked} className="setting-stack">
      <SettingSection title="Detector">
        <div className="setting-group">
          <TargetClass config={config} update={update} />
          {rows(detector)}
        </div>
      </SettingSection>
      <SettingSection title="Hardware">
        <div className="setting-group">
          {rows(hardware)}
          <SettingRow id="target-cuda-graph" label="CUDA graph">
            <Switch
              id="target-cuda-graph"
              checked={config.cuda_graph}
              disabled={!["CUDA", "TensorRT"].includes(config.provider)}
              onCheckedChange={(checked) => update("cuda_graph", checked)}
            />
          </SettingRow>
          {config.provider === "TensorRT" && (
            <SettingRow id="target-fp16" label="FP16 precision">
              <Switch
                id="target-fp16"
                checked={config.tensorrt_fp16}
                onCheckedChange={(checked) => update("tensorrt_fp16", checked)}
              />
            </SettingRow>
          )}
        </div>
      </SettingSection>
    </fieldset>
  );
}

/** the model's own class names when its file lists them, else the class index typed in */
function TargetClass({ config, update }: { config: VisionConfig; update: Update }) {
  const [classes, setClasses] = useState<string[]>([]);
  useEffect(() => {
    setClasses([]);
    if (!native || !config.model) return;
    let active = true;
    invoke<string[]>("model_classes", { path: config.model })
      .then((names) => {
        if (active) setClasses(Array.isArray(names) ? names : []);
      })
      // a model that cannot be read says so when target capture starts
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [config.model]);
  if (!classes.length) return <NumberRow setting={targetClass} config={config} update={update} />;
  return (
    <SettingRow id="target-target_class" label="Target class">
      <Dropdown
        id="target-target_class"
        value={String(config.target_class)}
        onValueChange={(value) => update("target_class", Number(value))}
      >
        <DropdownOption value="-1">Any class</DropdownOption>
        {classes.map((name, i) => (
          <DropdownOption key={i} value={String(i)}>
            {name}
          </DropdownOption>
        ))}
      </Dropdown>
    </SettingRow>
  );
}

// the detections on a picture of the captured screen, and how current they are
function TargetLive({ status, fresh }: { status?: VisionStatus; fresh: boolean }) {
  const [expanded, setExpanded] = useState(false);
  // a 16:9 stand-in until the display is open
  const [width, height] = status?.viewport?.[0] ? status.viewport : [1920, 1080];
  const targets = fresh ? status!.last!.detections : [];
  const seen = targets.filter(
    (target) => targetCircle(target.bounds, width, height) !== null,
  ).length;
  return (
    <div className={`vision-live${expanded ? " preview-expanded" : ""}`}>
      <div className="vision-preview">
        <TargetStage
          width={width}
          height={height}
          targets={targets}
          label="Detected target circles in captured viewport"
          empty={
            <div className="target-empty">
              <Crosshair size={18} aria-hidden="true" />
              <strong>
                {!status?.ready
                  ? "Waiting for capture"
                  : !fresh
                    ? "No recent screen frame"
                    : "No target detected"}
              </strong>
              {fresh && <span>bring the target into view</span>}
            </div>
          }
        />
        <div className="target-caption">
          <span>
            {width} × {height}
          </span>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={expanded ? "Compact target preview" : "Expand target preview"}
            aria-pressed={expanded}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? <Minimize2 /> : <Maximize2 />}
          </Button>
        </div>
      </div>
      <div className="vision-readouts">
        <Rates status={status} fresh={fresh} />
        <dl className="readout">
          <dt>Frame age</dt>
          <dd>{status?.frame_age_ms == null ? "-" : `${Math.round(status.frame_age_ms)} ms`}</dd>
          <dt>Targets</dt>
          <dd>
            <Items>
              <span>{seen}</span>
              {seen > 1 && (
                <span className="text-destructive">
                  events only work with exactly one target on screen
                </span>
              )}
            </Items>
          </dd>
        </dl>
      </div>
    </div>
  );
}

// how fast frames come and go now, on average and at their worst over the last ten seconds
function Rates({ status, fresh }: { status?: VisionStatus; fresh: boolean }) {
  const rows = [
    ["Analyzed", status?.fps, status?.analyzed, "fps"],
    ["Captured", status?.capture_fps, status?.captured, "fps"],
    ["Pipeline", status?.last?.pipeline_ms, status?.pipeline, "ms"],
  ] as const;
  // a rate measured over a frame the worker no longer has is not a current rate
  const shown = (value: number | null | undefined) =>
    fresh && value != null ? value.toFixed(1) : "-";
  return (
    <table className="rate-readout">
      <thead>
        <tr>
          <td />
          <th scope="col">Now</th>
          <th scope="col">10 s avg</th>
          <th scope="col">Worst 1%</th>
          <td />
        </tr>
      </thead>
      <tbody>
        {rows.map(([label, now, spread, unit]) => (
          <tr key={label}>
            <th scope="row">{label}</th>
            <td>{shown(now)}</td>
            <td>{shown(spread?.average)}</td>
            <td>{shown(spread?.worst)}</td>
            <td>{unit}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
