import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Pause, Play } from "lucide-react";
import { Notice } from "@/components/notice";
import { Button } from "@/components/ui/button";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { TargetStage } from "@/components/target-stage";
import { colors } from "@/charts/chart";
import { frameAt, trail } from "@/lib/replay";
import type { Replay, Run } from "@/types";
import { RunDialog, type ShownRun } from "./run-dialog";

const seconds = (ms: number) => `${(ms / 1000).toFixed(2)} s`;

/** whether a recording was made with target capture, so it has a screen to play back */
export function hadTargets(run: Run) {
  const participants = run.metadata.participants as { id: string }[] | undefined;
  return !!participants?.some((p) => p.id === "vision");
}

/** a recording played back: what target capture saw, the crosshair and where it has been */
export function ReplayRun({ shown, onClose }: { shown: ShownRun | null; onClose: () => void }) {
  return (
    <RunDialog shown={shown} title="Replay" className="sm:max-w-4xl" onClose={onClose}>
      {(run) => <ReplayPlayer key={run.id} run={run} />}
    </RunDialog>
  );
}

function ReplayPlayer({ run }: { run: Run }) {
  const [replay, setReplay] = useState<Replay | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    invoke<Replay>("run_replay", { id: run.id })
      .then((loaded) => active && setReplay(loaded))
      .catch((e) => active && setError(String(e)));
    return () => {
      active = false;
    };
  }, [run.id]);
  if (error) return <Notice error>{error}</Notice>;
  if (!replay)
    return (
      <p className="replay-loading field-description" aria-busy="true">
        Loading the recording…
      </p>
    );
  return <Player replay={replay} />;
}

function Player({ replay }: { replay: Replay }) {
  const end = Math.max(1, replay.mouse.length - 1);
  const [now, setNow] = usePlayback(end);
  const [trailOn, setTrailOn] = useState(true);
  const [trailSeconds, setTrailSeconds] = useState(1);
  const frame = frameAt(replay, now.at);
  const clicked = replay.clicks.some((click) => click <= now.at && now.at - click < 120);
  return (
    <div className="replay">
      <p className="field-description">
        the screen as target capture saw it, with the crosshair in the middle and its path over the
        last seconds. the path only lines up when the sensitivity and FOV in Target capture match
        the game
      </p>
      <TargetStage
        width={replay.width}
        height={replay.height}
        targets={(frame?.boxes ?? []).map((bounds) => ({ bounds }))}
        crosshair="dot"
        label="Replayed screen with its targets and the crosshair"
      >
        {trailOn && (
          <path
            d={trail(replay, now.at, trailSeconds)}
            fill="none"
            stroke={colors.candidate}
            strokeOpacity={0.8}
            strokeWidth={2}
            vectorEffect="non-scaling-stroke"
            data-testid="replay-trail"
          />
        )}
        {clicked && (
          <circle
            cx={replay.width / 2}
            cy={replay.height / 2}
            r={replay.width / 80}
            fill="none"
            stroke="var(--color-foreground)"
            strokeWidth={2}
            vectorEffect="non-scaling-stroke"
            data-testid="replay-click"
          />
        )}
      </TargetStage>
      <div className="replay-controls">
        <Button
          size="icon-sm"
          variant="outline"
          aria-label={now.playing ? "Pause" : "Play"}
          onClick={() => setNow({ playing: !now.playing })}
        >
          {now.playing ? <Pause /> : <Play />}
        </Button>
        <Slider
          aria-label="Replay time"
          min={0}
          max={end}
          step={1}
          value={[now.at]}
          onValueChange={([at]) => setNow({ at, playing: false })}
        />
        <span className="replay-time">
          {seconds(now.at)} / {seconds(end)}
        </span>
        <span className="replay-speed">
          <Dropdown
            aria-label="Replay speed"
            size="sm"
            value={now.speed}
            onValueChange={(speed) => setNow({ speed: Number(speed) })}
          >
            {[0.1, 0.25, 0.5, 1].map((speed) => (
              <DropdownOption key={speed} value={speed}>
                {speed}×
              </DropdownOption>
            ))}
          </Dropdown>
        </span>
      </div>
      <div className="replay-controls">
        <Switch id="replay-trail" checked={trailOn} onCheckedChange={setTrailOn} />
        <label htmlFor="replay-trail">Trail</label>
        <Slider
          aria-label="Trail length"
          min={0.1}
          max={5}
          step={0.1}
          disabled={!trailOn}
          value={[trailSeconds]}
          onValueChange={([value]) => setTrailSeconds(value)}
        />
        <span className="replay-time">last {trailSeconds.toFixed(1)} s</span>
      </div>
    </div>
  );
}

type Playback = { at: number; playing: boolean; speed: number };

/** the replay's clock in ms, moved on by the screen's own frames while it plays and stopped at
 * `end` */
function usePlayback(end: number) {
  const [state, setState] = useState<Playback>({
    at: 0,
    playing: false,
    speed: 1,
  });
  const last = useRef<number | null>(null);
  useEffect(() => {
    if (!state.playing) return;
    let frame = requestAnimationFrame(function step(time) {
      const before = last.current ?? time;
      last.current = time;
      setState((s) => {
        const at = Math.min(end, s.at + (time - before) * s.speed);
        return { ...s, at, playing: at < end };
      });
      frame = requestAnimationFrame(step);
    });
    return () => {
      cancelAnimationFrame(frame);
      last.current = null;
    };
  }, [state.playing, end]);
  const set = (change: Partial<Playback>) =>
    setState((s) => {
      const next = { ...s, ...change };
      // playing from the end starts over
      if (change.playing && s.at >= end) next.at = 0;
      return next;
    });
  return [state, set] as const;
}
