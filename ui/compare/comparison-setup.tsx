import { Button } from "@/components/ui/button";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Input } from "@/components/ui/input";
import { DeviceLabel } from "@/components/device-label";
import { Summary } from "@/components/summary";
import { native } from "@/lib/native";
import type { Session } from "@/app/use-session";
import type { Run } from "@/types";
import type { Comparison } from "./use-comparison";
import { RecordingPicker } from "./recording-picker";
import { Notice } from "@/components/notice";
import { panelLabel, panelLabels } from "@/lib/panels";
import { cases, isCase, listed, recordedDifferences, tagged } from "@/lib/sides";
import { useSettings } from "@/lib/settings";

// why a paired comparison can't run on what's picked, the same words native would refuse it with
function pairedRefusal(runs: Run[], candidate: string[]) {
  if (candidate.length > 1)
    return `a paired comparison reads one recording against its own box input, so pick just one, ${candidate.length} are selected`;
  const run = runs.find((run) => run.id === candidate[0]);
  return run?.refusals.paired
    ? `${run.name} can't be compared this way, ${run.refusals.paired}`
    : null;
}

// what stands in the way of running, and whether a choice is still missing; a library load or a
// job in flight is only a wait
function readiness(snapshot: Session["snapshot"], comparison: Comparison) {
  const { paired, panel, baseline, candidate, usablePanel } = comparison;
  const refusal = paired ? pairedRefusal(snapshot.runs, candidate) : null;
  const blocked =
    !native ||
    !candidate.length ||
    (!paired && !baseline.length) ||
    !!refusal ||
    !usablePanel(panel);
  return { refusal, blocked };
}

export function ComparisonSetup({
  session,
  comparison,
}: {
  session: Session;
  comparison: Comparison;
}) {
  const { snapshot, libraryReady } = session;
  const { paired, matching, panel, baseline, candidate, usablePanel } = comparison;
  const running = snapshot.job?.status === "running";
  const { refusal, blocked } = readiness(snapshot, comparison);
  return (
    <section className="panel compare-setup">
      <fieldset disabled={running}>
        <div className="control-row">
          <label>
            Comparison mode
            <Dropdown
              value={paired ? "paired" : matching ? "matching" : "independent"}
              onValueChange={(value) => (
                comparison.setPaired(value === "paired"),
                comparison.setMatching(value === "matching")
              )}
            >
              <DropdownOption value="independent">Separate recordings</DropdownOption>
              <DropdownOption value="matching">Matching source events</DropdownOption>
              <DropdownOption value="paired">Physical input / wire output</DropdownOption>
            </Dropdown>
          </label>
          <label>
            Features
            <Dropdown value={panel} onValueChange={(value) => comparison.setPanel(value)}>
              {["texture19", "trajectory14", "full49"].map((p) => (
                <DropdownOption
                  key={p}
                  value={p}
                  disabled={candidate.length > 0 && !usablePanel(p)}
                >
                  {panelLabels[p]}
                </DropdownOption>
              ))}
            </Dropdown>
          </label>
        </div>
        <TaggedSides runs={snapshot.runs} comparison={comparison} />
        <Populations session={session} comparison={comparison} />
        <div className="control-row analysis-config">
          <JudgeSettings comparison={comparison} />
          <Button
            variant={blocked ? "outline" : "default"}
            disabled={blocked || !libraryReady || running}
            onClick={() => void comparison.analyze()}
          >
            Run comparison
          </Button>
        </div>
        {paired && (
          <p className="field-description">
            this tests the physical input against what the box sends out. the thing is injected
            movement adds stuff that input doesn't have, so to judge the injected trajectory itself
            use a matching reference population
          </p>
        )}
        {refusal && <Notice>{refusal}</Notice>}
        {comparison.selectedRun && comparison.selectedRun.missing[panel] && (
          <Notice>
            {panelLabel(panel)}: {comparison.selectedRun.missing[panel]}
          </Notice>
        )}
        {!candidate.length || (!paired && !baseline.length) ? (
          <p className="field-description">
            select a {candidate.length ? "baseline" : "candidate"} to run a comparison
          </p>
        ) : null}
      </fieldset>
    </section>
  );
}

