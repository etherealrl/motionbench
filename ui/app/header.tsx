import { useLayoutEffect, useRef, useState } from "react";
import {
  ALargeSmall,
  GitCompareArrows,
  Import,
  Mouse,
  Settings,
  Volume2,
  VolumeX,
  SunMoon,
} from "lucide-react";
import { InfoTip } from "@/components/info-tip";
import { Badge, type BadgeTone } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import { Slider } from "@/components/ui/slider";
import { TabsList, TabsTrigger } from "@/components/ui/tabs";
import { updateSettings, useSettings } from "@/lib/settings";
import { RecordingsFolder } from "@/app/recordings-folder";
import { native } from "@/lib/native";
import { recordingTime } from "@/capture/recording";
import { useTheme } from "@/lib/theme";
import type { Snapshot, Theme } from "@/types";
import { PurposePick } from "@/capture/comparing";

// the one global status, the most pressing first; none while nothing runs
function sessionStatus(snapshot: Snapshot): { label: string; tone: BadgeTone } | null {
  if (snapshot.readiness.record?.need === "broken")
    return { label: "Restart needed", tone: "danger" };
  if (snapshot.health.error) return { label: "Capture needs attention", tone: "danger" };
  if (snapshot.recording)
    return {
      label: `Recording ${recordingTime(snapshot.recording_elapsed_ms ?? 0)}`,
      tone: "danger",
    };
  if (snapshot.experiment?.armed) return { label: "Experiment armed", tone: "warning" };
  if (snapshot.live) return { label: "Monitoring", tone: "success" };
  return null;
}

export function AppHeader({
  snapshot,
  libraryReady,
  onImport,
  onError,
}: {
  snapshot: Snapshot;
  libraryReady: boolean;
  onImport: () => void;
  onError: (message: string) => void;
}) {
  const status = sessionStatus(snapshot);
  const ref = useStuckBottom();
  return (
    <header className="app-header" ref={ref}>
      <div className="brand">
        <Mouse size={20} aria-hidden />
        <h1>Motion Bench</h1>
        {!native && (
          <>
            <Badge variant="muted">Preview</Badge>
            <InfoTip label="About the preview">
              this is just a sample recording so you can see the charts, recording and the hardware
              controls only work in the desktop app
            </InfoTip>
          </>
        )}
      </div>
      <nav className="nav-row" aria-label="Bench views">
        <TabsList variant="line">
          <TabsTrigger value="capture">Capture</TabsTrigger>
          <TabsTrigger value="compare">Compare</TabsTrigger>
          {snapshot.developer_tools && <TabsTrigger value="experiment">Experiment</TabsTrigger>}
          <TabsTrigger value="runs">
            Runs
            {snapshot.runs.length > 0 && <Badge variant="muted">{snapshot.runs.length}</Badge>}
          </TabsTrigger>
        </TabsList>
      </nav>
      <div className="header-actions">
        {status && (
          <span className="global-session-status">
            <Badge variant={status.tone}>{status.label}</Badge>
          </span>
        )}
        <SettingsMenu recording={!!snapshot.recording} onError={onError} />
        <Button variant="outline" size="sm" disabled={!native || !libraryReady} onClick={onImport}>
          <Import /> Import
        </Button>
      </div>
    </header>
  );
}

function SettingsMenu({
  recording,
  onError,
}: {
  /** a recording takes its tag when it ends, so the comparison stays put until then */
  recording: boolean;
  onError: (message: string) => void;
}) {
  const display = useSettings().display;
  const zoom = display.zoom;
  const setZoom = (zoom: number) =>
    updateSettings((shown) => ({ display: { ...shown.display, zoom } }));
  useTheme(display.theme);
  // before the first paint, so a saved scale never jumps from another one
  useLayoutEffect(() => {
    document.documentElement.style.zoom = String(zoom);
    document.documentElement.style.setProperty("--ui-scale", String(zoom));
  }, [zoom]);
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label="Settings">
          <Settings />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="display-settings">
        <div className="settings-row">
          <span className="settings-icon">
            <ALargeSmall aria-hidden className="size-4" />
          </span>
          <Dropdown
            aria-label="UI scale"
            value={zoom}
            onValueChange={(value) => setZoom(Number(value))}
          >
            {[1, 1.1, 1.25, 1.5].map((v) => (
              <DropdownOption key={v} value={v}>
                {Math.round(v * 100)}%
              </DropdownOption>
            ))}
          </Dropdown>
        </div>
        <div className="settings-row">
          <span className="settings-icon">
            <SunMoon aria-hidden className="size-4" />
          </span>
          <Dropdown
            aria-label="Theme"
            value={display.theme}
            onValueChange={(theme) =>
              updateSettings((shown) => ({ display: { ...shown.display, theme: theme as Theme } }))
            }
          >
            <DropdownOption value="system">Same as Windows</DropdownOption>
            <DropdownOption value="light">Light</DropdownOption>
            <DropdownOption value="dark">Dark</DropdownOption>
          </Dropdown>
        </div>
        <div className="settings-row">
          <span className="settings-icon">
            <GitCompareArrows aria-hidden className="size-4" />
          </span>
          <PurposePick disabled={recording} />
        </div>
        <CueVolume />
        {native && <RecordingsFolder onError={onError} />}
      </PopoverContent>
    </Popover>
  );
}

/** how loud the recording's start and stop cues are; the icon mutes them and keeps the level */
function CueVolume() {
  const display = useSettings().display;
  // follows the drag, saved once it's let go
  const [dragging, setDragging] = useState<number | null>(null);
  const level = Math.round((dragging ?? display.volume) * 100);
  const silent = display.muted || level === 0;
  const save = (patch: Partial<typeof display>) =>
    updateSettings((shown) => ({ display: { ...shown.display, ...patch } }));
  return (
    <div className="settings-row">
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={display.muted ? "Unmute the cues" : "Mute the cues"}
        aria-pressed={display.muted}
        onClick={() => save({ muted: !display.muted })}
      >
        {silent ? <VolumeX /> : <Volume2 />}
      </Button>
      <Slider
        aria-label="Cue volume"
        min={0}
        max={100}
        value={[display.muted && dragging === null ? 0 : level]}
        onValueChange={([value]) => setDragging(value / 100)}
        onValueCommit={([value]) => {
          setDragging(null);
          save({ volume: value / 100, muted: false });
        }}
      />
    </div>
  );
}

/** the header sticks to the top of the page, and what sticks under it, like the recordings
 * table's header, reads where it ends from --app-header-bottom */
function useStuckBottom() {
  const ref = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const header = ref.current!;
    const set = () => {
      const top = parseFloat(getComputedStyle(header).top) || 0;
      document.documentElement.style.setProperty(
        "--app-header-bottom",
        `${top * 2 + header.offsetHeight}px`,
      );
    };
    const observer = new ResizeObserver(set);
    observer.observe(header);
    set();
    return () => observer.disconnect();
  }, []);
  return ref;
}
