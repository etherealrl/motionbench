import { runPlot, snapshot as fixture } from "../../ui/fixture";
import { expect, test } from "./settings-mock";
import type { Page } from "@playwright/test";

/** a live capture showing `snapshot` with `plot`, recording what the page asks of the native
 * side. only a recording's plot has a relay, the live one never does */
async function live(page: Page, decoded: boolean, plot = fixture.plot) {
  await page.addInitScript(
    ({ fixture, decoded, plot }) => {
      const state = structuredClone(fixture);
      state.plot = plot;
      state.live = true;
      // bursts are listed live only for a paused range
      (state as { range?: unknown }).range = {
        from_us: state.plot.from_us,
        to_us: state.plot.to_us,
      };
      if (!decoded) {
        state.plot.movement_available = false;
        state.plot.readouts.period_us = null;
        state.plot.readouts.jitter_us = null;
        state.plot.readouts.longest_moving_us = null;
        (state.plot.tabs as { movement: unknown }).movement = null;
        (state.plot.tabs as { relay: unknown }).relay = null;
      }
      const calls: [string, unknown][] = [];
      Object.assign(window, {
        isTauri: true,
        nativeCalls: calls,
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: () => 1,
          unregisterCallback: () => {},
          invoke: async (command: string, args: Record<string, unknown> = {}) => {
            if (command === "set_tab") calls.push([command, args.tab]);
            if (command === "set_range") calls.push([command, args.range]);
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
    },
    { fixture, decoded, plot },
  );
  await page.goto("/");
}

const calls = (page: Page) =>
  page.evaluate(() => (window as unknown as { nativeCalls: [string, unknown][] }).nativeCalls);

test("the Timing tab states what it counts, and a burst selects its range", async ({ page }) => {
  // a paused range lists its bursts, the recording's plot stands in for one
  await live(page, true, runPlot);
  const jitter = page.locator(".chart").filter({ hasText: "Jitter around the period" });
  await expect(jitter.locator(".chart-foot")).toContainText("intervals at 1×");
  await expect(jitter.locator(".chart-foot")).toContainText("not at 1×");
  await expect(page.getByText("Paused range")).toBeVisible();
  const burst = runPlot.tabs.timing!.bursts!.rows[0];
  await page.getByRole("row").filter({ hasText: "1,000 Hz" }).first().click();
  expect(await calls(page)).toContainEqual([
    "set_range",
    { from_us: burst.from_us, to_us: burst.to_us },
  ]);
  await page.getByRole("tab", { name: "Movement" }).click();
  // the held height lets go once the tab is there
  await expect(page.locator(".analysis-tabs")).toHaveAttribute("style", "--tabs-hold: 0px;");
  expect(await calls(page)).toEqual(
    expect.arrayContaining([
      ["set_tab", "timing"],
      ["set_tab", "movement"],
    ]),
  );
  await page.screenshot({
    path: "test-results/timing-tab.png",
    fullPage: true,
  });
});

test("without a decoder the moving views say why they are empty", async ({ page }) => {
  await live(page, false);
  await expect(page.getByText(/unavailable, there are no moving intervals/)).toBeVisible();
  const period = page.locator(".metric-grid > div").filter({ hasText: "Period estimate" });
  await expect(period.locator("strong")).toHaveText("-");
  // the timeline keeps every gap
  await expect(page.getByLabel(/Report interval timeline/)).toBeVisible();
  await page.getByRole("tab", { name: "Movement" }).click();
  await expect(page.getByText(/unavailable, there's no decoder/).first()).toBeVisible();
  // only a recording with the box's own input has a relay
  await expect(page.getByRole("tab", { name: "Relay" })).toHaveCount(0);
});

test("the Movement tab bins counts, sets intervals by speed and folds the path away", async ({
  page,
}) => {
  await live(page, true);
  await page.getByRole("tab", { name: "Movement" }).click();
  const counts = page.locator(".chart").filter({ hasText: "Counts per report" });
  await expect(counts.locator(".chart-foot")).toContainText("next report alike");
  const speed = page.locator(".chart").filter({ hasText: "Interval by speed" });
  const rows = fixture.plot.tabs.movement!.speed.filter((s) => s.intervals);
  await expect(speed.getByRole("row")).toHaveCount(rows.length + 1);
  // the toggle sits as far below the tab row as the first card sits below it
  const tabRow = (await page.locator(".analysis-tabs-head").boundingBox())!;
  const option = (await page.locator(".chart-option").boundingBox())!;
  const card = (await page.locator(".analysis-tabs .chart").first().boundingBox())!;
  expect(
    Math.abs(option.y - (tabRow.y + tabRow.height) - (card.y - (option.y + option.height))),
  ).toBeLessThan(1);
  await page.getByLabel("Signed dx and dy").click();
  await expect(page.getByLabel("Signed dx and dy")).toBeChecked();
  await expect(page.getByLabel("Count-space movement path")).toBeHidden();
  await page.getByText("Count-space path").click();
  await expect(page.getByLabel("Count-space movement path")).toBeVisible();
});

test("the Relay tab sets the box's counts against the wire's", async ({ page }) => {
  await live(page, true, runPlot);
  await page.getByRole("tab", { name: "Relay" }).click();
  const relay = runPlot.tabs.relay!;
  const conserved = page.locator(".chart").filter({ hasText: "Counts conserved" });
  await expect(conserved.locator(".chart-foot")).toContainText(
    `box dx ${relay.box_counts[0].toLocaleString("en-US")}`,
  );
  const matching = page.locator(".chart").filter({ hasText: "Matching" });
  await expect(matching.getByText("Paired")).toBeVisible();
  await expect(matching).toContainText(String(relay.matched));
  await expect(page.locator(".chart").filter({ hasText: "Offset variation" })).toContainText(
    "absolute latency isn't measured",
  );
});

test("a build without the developer tools has no Experiment view", async ({ page }) => {
  await page.addInitScript((fixture) => {
    const state = structuredClone(fixture);
    (state as { developer_tools?: boolean }).developer_tools = false;
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
  await expect(page.getByRole("tab", { name: "Capture" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Experiment" })).toHaveCount(0);
});

test("switching tabs live keeps the scroll position while the new tab arrives", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1400, height: 700 });
  await page.addInitScript((fixture) => {
    // like native, a live frame carries only the tab shown, and the new one a frame later
    const all = structuredClone(fixture.plot.tabs);
    const state = structuredClone(fixture);
    state.live = true;
    let shown = "timing";
    let channel: { onmessage: (value: unknown) => void } | null = null;
    const frame = (tab: string | null) => {
      state.plot.tabs = { timing: null, movement: null, relay: null };
      if (tab)
        (state.plot.tabs as Record<string, unknown>)[tab] = (all as Record<string, unknown>)[tab];
      state.seq++;
      channel?.onmessage(structuredClone(state));
    };
    frame("timing");
    Object.assign(window, {
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
          if (command === "subscribe") channel = args.channel as typeof channel;
          if (command === "set_tab") {
            shown = String(args.tab);
            frame(null);
            setTimeout(() => frame(shown), 300);
          }
          if (command === "plugin:event|listen") return 1;
        },
      },
    });
  }, fixture);
  await page.goto("/");
  await page.getByRole("tab", { name: "Movement" }).click();
  await expect(page.locator(".analysis-tabs .chart")).toHaveCount(4);
  // the tabs at the top of the window: a page shrunk while the next tab is on its way would pull
  // the view up
  await page
    .getByRole("tab", { name: "Timing" })
    .evaluate((tab) => tab.scrollIntoView({ block: "start" }));
  const before = await page.evaluate(() => window.scrollY);
  expect(before).toBeGreaterThan(0);
  await page.getByRole("tab", { name: "Timing" }).click();
  await page.waitForTimeout(150);
  expect(await page.evaluate(() => window.scrollY)).toBe(before);
  // the held height lets go once the tab is there
  await expect(page.locator(".analysis-tabs")).toHaveAttribute("style", "--tabs-hold: 0px;");
});
