import { flushSettings, useSettings, updateSettings } from "@/lib/settings";
import { useState } from "react";
import { Circle, CircleStop, Play } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { Label } from "@/components/ui/label";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Badge } from "@/components/ui/badge";
import type { AimTuning, Blocker, ExperimentStatus, Snapshot } from "@/types";
import { MouseBinding } from "@/components/mouse-binding";
import { Notice } from "@/components/notice";
import { Items } from "@/components/inline";
import { Summary } from "@/components/summary";
import { fixHint } from "@/lib/text";
import { aimMessage, aimState, fixedInCapture, type Aim } from "./aim";

/** the controller's failure, else the buttons' trouble, which does not stop a recording */
function AimNotices({ status }: { status?: ExperimentStatus }) {
  if (status?.error) return <Notice error>{status.error}</Notice>;
  if (status?.buttons_error) return <Notice>{status.buttons_error}</Notice>;
  return null;
}

export function ExperimentPanel({
  snapshot,
  native,
  busy,
  command,
  record,
  capture,
}: {
  snapshot: Snapshot;
  native: boolean;
  busy: boolean;
  command: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  record: () => Promise<unknown>;
  capture: () => void;
}) {
  const { speed, deadzone_counts } = useSettings().aim;
  const config = { speed, deadzone_counts };
  const setConfig = (aim: AimTuning) => updateSettings({ aim });
  const [pending, setPending] = useState(false);
  const aim = aimState(snapshot, native);
  const locked = pending || busy;
  // the settings an armed aim or a recording uses stay readable but cannot change
  const inUse = aim.armed || aim.recording;
  const status = aim.status;
  const send = async (name: string, args?: Record<string, unknown>) => {
    setPending(true);
    try {
      // arming uses the saved settings, so a change not saved yet stops it
      if (name === "experiment_arm" && !(await flushSettings())) return;
      await command(name, args);
    } finally {
      setPending(false);
    }
  };
  return (
    <section aria-label="Linear aim experiment">
      <div className="section-intro">
        <div className="flex items-center gap-2">
          <h2>Linear aim</h2>
          {aim.armed && (
            <Badge variant={status?.active ? "success" : "warning"}>
              {status?.active ? "Moving" : "Armed"}
            </Badge>
          )}
        </div>
      </div>
      <div className="panel experiment-panel">
        <div className="aim-form">
          <AimSettings config={config} setConfig={setConfig} inUse={inUse} disabled={locked} />
          <AimButton
            aim={aim}
            blocker={snapshot.readiness.aim_button}
            locked={inUse}
            disabled={!native || locked}
            onPick={() => void send("experiment_learn", { cancel: aim.learning })}
          />
        </div>
        <div className="experiment-footer">
          <AimActions
            aim={aim}
            disabled={!native || locked}
            onArm={() => void send(aim.armed ? "experiment_disarm" : "experiment_arm")}
            onRecord={() => void record()}
          />
          <AimStatus aim={aim} mask={status?.hotkey_mask ?? 0} capture={capture} />
        </div>
        <AimNotices status={status} />
        <details className="advanced-settings experiment-diagnostics">
          <Summary>Activity details</Summary>
          <dl className="readout">
            <dt>Commands sent</dt>
            <dd>{(status?.commands ?? 0).toLocaleString()}</dd>
            <dt>Target offset</dt>
            <dd>
              {status?.target_counts
                ? `${status.target_counts.map((v) => v.toFixed(1)).join(" / ")} counts`
                : "-"}
            </dd>
          </dl>
        </details>
      </div>
    </section>
  );
}

// the aim button's binding, and why it cannot change when that is something to fix
function AimButton({
  aim,
  blocker,
  locked,
  disabled,
  onPick,
}: {
  aim: Aim;
  blocker: Blocker | null;
  locked: boolean;
  disabled: boolean;
  onPick: () => void;
}) {
  // armed, the note under the actions already says to disarm
  const hint = aim.armed ? null : fixHint(blocker);
  return (
    <>
      <Label>Aim button</Label>
      <div className="aim-form-note aim-binding">
        <MouseBinding
          label="Aim button"
          mask={aim.status?.hotkey_mask ?? 0}
          learning={aim.learning}
          locked={locked}
          disabled={disabled || blocker !== null}
          onPick={onPick}
        />
      </div>
      {hint && <p className="field-description aim-form-note">{hint}</p>}
    </>
  );
}

