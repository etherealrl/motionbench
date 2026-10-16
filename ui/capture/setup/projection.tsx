import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Save, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Input } from "@/components/ui/input";
import { SelectGroup, SelectLabel } from "@/components/ui/select";
import { SettingRow, SettingSection } from "@/components/setting-row";
import { native } from "@/lib/native";
import { updateSettings, useSettings } from "@/lib/settings";
import type { FovAxis, Projection, VisionConfig, VisionStatus } from "@/types";
import { NumberRow, type Update } from "./number-row";

type Game = {
  id: string;
  name: string;
  yaw: number;
  pitch: number;
  /** the FOV when the game fixes it */
  fov?: number;
  fov_axis: FovAxis;
};

/** the games whose numbers we know for sure. the sensitivity is yours, so a game never sets it */
const games: Game[] = [
  { id: "valorant", name: "Valorant", yaw: 0.07, pitch: 0.07, fov: 103, fov_axis: "horizontal" },
  {
    id: "aimlab",
    name: "Aim Lab (Valorant scale)",
    yaw: 0.07,
    pitch: 0.07,
    fov_axis: "horizontal",
  },
  {
    id: "cs2",
    name: "Counter-Strike 2",
    yaw: 0.022,
    pitch: 0.022,
    fov: 90,
    fov_axis: "horizontal_4x3",
  },
  { id: "apex", name: "Apex Legends", yaw: 0.022, pitch: 0.022, fov_axis: "horizontal_4x3" },
  { id: "overwatch", name: "Overwatch 2", yaw: 0.0066, pitch: 0.0066, fov_axis: "horizontal" },
];

const axes: [FovAxis, string][] = [
  ["horizontal", "Horizontal"],
  ["vertical", "Vertical"],
  ["horizontal_4x3", "4:3"],
];

type Screen = [number, number];
const radians = (degrees: number) => (degrees * Math.PI) / 180;

// how many pixels the FOV spans along its axis, 4:3 being a 4:3 screen as tall as this one
function across(axis: FovAxis, [width, height]: Screen) {
  if (axis === "horizontal") return width;
  if (axis === "vertical") return height;
  return (height * 4) / 3;
}

/** the same FOV said along another axis */
function convertFov(fov: number, from: FovAxis, to: FovAxis, screen: Screen) {
  const half = (Math.tan(radians(fov) / 2) * across(to, screen)) / across(from, screen);
  return (2 * Math.atan(half) * 180) / Math.PI;
}

/** the same FOV said along `axis`, unrounded, so switching back gives the same number and a
 * preset still fits */
const sameView = (config: VisionConfig, axis: FovAxis, screen: Screen) => ({
  fov_axis: axis,
  fov: convertFov(config.fov, config.fov_axis, axis, screen),
});

const same = (a: number, b: number) => Math.abs(a - b) < 1e-9;
const sameProjection = (a: Projection, b: VisionConfig) =>
  same(a.sensitivity, b.sensitivity) &&
  same(a.fov, b.fov) &&
  a.fov_axis === b.fov_axis &&
  same(a.yaw, b.yaw) &&
  same(a.pitch, b.pitch);
const fitsGame = (game: Game, config: VisionConfig) =>
  same(game.yaw, config.yaw) &&
  same(game.pitch, config.pitch) &&
  game.fov_axis === config.fov_axis &&
  (game.fov === undefined || same(game.fov, config.fov));

/** the size of the display target capture copies: what's running when it runs, else what native
 * reads from the display picked in the settings */
function useScreen(config: VisionConfig, status: VisionStatus | undefined): Screen | null {
  // the size is kept with the display it was read from, so a newly picked one has none until read
  const picked = `${config.adapter}:${config.output}`;
  const [display, setDisplay] = useState<{ of: string; size: Screen | null } | null>(null);
  useEffect(() => {
    if (!native) return;
    let active = true;
    invoke<Screen | null>("display_size", { adapter: config.adapter, output: config.output })
      .then((size) => active && setDisplay({ of: picked, size }))
      .catch(() => active && setDisplay({ of: picked, size: null }));
    return () => {
      active = false;
    };
  }, [config.adapter, config.output]);
  if (status?.running && status.viewport?.[0]) return status.viewport;
  return display?.of === picked ? display.size : null;
}

