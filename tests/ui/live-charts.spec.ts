import { runPlot, snapshot as fixture } from "../../ui/fixture";
import { expect, test } from "./settings-mock";

test("active cadence shows an unavailable value during idle and explains its scope", async ({
  page,
}) => {
  await page.addInitScript((fixture) => {
    const state = structuredClone(fixture);
    state.live = true;
    state.plot.readouts.rate_hz = 0;
    let channel: { onmessage: (value: unknown) => void } | null = null;
    Object.assign(window, {
      isTauri: true,
      publishActiveRate: () => {
        state.seq++;
        state.plot.readouts.rate_hz = 1000;
        channel?.onmessage(structuredClone(state));
      },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          if (command === "runs")
            return {
              revision: state.library,
              runs: structuredClone(state.runs),
            };
          if (command === "initialize") return { roots: [], snapshot: state };
          if (command === "subscribe") channel = args.channel as typeof channel;
          if (command === "plugin:event|listen") return 1;
        },
      },
    });
  }, fixture);
  await page.goto("/");
  const metric = page
    .locator(".metric-grid > div")
    .filter({ hasText: "Active report rate" })
    .first();
  await expect(metric.locator("strong")).toHaveText("-");
  await metric.getByRole("button", { name: "About active report rate" }).focus();
  await expect(page.getByRole("tooltip")).toHaveText(
    /latest second of movement.*pauses longer than 50 ms are left out/,
  );
  await page.keyboard.press("Escape");
  await page.evaluate(() =>
    (window as unknown as { publishActiveRate: () => void }).publishActiveRate(),
  );
  await expect(metric.locator("strong")).toHaveText("1,000Hz");
  await expect(page.getByText(/reports in the selected range/)).toHaveCount(0);
});

test("live chart redraws follow animation frames with one snapshot in flight", async ({ page }) => {
  // the scheduling is measured on a small plot, so how fast the browser draws doesn't decide it
  await page.addInitScript(
    ({ fixture, plot }) => {
      const state = structuredClone(fixture);
      state.plot = structuredClone(plot);
      state.live = true;
      let channel: { onmessage: (value: unknown) => void } | null = null;
      let outstanding = false;
      const metrics = {
        frames: 0,
        acknowledgements: 0,
        draws: {} as Record<string, number>,
        violations: 0,
      };
      const prototype = CanvasRenderingContext2D.prototype;
      const originalClear = Object.getOwnPropertyDescriptor(prototype, "clearRect")!.value as (
        ...args: Parameters<CanvasRenderingContext2D["clearRect"]>
      ) => void;
      prototype.clearRect = function (...args) {
        const label = this.canvas.getAttribute("aria-label") ?? "";
        metrics.draws[label] = (metrics.draws[label] ?? 0) + 1;
        originalClear.apply(this, args);
      };
      const tick = () => {
        metrics.frames++;
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
      const send = () => {
        if (outstanding) metrics.violations++;
        outstanding = true;
        state.seq++;
        state.plot.reports++;
        const path = state.plot.tabs.movement!.path;
        path[path.length - 1] = [state.seq, state.seq % 17];
        channel?.onmessage(structuredClone(state));
      };
      Object.assign(window, {
        liveChartMetrics: metrics,
        isTauri: true,
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: () => 1,
          unregisterCallback: () => {},
          invoke: async (command: string, args: Record<string, unknown> = {}) => {
            if (command === "runs")
              return {
                revision: state.library,
                runs: structuredClone(state.runs),
              };
            if (command === "initialize") return { roots: [], snapshot: state };
            if (command === "subscribe") {
              channel = args.channel as typeof channel;
              send();
            }
            if (command === "acknowledge") {
              outstanding = false;
              metrics.acknowledgements++;
              setTimeout(send, 2);
            }
            if (command === "plugin:event|listen") return 1;
          },
        },
      });
    },
    { fixture, plot: runPlot },
  );
  await page.goto("/");
  await page.getByRole("tab", { name: "Movement" }).click();
  await expect(page.getByLabel("Movement counts over time")).toBeVisible();
  await page.evaluate(() => {
    const metrics = (
      window as unknown as {
        liveChartMetrics: {
          frames: number;
          acknowledgements: number;
          draws: Record<string, number>;
        };
      }
    ).liveChartMetrics;
    metrics.frames = metrics.acknowledgements = 0;
    metrics.draws = {};
  });
  await page.waitForTimeout(1000);
  const metrics = await page.evaluate(
    () =>
      (
        window as unknown as {
          liveChartMetrics: {
            frames: number;
            acknowledgements: number;
            draws: Record<string, number>;
            violations: number;
          };
        }
      ).liveChartMetrics,
  );
  expect(metrics.frames).toBeGreaterThan(20);
  expect(metrics.acknowledgements).toBeGreaterThan(metrics.frames * 0.6);
  expect(metrics.acknowledgements).toBeLessThanOrEqual(metrics.frames + 1);
  // every chart keeps up, and none draws more often than a frame. how close it keeps up depends
  // on how busy the machine running the tests is, so the floor is half the frames
  for (const label of [
    "Report interval timeline. Drag to select a range.",
    "Movement counts over time",
    "Count-space movement path",
  ]) {
    expect(metrics.draws[label]).toBeGreaterThan(metrics.frames * 0.5);
    expect(metrics.draws[label]).toBeLessThanOrEqual(metrics.frames + 1);
  }
  expect(metrics.violations).toBe(0);
});

