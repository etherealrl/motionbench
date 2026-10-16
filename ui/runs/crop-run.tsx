import { useEffect, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeftToLine, ArrowRightToLine, RotateCcw } from "lucide-react";
import { Notice } from "@/components/notice";
import { Button } from "@/components/ui/button";
import { DialogFooter } from "@/components/ui/dialog";
import { Slider } from "@/components/ui/slider";
import { Items } from "@/components/inline";
import { cn } from "@/lib/utils";
import { Summary } from "@/components/summary";
import { Timeline } from "@/charts/timeline";
import type { Plot, Replay, Run } from "@/types";
import { ReplayScreen, hadTargets, replayEnd, usePlayback, useReplay } from "./replay";
import { RunDialog, type ShownRun } from "./run-dialog";

// the slider moves in 10 ms, the shortest crop native accepts
const STEP = 0.01;

const seconds = (value: number) => `${value.toFixed(2)} s`;

/** how much of a run its crop keeps, for the table */
export function cropLength(run: Run) {
  return run.crop ? `Cropped to ${seconds((run.crop.to_us - run.crop.from_us) / 1e6)}` : null;
}

// the timeline folds out of the way, and stays the way it was left
const TIMELINE = "crop-timeline-open";

/** the span of a recording every analysis reads, chosen over the whole recording's timeline and,
 * with target capture, while its replay plays */
export function CropRun({ shown, onClose }: { shown: ShownRun | null; onClose: () => void }) {
  return (
    <RunDialog
      shown={shown}
      title={shown && hadTargets(shown.run) ? "Crop and replay" : "Crop"}
      className="sm:max-w-4xl"
      onClose={onClose}
    >
      {(run) => <CropEditor key={run.id} run={run} onDone={onClose} />}
    </RunDialog>
  );
}

/** the whole recording's plot, and the crop over it starting from the saved one */
function useWholePlot(run: Run, setError: (error: string) => void) {
  const [plot, setPlot] = useState<Plot | null>(null);
  const [value, setValue] = useState<number[] | null>(null);
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
  return { plot, value, setValue };
}