/** how the game turns counts into the camera's angle: its sensitivity, FOV, yaw and pitch, filled
 * in from a game or a projection saved by name */
export function ProjectionSettings({
  config,
  status,
  locked,
}: {
  config: VisionConfig;
  status?: VisionStatus;
  locked: boolean;
}) {
  const screen = useScreen(config, status);
  const set = (patch: Partial<VisionConfig>) => updateSettings({ vision: { ...config, ...patch } });
  const update: Update = (key, value) => set({ [key]: value });
  return (
    <SettingSection title="Projection">
      <fieldset disabled={locked} className="setting-group">
        <Preset config={config} screen={screen} set={set} />
        <NumberRow
          setting={["sensitivity", "Sensitivity", null, 4, "the one in the game's settings"]}
          config={config}
          update={update}
        />
        <NumberRow
          setting={["fov", "FOV", "°", 2]}
          config={config}
          update={update}
          description={fovMeaning(config, screen)}
          after={
            <Dropdown
              aria-label="FOV measured"
              value={config.fov_axis}
              // without the display's size the same number would just mean another view
              disabled={!screen}
              onValueChange={(axis) => screen && set(sameView(config, axis as FovAxis, screen))}
            >
              {axes.map(([axis, label]) => (
                <DropdownOption key={axis} value={axis}>
                  {label}
                </DropdownOption>
              ))}
            </Dropdown>
          }
        />
        <NumberRow
          setting={[
            "yaw",
            "Yaw",
            "° per count",
            4,
            "how far one count turns the camera sideways at sensitivity 1",
          ]}
          config={config}
          update={update}
        />
        <NumberRow
          setting={["pitch", "Pitch", "° per count", 4, "the same, up and down"]}
          config={config}
          update={update}
        />
      </fieldset>
    </SettingSection>
  );
}

// the FOV said the other ways too, on this display, so a number from any game's settings can be
// checked
function fovMeaning(config: VisionConfig, screen: Screen | null) {
  const along: Record<FovAxis, string> = {
    horizontal: "across the screen",
    vertical: "from top to bottom",
    horizontal_4x3: "across a 4:3 screen as tall as yours, the way CS2 and Apex measure it",
  };
  if (!screen) return `measured ${along[config.fov_axis]}`;
  const shown = (axis: FovAxis) =>
    `${Number(convertFov(config.fov, config.fov_axis, axis, screen).toFixed(2))}°`;
  const others = (["horizontal", "vertical"] as const).filter((axis) => axis !== config.fov_axis);
  const said =
    others.length === 2
      ? `${shown("horizontal")} across and ${shown("vertical")} top to bottom`
      : others[0] === "horizontal"
        ? `${shown("horizontal")} across`
        : `${shown("vertical")} top to bottom`;
  return `measured ${along[config.fov_axis]}, that's ${said} on this ${screen[0]} × ${screen[1]} display`;
}

