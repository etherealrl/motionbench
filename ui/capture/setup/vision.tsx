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

// a number setting: key, label, unit, decimals shown, what it means
type NumberSetting = [keyof VisionConfig, string, string | null, number, string?];
const projection: NumberSetting[] = [
  ["sensitivity", "Sensitivity", null, 2],
  ["horizontal_fov", "Horizontal FOV", "°", 4],
  ["yaw", "Yaw", "° per count", 4],
  ["pitch", "Pitch", "° per count", 4],
];
const targetClass: NumberSetting = ["target_class", "Target class", null, 0, "-1 takes any class"];
const detector: NumberSetting[] = [["confidence", "Minimum confidence", null, 2]];
const hardware: NumberSetting[] = [
  ["output", "Display", null, 0, "which display gets captured, by its index"],
  ["adapter", "Capture GPU", null, 0, "which GPU does the capturing, by its index"],
  ["cuda_device", "CUDA device", null, 0],
];

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

type Update = (key: keyof VisionConfig, value: string | number | boolean) => void;
type Command = (name: string, args?: Record<string, unknown>) => Promise<unknown>;

// the saved target settings, and starting or stopping the worker with them
function useTargetCapture(status: VisionStatus | undefined, command: Command, reveal: () => void) {
  const config = useSettings().vision;
  const [pending, setPending] = useState(false);
  const running = status?.running ?? false;
  const active = running || !!status?.error;
  const fresh = currentFrame(status);
  const update: Update = (key, value) => updateSettings({ vision: { ...config, [key]: value } });
  const toggle = async () => {
    if (pending) return;
    // starting without a scenario still previews, the open settings show what is missing
    if (!active && !config.scenario.trim()) reveal();
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
    () => onOpenChange(true),
  );
  const estimate = useDelayEstimate(command);
  const locked = running || recording || pending;
  const state = captureState(status, pending, config.provider, fresh);
  return (
    <SetupSection
      title="Target capture"
      open={expanded}
      onOpenChange={onOpenChange}
      status={state && <Badge variant={state.tone}>{state.label}</Badge>}
      hint={(!pending && blockerHint(blocker)) || undefined}
      action={
        <StartStop
          active={active}
          pending={pending}
          disabled={!native || pending || blocker !== null}
          onClick={() => void toggle()}
        />
      }
      aside={active && <TargetLive status={status} config={config} fresh={fresh} />}
    >
      {status?.error && <Notice error>{status.error}</Notice>}
      {running && !recording && (
        <p className="field-description">
          stop target capture first if you want to change its settings
        </p>
      )}
      <TargetSettings config={config} update={update} locked={locked} native={native} />
      <StartOnLaunch />
      <details className="advanced-settings">
        <Summary>Projection, detector and hardware</Summary>
        <Projection config={config} update={update} locked={locked} />
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
  const { start_on_launch } = useSettings().targets;
  return (
    <div className="setting-group">
      <SettingRow
        id="targets-on-launch"
        label="Start with the app"
        description="starts target capture whenever Motion Bench opens, so it's ready by the time you are"
      >
        <Switch
          id="targets-on-launch"
          checked={start_on_launch}
          onCheckedChange={(start) => updateSettings({ targets: { start_on_launch: start } })}
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

function Projection({
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
      <SettingSection title="Projection">
        <div className="setting-group">{rows(projection)}</div>
      </SettingSection>
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

// a text field, so a decimal reads with a point like the readouts whatever the system's locale;
// what is typed stays as typed until the field loses focus
function NumberRow({
  setting: [key, label, unit, decimals, help],
  config,
  update,
}: {
  setting: NumberSetting;
  config: VisionConfig;
  update: Update;
}) {
  const [typed, setTyped] = useState<string | null>(null);
  return (
    <SettingRow id={`target-${key}`} label={label} description={help}>
      <InputGroup>
        <InputGroupInput
          id={`target-${key}`}
          inputMode="decimal"
          value={typed ?? String(Number(Number(config[key]).toFixed(decimals)))}
          onChange={(e) => setTyped(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
          // saved when the field is left, a half written value would be refused
          onBlur={() => {
            const value = Number(typed?.replace(",", "."));
            if (typed?.trim() && Number.isFinite(value)) update(key, value);
            setTyped(null);
          }}
        />
        {unit && <InputGroupAddon align="inline-end">{unit}</InputGroupAddon>}
      </InputGroup>
    </SettingRow>
  );
}

// the detections on a picture of the captured screen, and how current they are
function TargetLive({
  status,
  config,
  fresh,
}: {
  status?: VisionStatus;
  config: VisionConfig;
  fresh: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const [width, height] = status?.viewport?.[0] ? status.viewport : [config.width, config.height];
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