/** fills the comparison with the recordings tagged with a side, when both sides have some */
function TaggedSides({ runs, comparison }: { runs: Run[]; comparison: Comparison }) {
  const kind = useSettings().display.purpose;
  if (!isCase(kind)) return null;
  const sides = tagged(runs, kind);
  const baseline = sides.filter((run) => run.tag?.side === "baseline").map((run) => run.id);
  const candidate = sides.filter((run) => run.tag?.side === "candidate").map((run) => run.id);
  if (!baseline.length || !candidate.length) return null;
  // only an assist acts on targets, the boxes are judged on the texture of the movement
  const assist = kind === "assist";
  const full = assist && sides.every((run) => run.panels.includes("full49"));
  const differs = assist ? recordedDifferences(sides) : [];
  const use = () => {
    comparison.setPaired(false);
    comparison.setMatching(false);
    comparison.setBaseline(baseline);
    comparison.setCandidate(candidate);
    comparison.setPanel(full ? "full49" : "texture19");
    comparison.setRange(null);
  };
  const names = cases[kind].sides;
  const notes = [
    `${baseline.length} ${names.baseline.toLowerCase()} against ${candidate.length} ${names.candidate.toLowerCase()}, as Separate recordings`,
    full
      ? "with Full49"
      : assist
        ? "with Texture19, since some of them don't have target events for Full49 (a recording that just ended gets them a moment later)"
        : "with Texture19",
  ];
  return (
    <div className="tagged-sides">
      <Button variant="outline" onClick={use}>
        Use the tagged sides
      </Button>
      <p className="field-description">
        {notes.join(" ")}
        {differs.length > 0 &&
          `. just so you know, the ${listed(differs)} isn't the same in all of them`}
      </p>
    </div>
  );
}

// the two populations; choosing a recording on one side takes it off the other
function Populations({ session, comparison }: { session: Session; comparison: Comparison }) {
  const { snapshot, call } = session;
  const { paired, baseline, candidate, setRange, setResult } = comparison;
  return (
    <>
      <div className="population-grid">
        {paired ? (
          <div className="population-card">
            <h3>Physical input</h3>
          </div>
        ) : (
          <RecordingPicker
            name="Baseline"
            disabled={snapshot.job?.status === "running"}
            runs={snapshot.runs}
            selected={baseline}
            onChange={(ids) => {
              comparison.setBaseline(ids);
              comparison.setCandidate((previous) => previous.filter((id) => !ids.includes(id)));
              setRange(null);
              setResult(null);
            }}
          />
        )}
        <RecordingPicker
          name="Candidate"
          disabled={snapshot.job?.status === "running"}
          runs={snapshot.runs}
          selected={candidate}
          onChange={(ids) => {
            comparison.setCandidate(ids);
            comparison.setBaseline((previous) => previous.filter((id) => !ids.includes(id)));
            setRange(null);
            setResult(null);
          }}
        />
      </div>
      {[...new Set([...baseline, ...candidate])]
        .map((id) => snapshot.runs.find((r) => r.id === id))
        .filter((run): run is Run => !!run && run.devices.length > 1)
        .map((run) => (
          <label key={run.id}>
            Endpoint: {run.name}
            <Dropdown
              value={run.selected ?? ""}
              onValueChange={(value) =>
                void call("select_run_device", {
                  id: run.id,
                  device: value,
                })
              }
              placeholder="Select endpoint"
            >
              {run.devices.map((device) => (
                <DropdownOption key={device.id} value={device.id}>
                  <DeviceLabel device={device} />
                </DropdownOption>
              ))}
            </Dropdown>
          </label>
        ))}
    </>
  );
}

function JudgeSettings({ comparison }: { comparison: Comparison }) {
  const { config, setConfig } = comparison;
  return (
    <details className="advanced-settings judge-settings">
      <Summary>Advanced settings</Summary>
      <div className="advanced-content">
        <label>
          Window
          <Dropdown
            value={config.width_ms}
            onValueChange={(value) =>
              setConfig({
                ...config,
                width_ms: Number(value),
              })
            }
          >
            {[160, 250, 500, 1000].map((v) => (
              <DropdownOption key={v} value={v}>
                {v} ms
              </DropdownOption>
            ))}
          </Dropdown>
        </label>
        <label className="comparison-label">
          Comparison label
          <Dropdown
            value={comparison.question}
            onValueChange={(value) => comparison.setQuestion(value)}
          >
            <DropdownOption>firmware / injection comparison</DropdownOption>
            <DropdownOption>real vs real control</DropdownOption>
            <DropdownOption>relay control</DropdownOption>
          </Dropdown>
        </label>
        <div className="control-row">
          <label>
            Repeats
            <Input
              type="number"
              min="1"
              max="20"
              value={config.repeats}
              onChange={(e) =>
                setConfig({
                  ...config,
                  repeats: Number(e.target.value),
                })
              }
            />
          </label>
          <label>
            Permutations
            <Input
              type="number"
              min="0"
              max="10000"
              value={config.permutations}
              onChange={(e) =>
                setConfig({
                  ...config,
                  permutations: Number(e.target.value),
                })
              }
            />
          </label>
          <label>
            Seed
            <Input
              type="number"
              min="0"
              value={config.seed}
              onChange={(e) =>
                setConfig({
                  ...config,
                  seed: Number(e.target.value),
                })
              }
            />
          </label>
        </div>
      </div>
    </details>
  );
}
