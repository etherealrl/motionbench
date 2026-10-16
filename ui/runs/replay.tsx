import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Pause, Play, Settings2 } from "lucide-react";
import { Notice } from "@/components/notice";
import { Button } from "@/components/ui/button";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Slider } from "@/components/ui/slider";
import { TargetStage } from "@/components/target-stage";
import { colors } from "@/charts/chart";
import { frameAt, trail } from "@/lib/replay";
import type { Replay, Run } from "@/types";

const seconds = (ms: number) => `${(ms / 1000).toFixed(2)} s`;

/** whether a recording was made with target capture, so it has a screen to play back */
export function hadTargets(run: Run) {
  const participants = run.metadata.participants as { id: string }[] | undefined;
  return !!participants?.some((p) => p.id === "vision");
}

/** a recording's replay, loaded when it was made with target capture */
export function useReplay(run: Run) {
  const [replay, setReplay] = useState<Replay | null>(null);
  const [error, setError] = useState<string | null>(null);
  const has = hadTargets(run);
  useEffect(() => {
    if (!has) return;
    let active = true;
    invoke<Replay>("run_replay", { id: run.id })
      .then((loaded) => active && setReplay(loaded))
      .catch((e) => active && setError(String(e)));
    return () => {
      active = false;
    };
  }, [run.id, has]);
  return { has, replay, error };
}

/** how long a replay runs, in ms */
export const replayEnd = (replay: Replay | null) => Math.max(1, (replay?.mouse.length ?? 2) - 1);

/** the replay's screen while it loads or once it's there */
export function ReplayScreen({
  replay,
  error,
  playback,
  kept,
  marks,
  below,
}: {
  replay: Replay | null;
  error: string | null;
  playback: ReturnType<typeof usePlayback>;
  /** the span a crop keeps, in the replay's ms, drawn on its track */
  kept?: [number, number] | null;
  /** marks over the track, like where target capture saw several targets */
  marks?: ReactNode;
  /** what goes right under the track, like the buttons that set the crop from it */
  below?: ReactNode;
}) {
  if (error) return <Notice error>{error}</Notice>;
  if (!replay)
    return (
      <p className="replay-loading field-description" aria-busy="true">
        Loading the replay…
      </p>
    );
  return (
    <Player replay={replay} playback={playback} kept={kept ?? null} marks={marks} below={below} />
  );
}

/** a recording played back: what target capture saw, the crosshair and where it has been */
function Player({
  replay,
  playback: [now, setNow],
  kept,
  marks,
  below,
}: {
  replay: Replay;
  playback: ReturnType<typeof usePlayback>;
  kept: [number, number] | null;
  marks: ReactNode;
  below: ReactNode;
}) {
  const end = replayEnd(replay);
  // none at 0
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
        {trailSeconds > 0 && (
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
        <span className="replay-track">
          <Slider
            aria-label="Replay time"
            min={0}
            max={end}
            step={1}
            value={[now.at]}
            onValueChange={([at]) => setNow({ at, playing: false })}
          />
          {marks}
          {kept && <KeptSpan kept={kept} end={end} />}
        </span>
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
        <TrailSettings seconds={trailSeconds} onChange={setTrailSeconds} />
      </div>
      {below}
    </div>
  );
}

/** how much of the crosshair's path stays drawn, tucked away since it's set once */
function TrailSettings({
  seconds,
  onChange,
}: {
  seconds: number;
  onChange: (seconds: number) => void;
}) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button size="icon-sm" variant="outline" aria-label="Replay settings">
          <Settings2 />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end">
        <div className="replay-settings">
          <span className="replay-settings-head">
            <span>Trail</span>
            <span className="replay-time">
              {seconds > 0 ? `last ${seconds.toFixed(1)} s` : "off"}
            </span>
          </span>
          <Slider
            aria-label="Trail length"
            min={0}
            max={5}
            step={0.1}
            value={[seconds]}
            onValueChange={([value]) => onChange(value)}
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}

/** the span a crop keeps, over the replay's track */
function KeptSpan({ kept: [from, to], end }: { kept: [number, number]; end: number }) {
  const at = (ms: number) => Math.min(100, Math.max(0, (ms / end) * 100));
  return (
    <span
      className="replay-kept"
      aria-hidden
      style={{ "--from": at(from), "--to": at(to) } as CSSProperties}
    />
  );
}

type Playback = { at: number; playing: boolean; speed: number };

/** the replay's clock in ms, moved on by the screen's own frames while it plays and stopped at
 * `end` */
export function usePlayback(end: number) {
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
