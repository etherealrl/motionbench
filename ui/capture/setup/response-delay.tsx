import { useState } from "react";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Items } from "@/components/inline";
import { Notice } from "@/components/notice";
import { SettingRow } from "@/components/setting-row";
import type { ResponseCalibration, Run, VisionConfig } from "@/types";

type Estimate = ResponseCalibration & { profile: VisionConfig };

/** an estimate being worked on: the chosen recording, its confirmation and the result. the section
 *  keeps it, so folding the section loses none of it */
export function useDelayEstimate(
  command: (name: string, args?: Record<string, unknown>) => Promise<unknown>,
) {
  const [id, setId] = useState("");
  const [stationary, setStationary] = useState(false);
  const [pending, setPending] = useState(false);
  const [result, setResult] = useState<Estimate | null>(null);
  const choose = (value: string) => {
    setId(value);
    setResult(null);
    setStationary(false);
  };
  const run = async () => {
    setPending(true);
    setResult(null);
    try {
      setResult(
        (await command("calibrate_response", {
          id,
          stationaryTarget: stationary,
        })) as Estimate,
      );
    } finally {
      setPending(false);
    }
  };
  return { id, choose, stationary, setStationary, pending, result, run };
}

/** the display delay target capture assumes, set by hand or estimated from a recording */
export function ResponseDelay({
  estimate: { id, choose, stationary, setStationary, pending, result, run },
  runs,
  locked,
  native,
  profile,
  apply,
}: {
  estimate: ReturnType<typeof useDelayEstimate>;
  runs: Run[];
  locked: boolean;
  native: boolean;
  profile: VisionConfig;
  apply: (delay: number) => void;
}) {
  return (
    <div className="setting-group" aria-label="Response delay" role="group">
      <SettingRow
        id="display-delay"
        label="Display delay"
        description="how long it takes for a mouse movement to actually show up on screen"
      >
        <InputGroup>
          <InputGroupInput
            id="display-delay"
            type="number"
            min={0}
            max={250}
            step={1}
            value={profile.display_delay_ms}
            disabled={locked}
            onChange={(e) => apply(Number(e.target.value))}
          />
          <InputGroupAddon align="inline-end">ms</InputGroupAddon>
        </InputGroup>
      </SettingRow>
      <SettingRow
        id="calibration-recording"
        label="Calibration recording"
        description="record 5 to 10 seconds with USB and target capture on, with one target that doesn't move in view, and move back and forth around it without clicking"
      >
        <Dropdown
          id="calibration-recording"
          value={id}
          disabled={locked || pending}
          onValueChange={choose}
          placeholder="Choose a recording"
        >
          {runs
            .filter((r) => r.path.endsWith(".pcap"))
            .map((r) => (
              <DropdownOption key={r.id} value={r.id}>
                {r.name}
              </DropdownOption>
            ))}
        </Dropdown>
      </SettingRow>
      <SettingRow
        id="stationary-target"
        label="Stationary target"
        description="the target didn't move and neither did the player while it was recorded"
      >
        <Switch
          id="stationary-target"
          checked={stationary}
          disabled={locked || pending}
          onCheckedChange={setStationary}
        />
      </SettingRow>
      <div className="setting-actions">
        <Button
          variant="outline"
          disabled={!native || locked || pending || !id || !stationary}
          onClick={() => void run()}
        >
          {pending ? "Estimating…" : "Estimate delay"}
        </Button>
      </div>
      {result && <EstimateResult result={result} profile={profile} locked={locked} apply={apply} />}
    </div>
  );
}

function EstimateResult({
  result,
  profile,
  locked,
  apply,
}: {
  result: Estimate;
  profile: VisionConfig;
  locked: boolean;
  apply: (delay: number) => void;
}) {
  const matching = (
    ["width", "height", "horizontal_fov", "sensitivity", "yaw", "pitch"] as const
  ).every((key) => result.profile[key] === profile[key]);
  const curveMax = Math.max(1, ...result.residual_curve.map((v) => v[1]));
  return (
    <div className="calibration-result" role="status">
      <strong>Mouse-to-screen offset: {result.estimated_delay_ms} ms best fit</strong>
      <p className="field-description">
        it lines up the recorded USB mouse movement with how far the stationary target moves on
        screen when you turn the camera. offsets from {result.admissible_delay_ms[0]} to{" "}
        {result.admissible_delay_ms[1]} ms fit about as well, so read that range as how unsure the
        offset is. it doesn't say how much the latency moves around or what the driver adds on its
        own, and it isn't a confidence interval either, just a tolerance band
      </p>
      <span className="field-description">
        <Items>
          <span>p90 residual {result.residual_counts.toFixed(1)} counts</span>
          <span>{result.frames} frames</span>
          <span>{(result.span_ms / 1000).toFixed(1)} seconds</span>
        </Items>
      </span>
      <svg viewBox="0 0 500 120" role="img" aria-label="Response delay residual curve">
        <title>residual by delay from 0 to 250 milliseconds, lower is better</title>
        <polyline
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          points={result.residual_curve
            .map(([x, y]) => `${x * 2},${95 - (85 * y) / curveMax}`)
            .join(" ")}
        />
        <line
          x1={result.estimated_delay_ms * 2}
          x2={result.estimated_delay_ms * 2}
          y1="5"
          y2="95"
          stroke="currentColor"
          strokeDasharray="4 4"
        />
        <text x="0" y="114" fontSize="10" fill="currentColor">
          0 ms
        </text>
        <text x="250" y="114" textAnchor="middle" fontSize="10" fill="currentColor">
          125 ms
        </text>
        <text x="500" y="114" textAnchor="end" fontSize="10" fill="currentColor">
          250 ms
        </text>
      </svg>
      {result.selected_range_seconds && (
        <span className="field-description">
          <Items>
            <span>
              used {result.selected_range_seconds.map((v) => v.toFixed(2)).join(" to ")} s of this
              recording
            </span>
            <span>{result.single_target_segments} uninterrupted single-target segments</span>
            <span>{result.excluded_samples} samples outside the fit</span>
          </Items>
        </span>
      )}
      {result.reason && <Notice>{result.reason}</Notice>}
      {!matching && (
        <Notice>
          the projection settings don't match this recording's, so match them first before you apply
          its estimate
        </Notice>
      )}
      <Button
        variant="outline"
        disabled={locked || !result.identifiable || !matching}
        onClick={() => apply(result.estimated_delay_ms)}
      >
        Use for future captures
      </Button>
    </div>
  );
}
