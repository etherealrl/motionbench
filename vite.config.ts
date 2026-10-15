import { defineConfig, lazyPlugins } from "vite-plus";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  lint: {
    jsPlugins: ["@shadcn/lint", { name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: {
      "vite-plus/prefer-vite-plus-imports": "error",
      "shadcn/no-restyle": [
        "error",
        {
          // these app classes size or place the component in its spot (widths, alignment), which
          // the rem scale cannot say exactly under the 13px root font
          allow: [
            "layout",
            "display-settings",
            "status-diagnostics",
            "recording-popover",
            "population-trigger",
            "mouse-binding-picker",
          ],
        },
      ],
      "shadcn/no-raw-colors": "error",
      "shadcn/no-arbitrary-values": "error",
      "shadcn/no-inline-styles": "error",
      "shadcn/no-unknown-classes": "error",
      "shadcn/require-static-classes": "error",
    },
    overrides: [
      {
        // the shadcn primitives style their own internals, keep shadcn's exact pixel values and
        // radix size variables as shipped, and compose their own cva variants in cn()
        files: ["ui/components/ui/**"],
        rules: {
          "shadcn/no-restyle": "off",
          "shadcn/no-arbitrary-values": "off",
          "shadcn/require-static-classes": "off",
        },
      },
    ],
    options: { typeAware: true, typeCheck: true, denyWarnings: true },
  },
  fmt: {
    // made by `bun run fixture`, the shape test reads it back as json
    ignorePatterns: [
      "ui/fixture.ts",
      // broken on purpose, the tests read them as they are
      "tests/fixtures/**",
      // copied from where they're kept, so they stay the same as there
      ".agents/**",
      "src-tauri/runtime/**",
    ],
  },
  test: { include: ["tests/unit/**/*.test.ts"] },
  plugins: lazyPlugins(() => [react(), tailwindcss()]),
  resolve: { alias: { "@": fileURLToPath(new URL("./ui", import.meta.url)) } },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/.venv/**"] },
  },
});
