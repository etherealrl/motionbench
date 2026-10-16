import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  // one app at a time, each one owns the USB capture and the box while it runs
  workers: 1,
  timeout: 60_000,
});
