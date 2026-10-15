import { useEffect, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Notice } from "@/components/notice";
import { Button } from "@/components/ui/button";
import { DialogFooter } from "@/components/ui/dialog";
import { Slider } from "@/components/ui/slider";
import { Items } from "@/components/inline";
import { Timeline } from "@/charts/timeline";
import type { Plot, Run } from "@/types";
import { RunDialog, type ShownRun } from "./run-dialog";

// the slider moves in 10 ms, the shortest crop native accepts
const STEP = 0.01;

const seconds = (value: number) => `${value.toFixed(2)} s`;

/** how much of a run its crop keeps, for the table */
export function cropLength(run: Run) {
  return run.crop ? `Cropped to ${seconds((run.crop.to_us - run.crop.from_us) / 1e6)}` : null;
}

/** the span of a recording every analysis reads, chosen over the whole recording's timeline */
export function CropRun({ shown, onClose }: { shown: ShownRun | null; onClose: () => void }) {
  return (
    <RunDialog shown={shown} title="Crop" className="sm:max-w-3xl" onClose={onClose}>
      {(run) => <CropEditor key={run.id} run={run} onDone={onClose} />}
    </RunDialog>
  );
}

function CropEditor({ run, onDone }: { run: Run; onDone: () => void }) {
  const [plot, setPlot] = useState<Plot | null>(null);
  const [value, setValue] = useState<number[] | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    invoke<Plot>("run_plot", {
      id: run.id,
      range: null,
      scale: null,
      uncropped: true,
    })
      .then((whole) => {
        if (!active) return;
        setPlot(whole);
        const end = length(whole);
        setValue(
          run.crop
            ? [(run.crop.from_us - whole.from_us) / 1e6, (run.crop.to_us - whole.from_us) / 1e6]
            : [0, end],
        );
      })
      .catch((e) => active && setError(String(e)));
    return () => {
      active = false;
    };
  }, [run.id]);
  const end = plot ? length(plot) : 0;
  const whole = !!value && value[0] <= 0 && value[1] >= end - STEP / 2;
  const save = async () => {
    if (!plot || !value) return;
    setPending(true);
    setError(null);
    try {
      // native clamps the end to the last report, so the slider's last step past it is fine
      await invoke("set_crop", {
        id: run.id,
        crop: whole
          ? null
          : {
              from_us: plot.from_us + Math.round(value[0] * 1e6),
              to_us: plot.from_us + Math.round(value[1] * 1e6),
            },
      });
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  };
  return (
    <>
      <p className="field-description">
        every analysis of this recording only reads the span you keep here, the recording's own
        files don't change
      </p>
      {plot && value ? (
        <div className="crop-editor">
          <Timeline
            plot={plot}
            label="whole recording"
            hover={hover}
            onHover={setHover}
            onBrush={([from, to]) => setValue([from, Math.min(end, to)])}
          />
          <div className="crop-range">
            <Slider
              labels={["Crop start", "Crop end"]}
              min={0}
              max={end}
              step={STEP}
              minStepsBetweenThumbs={1}
              value={value}
              onValueChange={setValue}
            />
            <CrowdedMarks run={run} plot={plot} end={end} />
          </div>
          <Items>
            <span>From {seconds(value[0])}</span>
            <span>to {seconds(value[1])}</span>
            <span>{seconds(value[1] - value[0])} kept</span>
          </Items>
        </div>
      ) : (
        !error && (
          <p className="crop-loading field-description" aria-busy="true">
            Loading the recording…
          </p>
        )
      )}
      {error && <Notice error>{error}</Notice>}
      <DialogFooter>
        <Button
          variant="ghost"
          className="sm:mr-auto"
          disabled={!value || whole || pending}
          onClick={() => setValue([0, end])}
        >
          Reset
        </Button>
        <Button variant="outline" disabled={pending} onClick={onDone}>
          Cancel
        </Button>
        <Button disabled={!value || pending} onClick={() => void save()}>
          {pending ? "Saving…" : "Save"}
        </Button>
      </DialogFooter>
    </>
  );
}

// the whole recording in seconds, up to the slider step at or past its last report
function length(plot: Plot) {
  return Math.ceil((plot.to_us - plot.from_us) / 1e6 / STEP) * STEP;
}

// where `us` falls on the slider's track
function percent(plot: Plot, end: number, us: number) {
  return Math.min(100, Math.max(0, ((us - plot.from_us) / 1e6 / end) * 100));
}

/** where target capture saw more than one target, marked on the track */
function CrowdedMarks({ run, plot, end }: { run: Run; plot: Plot; end: number }) {
  const [spans, setSpans] = useState<[number, number][]>([]);
  useEffect(() => {
    let active = true;
    // only a help for choosing, the crop works without it
    invoke<[number, number][]>("run_crowded", { id: run.id })
      .then((found) => active && setSpans(found))
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [run.id]);
  if (!spans.length) return null;
  return (
    <div className="crop-crowded" aria-hidden>
      {spans.map(([from, to]) => (
        <span
          key={from}
          title="more than one target on screen, a trial here can't count"
          style={
            {
              "--from": percent(plot, end, from),
              "--to": percent(plot, end, to),
            } as CSSProperties
          }
        />
      ))}
    </div>
  );
}