test("pause holds the live view on what it shows and resume follows the clock again", async ({
  page,
}) => {
  await page.addInitScript((fixture) => {
    const state = structuredClone(fixture);
    state.live = true;
    let channel: { onmessage: (value: unknown) => void } | null = null;
    const calls: unknown[] = [];
    Object.assign(window, {
      isTauri: true,
      rangeCalls: calls,
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          if (command === "set_range") {
            calls.push(args.range);
            (state as { range?: unknown }).range = args.range;
            state.seq++;
            channel?.onmessage(structuredClone(state));
          }
          if (command === "runs")
            return {
              revision: state.library,
              runs: structuredClone(state.runs),
            };
          if (command === "initialize") return { roots: [], snapshot: state };
          if (command === "subscribe") channel = args.channel as typeof channel;
          if (command === "plugin:event|listen") return 1;
        },
      },
    });
  }, fixture);
  await page.goto("/");
  const toolbar = page.locator(".range-toolbar").first();
  await expect(toolbar).toContainText("Live");
  await page.getByRole("button", { name: "Pause" }).click();
  await expect(toolbar).toContainText("Paused");
  await page.getByRole("button", { name: "Resume live" }).click();
  await expect(toolbar).toContainText("Live");
  expect(
    await page.evaluate(() => (window as unknown as { rangeCalls: unknown[] }).rangeCalls),
  ).toEqual([{ from_us: fixture.plot.from_us, to_us: fixture.plot.to_us }, null]);
  await toolbar.screenshot({ path: "test-results/paused.png" });
});

test("a paused range with nothing left in it can still resume", async ({ page }) => {
  await page.addInitScript((fixture) => {
    const state = structuredClone(fixture);
    state.live = true;
    state.plot.reports = 0;
    (state as { range?: unknown }).range = {
      from_us: state.plot.from_us,
      to_us: state.plot.to_us,
    };
    Object.assign(window, {
      isTauri: true,
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (command: string) => {
          if (command === "runs")
            return {
              revision: state.library,
              runs: structuredClone(state.runs),
            };
          if (command === "initialize") return { roots: [], snapshot: state };
          if (command === "plugin:event|listen") return 1;
        },
      },
    });
  }, fixture);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Resume live" })).toBeVisible();
});

test("a tooltip sits centred over its button at any ui scale", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Settings").click();
  await page.getByLabel("UI scale").click();
  await page
    .getByRole("option")
    .filter({ hasText: /^150%$/ })
    .click();
  await page.keyboard.press("Escape");
  const button = page.getByRole("button", { name: "About period estimate" });
  await button.hover();
  const tip = page.locator("[data-slot=tooltip-content]");
  await expect(tip).toBeVisible();
  const [b, t] = [(await button.boundingBox())!, (await tip.boundingBox())!];
  expect(Math.abs(b.x + b.width / 2 - (t.x + t.width / 2))).toBeLessThan(2);
  expect(t.y + t.height).toBeLessThanOrEqual(b.y);
});
