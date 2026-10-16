import { test as base } from "@playwright/test";
import { previewSettings } from "../../ui/lib/preview-settings";
import type { AppSettings, SettingsPatch } from "../../ui/types";

type Invoke = (command: string, args?: Record<string, unknown>) => unknown;

/**
 * every page answers the settings commands the way native does, unless a test's own mock
 * answers them first
 */
export const test = base.extend({
  page: async ({ page }, use) => {
    await page.addInitScript((initial: AppSettings) => {
      let saved = initial;
      let internals: unknown;
      // a test reads what was saved, and may start from other saved settings
      Object.assign(window, {
        savedSettings: () => saved,
        seedSettings: (patch: Record<string, object>) => {
          const sections = saved as unknown as Record<string, object>;
          for (const [key, value] of Object.entries(patch))
            sections[key] = { ...sections[key], ...value };
        },
      });
      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: true,
        get: () => internals,
        set: (value: { invoke: Invoke }) => {
          internals = {
            ...value,
            invoke: async (command: string, args?: Record<string, unknown>) => {
              const reply = await value.invoke(command, args);
              if (reply !== undefined) return reply;
              if (command === "settings") return saved;
              if (command === "set_recordings_folder") {
                saved = {
                  ...saved,
                  storage: {
                    folder: args?.folder as string | null,
                    // the mock's running folder, which is the one the recordings would leave
                    move_from: args?.moveRecordings ? "C:/running" : null,
                  },
                };
                return saved;
              }
              if (command !== "update_settings") return reply;
              const patch = args?.patch as SettingsPatch;
              saved = {
                ...saved,
                ...patch,
                aim: { ...saved.aim, ...patch.aim },
              };
              return saved;
            },
          };
        },
      });
    }, previewSettings);
    await use(page);
  },
});
export { expect } from "@playwright/test";
export type SavedSettings = {
  savedSettings: () => AppSettings;
  seedSettings: (patch: Partial<Record<keyof AppSettings, object>>) => void;
};
