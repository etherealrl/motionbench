import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { native } from "./native";
import type { AppSettings, SettingsPatch } from "@/types";
import { previewSettings } from "./preview-settings";

// what native saved, the change it is saving and the changes waiting for it; the ui shows
// them laid over each other in that order
let saved = previewSettings;
let sending: SettingsPatch = {};
let queued: SettingsPatch = {};
let shown = saved;
let timer: ReturnType<typeof setTimeout> | undefined;
let writing: Promise<boolean> | null = null;
const listeners = new Set<() => void>();
const errors = new Set<(message: string) => void>();

function publish() {
  shown = { ...saved, ...sending, ...queued } as AppSettings;
  shown.aim = { ...saved.aim, ...sending.aim, ...queued.aim };
  shown.recording = { ...saved.recording, ...sending.recording, ...queued.recording };
  listeners.forEach((listener) => listener());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function useSettings() {
  return useSyncExternalStore(subscribe, () => shown);
}
export function onSettingsError(listener: (message: string) => void) {
  errors.add(listener);
  return () => {
    errors.delete(listener);
  };
}

/** the settings as native has them now, before the first render and after native saved some */
export async function loadSettings() {
  if (!native) return;
  saved = await invoke<AppSettings>("settings");
  publish();
}

/** whether every change made so far is saved; a refused one shows the saved value again */
export async function flushSettings(): Promise<boolean> {
  clearTimeout(timer);
  // a change already on its way counts as well: a refused one is not saved either
  let joined = true;
  while (writing) joined = (await writing) && joined;
  if (!Object.keys(queued).length) return joined;
  if (!native) {
    saved = shown;
    queued = {};
    publish();
    return true;
  }
  sending = queued;
  queued = {};
  writing = invoke<AppSettings>("update_settings", { patch: sending }).then(
    (value) => {
      saved = value;
      return true;
    },
    (error) => {
      errors.forEach((listener) => listener(`couldn't save the settings: ${String(error)}`));
      return false;
    },
  );
  const done = await writing;
  writing = null;
  sending = {};
  publish();
  return done && flushSettings();
}

/** a change to some sections, each whole. one made from what's shown at the time it's made, as a
 * function of it, keeps a change made a moment before that the caller hasn't rendered yet */
export function updateSettings(patch: SettingsPatch | ((shown: AppSettings) => SettingsPatch)) {
  queued = { ...queued, ...(typeof patch === "function" ? patch(shown) : patch) };
  publish();
  clearTimeout(timer);
  timer = setTimeout(() => void flushSettings(), 300);
}

/** where recordings go from the next start on, null for the default, and whether the ones already
 * made move there too. native checks it can write there first, so a refusal comes back as the error */
export async function setRecordingsFolder(folder: string | null, move: boolean) {
  if (native)
    saved = await invoke<AppSettings>("set_recordings_folder", { folder, moveRecordings: move });
  else saved = { ...saved, storage: { folder, move_from: null } };
  publish();
}