/** saves the crop, none when it keeps the whole recording */
function useSaveCrop(run: Run, setError: (error: string | null) => void, onDone: () => void) {
  const [pending, setPending] = useState(false);
  const save = async (plot: Plot, value: number[], whole: boolean) => {
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
  return { pending, save };
}

function CropEditor({ run, onDone }: { run: Run; onDone: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const { plot, value, setValue } = useWholePlot(run, setError);
  const { pending, save } = useSaveCrop(run, setError, onDone);
  const replay = useReplay(run);
  const crowded = useCrowded(run);
  const playback = usePlayback(replayEnd(replay.replay));
  const end = plot ? length(plot) : 0;
  const whole = !!value && value[0] <= 0 && value[1] >= end - STEP / 2;
  const { crop, cropAt, kept } = linkReplay(value, setValue, plot, replay.replay, playback, end);
  // the replay's own track sets the crop, so it needs no slider of its own
  const replayed = replay.has && !replay.error;
  return (
    <>
      <p className="field-description">
        every analysis of this recording only reads the span you keep here, the recording's own
        files don't change
      </p>
      {replay.has && (
        <CropReplay
          replay={replay}
          playback={playback}
          kept={kept}
          crowded={crowded}
          value={value}
          onCropAt={cropAt}
        />
      )}
      {plot && value && (
        <CropRange crowded={crowded} plot={plot} value={value} replayed={replayed} onCrop={crop} />
      )}
      <CropStatus loading={!plot} error={error} />
      <CropFooter
        pending={pending}
        canReset={!!value && !whole}
        onReset={() => setValue([0, end])}
        onCancel={onDone}
        onSave={plot && value ? () => void save(plot, value, whole) : null}
      />
    </>
  );
}

/** the recording loading, or why it couldn't be cropped */
function CropStatus({ loading, error }: { loading: boolean; error: string | null }) {
  if (error) return <Notice error>{error}</Notice>;
  if (!loading) return null;
  return (
    <p className="crop-loading field-description" aria-busy="true">
      Loading the recording…
    </p>
  );
}

/** the replay, its track marked with what the crop keeps and where there were several targets */
function CropReplay({
  replay,
  playback,
  kept,
  crowded,
  value,
  onCropAt,
}: {
  replay: ReturnType<typeof useReplay>;
  playback: ReturnType<typeof usePlayback>;
  kept: [number, number] | null;
  crowded: [number, number][];
  value: number[] | null;
  onCropAt: (edge: 0 | 1) => void;
}) {
  const shown = replay.replay;
  return (
    <ReplayScreen
      replay={shown}
      error={replay.error}
      playback={playback}
      kept={kept}
      marks={
        shown && (
          <CrowdedMarks
            spans={crowded}
            className="replay-crowded"
            at={(us) => ((us - shown.from_us) / 1000 / replayEnd(shown)) * 100}
          />
        )
      }
      below={value && <CropSummary value={value} onCropAt={onCropAt} />}
    />
  );
}

function CropFooter({
  pending,
  canReset,
  onReset,
  onCancel,
  onSave,
}: {
  pending: boolean;
  canReset: boolean;
  onReset: () => void;
  onCancel: () => void;
  onSave: (() => void) | null;
}) {
  return (
    <DialogFooter>
      <Button
        variant="ghost"
        className="sm:mr-auto"
        disabled={!canReset || pending}
        onClick={onReset}
      >
        <RotateCcw />
        Reset
      </Button>
      <Button variant="outline" disabled={pending} onClick={onCancel}>
        Cancel
      </Button>
      <Button disabled={!onSave || pending} onClick={() => onSave?.()}>
        {pending ? "Saving…" : "Save"}
      </Button>
    </DialogFooter>
  );
}

/** the crop over the whole recording's timeline, folded away when the replay is there, which
 * also takes over the slider */
function CropRange({
  crowded,
  plot,
  value,
  replayed,
  onCrop,
}: {
  crowded: [number, number][];
  plot: Plot;
  value: number[];
  replayed: boolean;
  onCrop: (value: number[]) => void;
}) {
  const end = length(plot);
  const fold = (
    <TimelineFold
      plot={plot}
      folded={replayed}
      onBrush={([from, to]) => onCrop([from, Math.min(end, to)])}
    />
  );
  if (replayed) return <div className="crop-editor">{fold}</div>;
  return (
    <div className="crop-editor">
      {fold}
      <div className="crop-range">
        <Slider
          labels={["Crop start", "Crop end"]}
          min={0}
          max={end}
          step={STEP}
          minStepsBetweenThumbs={1}
          value={value}
          onValueChange={onCrop}
        />
        <CrowdedMarks
          spans={crowded}
          at={(us) => Math.min(100, Math.max(0, ((us - plot.from_us) / 1e6 / end) * 100))}
        />
      </div>
      <CropSummary value={value} onCropAt={null} />
    </div>
  );
}

/** the crop and the replay moved together. the crop counts seconds from the plot's first report
 * and the replay ms from the first one it could read, which can come a little later, so both go
 * through the system clock they share */
function linkReplay(
  value: number[] | null,
  setValue: (value: number[]) => void,
  plot: Plot | null,
  replay: Replay | null,
  [now, setNow]: ReturnType<typeof usePlayback>,
  end: number,
) {
  // the replay shows the frame at the edge being moved, so you see where the crop starts or ends
  const crop = (next: number[]) => {
    if (value && plot && replay) {
      const moved = next[0] !== value[0] ? next[0] : next[1];
      const at = (plot.from_us + moved * 1e6 - replay.from_us) / 1000;
      setNow({ at: Math.min(replayEnd(replay), Math.max(0, at)), playing: false });
    }
    setValue(next);
  };
  // the kept span in the replay's ms, for its track
  const kept: [number, number] | null =
    value && plot && replay
      ? [
          (plot.from_us + value[0] * 1e6 - replay.from_us) / 1000,
          (plot.from_us + value[1] * 1e6 - replay.from_us) / 1000,
        ]
      : null;
  // the crop's edge set where the replay is
  const cropAt = (edge: 0 | 1) => {
    if (!value || !plot || !replay) return;
    const at = Math.min(end, Math.max(0, (replay.from_us + now.at * 1000 - plot.from_us) / 1e6));
    setValue(
      edge === 0
        ? [Math.min(at, value[1] - STEP), value[1]]
        : [value[0], Math.max(at, value[0] + STEP)],
    );
  };
  return { crop, cropAt, kept };
}

/** the whole recording's report intervals, folded away under their title and kept the way they
 * were left. `folded` is how it starts the first time */
function TimelineFold({
  plot,
  folded,
  onBrush,
}: {
  plot: Plot;
  folded: boolean;
  onBrush: (range: [number, number]) => void;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const [open, setOpen] = useState(() => {
    const kept = localStorage.getItem(TIMELINE);
    return kept === null ? !folded : kept === "true";
  });
  return (
    <details
      className="crop-timeline"
      open={open}
      onToggle={(e) => {
        // the browser also fires this when it first draws the timeline open, which isn't a choice
        if (e.currentTarget.open === open) return;
        setOpen(e.currentTarget.open);
        localStorage.setItem(TIMELINE, String(e.currentTarget.open));
      }}
    >
      <Summary>Report intervals over time</Summary>
      <Timeline
        plot={plot}
        label="whole recording"
        hover={hover}
        onHover={setHover}
        onBrush={onBrush}
      />
    </details>
  );
}

/** what the crop keeps, with the buttons that set an edge where the replay is when there is one */
function CropSummary({
  value,
  onCropAt,
}: {
  value: number[];
  onCropAt: ((edge: 0 | 1) => void) | null;
}) {
  return (
    <div className="crop-summary">
      <Items>
        <span>From {seconds(value[0])}</span>
        <span>to {seconds(value[1])}</span>
        <span>{seconds(value[1] - value[0])} kept</span>
      </Items>
      {onCropAt && (
        <span className="crop-at">
          <Button size="sm" variant="outline" onClick={() => onCropAt(0)}>
            <ArrowRightToLine />
            Start here
          </Button>
          <Button size="sm" variant="outline" onClick={() => onCropAt(1)}>
            <ArrowLeftToLine />
            End here
          </Button>
        </span>
      )}
    </div>
  );
}

// the whole recording in seconds, up to the slider step at or past its last report
function length(plot: Plot) {
  return Math.ceil((plot.to_us - plot.from_us) / 1e6 / STEP) * STEP;
}

/** where target capture saw more than one target, on the system clock. only a help for choosing,
 * so the crop works without it */
function useCrowded(run: Run) {
  const [spans, setSpans] = useState<[number, number][]>([]);
  useEffect(() => {
    let active = true;
    invoke<[number, number][]>("run_crowded", { id: run.id })
      .then((found) => active && setSpans(found))
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [run.id]);
  return spans;
}

/** where target capture saw more than one target, marked on a track, `at` placing a system clock
 * time on it in percent */
function CrowdedMarks({
  spans,
  at,
  className,
}: {
  spans: [number, number][];
  at: (us: number) => number;
  className?: string;
}) {
  if (!spans.length) return null;
  return (
    <div className={cn("crop-crowded", className)} aria-hidden>
      {spans.map(([from, to]) => (
        <span
          key={from}
          title="more than one target on screen, a trial here can't count"
          style={{ "--from": at(from), "--to": at(to) } as CSSProperties}
        />
      ))}
    </div>
  );
}
