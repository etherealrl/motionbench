import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Save } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { SettingRow, SettingSection } from "@/components/setting-row";
import { native } from "@/lib/native";
import { updateSettings, useSettings } from "@/lib/settings";
import type { FovAxis, VisionConfig, VisionStatus } from "@/types";
import { NumberRow, type Update } from "./number-row";

type Game = {
  id: string;
  name: string;
  yaw: number;
  pitch: number;
  fov_axis: FovAxis;
  /** the FOV the game fixes, or its default when it lets you pick one */
  fov: number;
  /** whether the game lets you pick the FOV */
  free?: boolean;
};

/** the games whose numbers we know for sure */
const games: Game[] = [
  { id: "valorant", name: "Valorant", yaw: 0.07, pitch: 0.07, fov: 103, fov_axis: "horizontal" },
  {
    id: "cs2",
    name: "Counter-Strike 2",
    yaw: 0.022,
    pitch: 0.022,
    fov: 90,
    fov_axis: "horizontal_4x3",
  },
  {
    id: "apex",
    name: "Apex Legends",
    yaw: 0.022,
    pitch: 0.022,
    fov: 90,
    fov_axis: "horizontal_4x3",
    free: true,
  },
  {
    id: "overwatch",
    name: "Overwatch 2",
    yaw: 0.0066,
    pitch: 0.0066,
    fov: 103,
    fov_axis: "horizontal",
    free: true,
  },
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

const same = (a: number, b: number) => Math.abs(a - b) < 1e-9;
const fitsGame = (game: Game, config: VisionConfig) =>
  same(game.yaw, config.yaw) &&
  same(game.pitch, config.pitch) &&
  game.fov_axis === config.fov_axis &&
  (game.free || same(game.fov, config.fov));

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

/** how the game turns counts into the camera's angle. a game fills in its numbers, and the
 * sensitivity, and the FOV where the game lets you change it, are yours */
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
  const targets = useSettings().targets;
  // the game picked while it still fits, since CS2 and Apex share their numbers
  const game =
    games.find((g) => g.id === targets.game && fitsGame(g, config)) ??
    games.find((g) => fitsGame(g, config));
  const update: Update = (key, value) =>
    updateSettings((shown) => ({ vision: { ...shown.vision, [key]: value } }));
  return (
    <SettingSection title="Projection">
      <fieldset disabled={locked} className="setting-group">
        <Pick config={config} game={game} />
        <NumberRow
          setting={[
            "sensitivity",
            "Sensitivity",
            null,
            4,
            "exactly what's in your game's settings",
          ]}
          config={config}
          update={update}
        />
        {game?.free && (
          <NumberRow
            setting={["fov", "FOV", "°", 2]}
            config={config}
            update={update}
            description={`exactly what's in your game's settings, ${fovMeaning(config, screen)}`}
          />
        )}
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

/** the game to take the numbers from, and saving your sensitivity for it */
function Pick({ config, game }: { config: VisionConfig; game: Game | undefined }) {
  const targets = useSettings().targets;
  const saved = targets.sensitivities;
  const pick = (id: string) => {
    const chosen = games.find((g) => g.id === id)!;
    updateSettings((shown) => ({
      vision: {
        ...shown.vision,
        yaw: chosen.yaw,
        pitch: chosen.pitch,
        fov_axis: chosen.fov_axis,
        fov: chosen.fov,
        sensitivity: shown.targets.sensitivities[chosen.id] ?? shown.vision.sensitivity,
      },
      targets: { ...shown.targets, game: chosen.id },
    }));
  };
  const kept = game !== undefined && saved[game.id] === config.sensitivity;
  return (
    <SettingRow
      id="target-game"
      label="Game"
      description="fills in how the game turns your mouse into the camera. save keeps your sensitivity for that game, so picking it again brings it back"
    >
      <div className="input-with-action">
        <Dropdown
          id="target-game"
          aria-label="Game"
          value={game?.id ?? ""}
          placeholder="Pick a game"
          onValueChange={pick}
        >
          {games.map((g) => (
            <DropdownOption key={g.id} value={g.id}>
              {g.name}
            </DropdownOption>
          ))}
        </Dropdown>
        <Button
          variant="outline"
          size="icon"
          aria-label={game ? `Save the sensitivity for ${game.name}` : "Save the sensitivity"}
          title={
            kept ? "this sensitivity is saved for this game" : "save this sensitivity for the game"
          }
          disabled={!game || kept}
          onClick={() =>
            game &&
            updateSettings((shown) => ({
              targets: {
                ...shown.targets,
                sensitivities: {
                  ...shown.targets.sensitivities,
                  [game.id]: shown.vision.sensitivity,
                },
              },
            }))
          }
        >
          <Save />
        </Button>
      </div>
    </SettingRow>
  );
}
