import { beforeEach, expect, test, vi } from "vite-plus/test";
import type { AppSettings, SettingsPatch } from "@/types";

// native's side of the settings: each reply is held until the test lets it go
const replies: {
  patch: SettingsPatch;
  resolve: (value: AppSettings) => void;
  reject: (error: string) => void;
}[] = [];
let native: AppSettings;
vi.mock("@/lib/native", () => ({ native: true }));
// what a component sees, read the way react reads it
vi.mock("react", () => ({
  useSyncExternalStore: (_: unknown, get: () => AppSettings) => get(),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: { patch: SettingsPatch }) => {
    if (command === "settings") return Promise.resolve(native);
    return new Promise((resolve, reject) => replies.push({ patch: args.patch, resolve, reject }));
  },
}));

import { previewSettings } from "@/lib/preview-settings";
const settings = await import("@/lib/settings");
const shown = () => settings.useSettings();
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(async () => {
  native = structuredClone(previewSettings);
  await settings.loadSettings();
});

test("a refused change shows the saved value again and says why", async () => {
  const errors: string[] = [];
  const off = settings.onSettingsError((message) => errors.push(message));
  settings.updateSettings({ aim: { speed: 50, deadzone_counts: 12 } });
  expect(shown().aim.speed).toBe(50);
  const flushed = settings.flushSettings();
  await tick();
  replies.shift()!.reject("aim speed is between 100 and 20,000 counts/s");
  expect(await flushed).toBe(false);
  expect(shown().aim.speed).toBe(2000);
  expect(errors[0]).toContain("aim speed");
  off();
});

test("a change made while another is saved stays shown after the reply", async () => {
  settings.updateSettings({ aim: { speed: 3000, deadzone_counts: 12 } });
  const flushed = settings.flushSettings();
  await tick();
  settings.updateSettings({ aim: { speed: 4000, deadzone_counts: 12 } });
  const first = replies.shift()!;
  first.resolve({ ...native, aim: { ...native.aim, ...first.patch.aim! } });
  await tick();
  expect(shown().aim.speed).toBe(4000);
  // the second change goes out once the first is answered
  await tick();
  const second = replies.shift()!;
  expect(second.patch.aim?.speed).toBe(4000);
  second.resolve({ ...native, aim: { ...native.aim, ...second.patch.aim! } });
  expect(await flushed).toBe(true);
  expect(shown().aim).toEqual({ speed: 4000, deadzone_counts: 12, button: 8 });
});

test("a change in another section stays shown while one is saved", async () => {
  settings.updateSettings({ aim: { speed: 3000, deadzone_counts: 12 } });
  const flushed = settings.flushSettings();
  await tick();
  settings.updateSettings({
    display: {
      zoom: 1.5,
      sections: { targets: true, box: false, box_settings: false },
      volume: 0.6,
      muted: false,
      theme: "system",
    },
  });
  expect(shown().aim.speed).toBe(3000);
  expect(shown().display.zoom).toBe(1.5);
  const first = replies.shift()!;
  first.resolve({ ...native, aim: { ...native.aim, ...first.patch.aim! } });
  await tick();
  replies.shift()!.resolve({
    ...native,
    aim: { ...native.aim, speed: 3000 },
    display: {
      zoom: 1.5,
      sections: { targets: true, box: false, box_settings: false },
      volume: 0.6,
      muted: false,
      theme: "system",
    },
  });
  expect(await flushed).toBe(true);
});

test("a save that joins one already on its way reports its refusal", async () => {
  vi.useFakeTimers();
  settings.updateSettings({ aim: { speed: 50, deadzone_counts: 12 } });
  // the debounce sends it, then something that needs it saved asks
  vi.advanceTimersByTime(300);
  vi.useRealTimers();
  const joined = settings.flushSettings();
  await tick();
  replies.shift()!.reject("aim speed is between 100 and 20,000 counts/s");
  expect(await joined).toBe(false);
});
