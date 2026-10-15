import { isTauri } from "@tauri-apps/api/core";

// the browser preview runs without the native side: a fixed snapshot, no capture, no library
export const native = isTauri();
