import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests/ui",
  use: {
    baseURL: "http://localhost:1420",
    viewport: { width: 1600, height: 1200 },
    // the app follows the system theme, and the color checks are written for the dark one
    colorScheme: "dark",
  },
  webServer: {
    command: "bun run dev",
    url: "http://localhost:1420",
    reuseExistingServer: true,
  },
});