// one primary action at a time: Arm until armed, then Record, then Finish
function AimActions({
  aim,
  disabled,
  onArm,
  onRecord,
}: {
  aim: Aim;
  disabled: boolean;
  onArm: () => void;
  onRecord: () => void;
}) {
  const { armed, recording, canRecord } = aim;
  return (
    <div className="session-actions">
      <Button
        variant={armed || !aim.native || aim.arm ? "outline" : "default"}
        disabled={disabled || aim.arm !== null}
        onClick={onArm}
      >
        {!armed && <Play />}
        {armed ? "Disarm" : "Arm"}
      </Button>
      <Button
        variant={recording || canRecord ? "default" : "outline"}
        disabled={disabled || (recording ? aim.record !== null : !canRecord)}
        onClick={onRecord}
      >
        {recording ? <CircleStop /> : <Circle />}
        {recording ? "Finish recording" : "Record"}
      </Button>
    </div>
  );
}

function AimSettings({
  config,
  setConfig,
  inUse,
  disabled,
}: {
  config: AimTuning;
  setConfig: (next: AimTuning) => void;
  inUse: boolean;
  disabled: boolean;
}) {
  return (
    <>
      <NumberSetting
        id="aim-speed"
        label="Speed"
        unit="counts/s"
        name="Speed in counts per second"
        slider="Linear speed"
        min={100}
        max={20000}
        step={100}
        value={config.speed}
        inUse={inUse}
        disabled={disabled}
        onChange={(speed) => setConfig({ ...config, speed })}
      />
      <NumberSetting
        id="aim-deadzone"
        label="Deadzone"
        unit="counts"
        name="Deadzone in counts"
        slider="Linear deadzone"
        min={0}
        max={100}
        step={1}
        value={config.deadzone_counts}
        inUse={inUse}
        disabled={disabled}
        onChange={(deadzone_counts) => setConfig({ ...config, deadzone_counts })}
      />
      <p className="field-description aim-form-note">
        inside it the aim holds still, so it doesn't wobble around the target center while target
        frames catch up
      </p>
    </>
  );
}

function AimStatus({ aim, mask, capture }: { aim: Aim; mask: number; capture: () => void }) {
  const message = aim.learning ? null : aimMessage(aim, mask);
  return (
    <div className="experiment-status" role="status">
      {(message || aim.armed) && (
        <Items>
          {message && <span>{message}</span>}
          {aim.armed && <span>disarm first if you want to change the settings</span>}
        </Items>
      )}
      {aim.native && !aim.learning && fixedInCapture(aim.waiting) && (
        <Button variant="link" size="sm" onClick={capture}>
          Capture setup
        </Button>
      )}
    </div>
  );
}

// one row of the aim form: the label, a slider and the exact number with its unit
function NumberSetting({
  id,
  label,
  unit,
  name,
  slider,
  min,
  max,
  step,
  value,
  inUse,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  unit: string;
  name: string;
  slider: string;
  min: number;
  max: number;
  step: number;
  value: number;
  inUse: boolean;
  disabled: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <>
      <Label htmlFor={id}>{label}</Label>
      <Slider
        aria-label={slider}
        min={min}
        max={max}
        step={step}
        value={[value]}
        disabled={inUse || disabled}
        onValueChange={([next]) => {
          if (Number.isFinite(next)) onChange(next);
        }}
      />
      <InputGroup data-disabled={disabled || undefined}>
        <InputGroupInput
          id={id}
          aria-label={name}
          key={value}
          type="number"
          min={min}
          max={max}
          step={step}
          defaultValue={value}
          readOnly={inUse}
          disabled={disabled}
          className="text-right"
          onBlur={(event) => {
            if (inUse) return;
            // the limits are native's, a refused value shows the saved one again
            const typed = Number(event.target.value);
            if (event.target.value === "" || !Number.isFinite(typed)) {
              event.target.value = String(value);
              return;
            }
            onChange(typed);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
          }}
        />
        <InputGroupAddon align="inline-end">{unit}</InputGroupAddon>
      </InputGroup>
    </>
  );
}