/** a game or a saved projection to fill the settings in from, and saving the current one */
function Preset({
  config,
  screen,
  set,
}: {
  config: VisionConfig;
  screen: Screen | null;
  set: (patch: Partial<VisionConfig>) => void;
}) {
  const targets = useSettings().targets;
  const saved = targets.projections;
  const [picked, setPicked] = useState<string | null>(null);
  const [naming, setNaming] = useState<string | null>(null);
  const mine = saved.find((projection) => sameProjection(projection, config));
  // the game picked last while it still fits, since two games can share their numbers
  const game =
    games.find((g) => g.id === picked && fitsGame(g, config)) ??
    games.find((g) => fitsGame(g, config));
  const value = mine ? `saved:${mine.name}` : game ? `game:${game.id}` : "";
  const pick = (choice: string) => {
    const [kind, id] = [
      choice.slice(0, choice.indexOf(":")),
      choice.slice(choice.indexOf(":") + 1),
    ];
    if (kind === "game") {
      const chosen = games.find((g) => g.id === id)!;
      setPicked(chosen.id);
      // a FOV the game lets you pick stays the view it was, said the way the game measures it
      const fov =
        chosen.fov !== undefined
          ? { fov_axis: chosen.fov_axis, fov: chosen.fov }
          : screen
            ? sameView(config, chosen.fov_axis, screen)
            : {};
      set({ yaw: chosen.yaw, pitch: chosen.pitch, ...fov });
    } else {
      const { name: _, ...projection } = saved.find((p) => p.name === id)!;
      set(projection);
    }
  };
  const store = (projections: Projection[]) =>
    updateSettings({ targets: { ...targets, projections } });
  return (
    <SettingRow
      id="target-preset"
      label="Preset"
      description="fills in a game's yaw and pitch, and its FOV when the game doesn't let you change it. otherwise your FOV stays the same view, said the way that game measures it. the ones you save show up here too"
    >
      <div className="input-with-action">
        <Dropdown
          id="target-preset"
          aria-label="Preset"
          value={value}
          placeholder="Custom"
          onValueChange={pick}
        >
          <SelectGroup>
            <SelectLabel>Games</SelectLabel>
            {games.map((g) => (
              <DropdownOption key={g.id} value={`game:${g.id}`}>
                {g.name}
              </DropdownOption>
            ))}
          </SelectGroup>
          {saved.length > 0 && (
            <SelectGroup>
              <SelectLabel>Saved</SelectLabel>
              {saved.map((p) => (
                <DropdownOption key={p.name} value={`saved:${p.name}`}>
                  {p.name}
                </DropdownOption>
              ))}
            </SelectGroup>
          )}
        </Dropdown>
        {mine ? (
          <Button
            variant="outline"
            size="icon"
            aria-label={`Delete ${mine.name}`}
            title={`delete ${mine.name}`}
            onClick={() => store(saved.filter((p) => p !== mine))}
          >
            <Trash2 />
          </Button>
        ) : (
          <Button
            variant="outline"
            size="icon"
            aria-label="Save this projection"
            title="save this projection under a name"
            onClick={() => setNaming(game?.name ?? "")}
          >
            <Save />
          </Button>
        )}
      </div>
      <SaveProjection
        name={naming}
        taken={saved.map((p) => p.name)}
        onClose={() => setNaming(null)}
        onSave={(name) => {
          const { fov, fov_axis, sensitivity, yaw, pitch } = config;
          store([
            ...saved.filter((p) => p.name !== name),
            { name, sensitivity, fov, fov_axis, yaw, pitch },
          ]);
          setNaming(null);
        }}
      />
    </SettingRow>
  );
}

function SaveProjection({
  name,
  taken,
  onClose,
  onSave,
}: {
  name: string | null;
  taken: string[];
  onClose: () => void;
  onSave: (name: string) => void;
}) {
  const [typed, setTyped] = useState("");
  useEffect(() => {
    if (name !== null) setTyped(name);
  }, [name]);
  const trimmed = typed.trim();
  return (
    <Dialog open={name !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Save projection</DialogTitle>
          <DialogDescription>
            the sensitivity, FOV, yaw and pitch, saved under a name so switching games is one pick
          </DialogDescription>
        </DialogHeader>
        <form
          className="setting-stack"
          onSubmit={(e) => {
            e.preventDefault();
            if (trimmed) onSave(trimmed);
          }}
        >
          <Input
            aria-label="Name"
            autoFocus
            maxLength={60}
            placeholder="Valorant 0.35"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
          {taken.includes(trimmed) && (
            <p className="field-description">it replaces the one already called {trimmed}</p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={!trimmed}>
              Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
