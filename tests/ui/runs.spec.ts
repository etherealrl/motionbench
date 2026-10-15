import { runPlot, snapshot as fixture } from "../../ui/fixture";
import { expect, test } from "./settings-mock";
import type { Page } from "@playwright/test";

async function open(page: Page, libraryWarnings: string[] = []) {
  await page.addInitScript(
    ({ fixture, runPlot, libraryWarnings }) => {
      const snapshot = structuredClone(fixture);
      snapshot.runs = [
        {
          ...snapshot.runs[0],
          id: "first",
          name: "first.pcap",
          // a recording that saved no clock data
          metadata: {
            ...snapshot.runs[0].metadata,
            timing: undefined,
            observers: undefined,
            paired: undefined,
          },
        },
        {
          ...snapshot.runs[0],
          id: "second",
          name: "second.pcap",
          // target capture was on, and no click made a trial
          panels: ["texture19"],
          missing: {
            trajectory14: "target capture was on but none of the 4 clicks made a trial",
          },
          status: {
            kind: "degraded",
            message: "the receipt clock drifted",
            reason: null,
          },
          metadata: {
            ...snapshot.runs[0].metadata,
            timing: {
              anchor_summary: {
                samples: 12,
                max_bracket_us: 3.2,
                max_mapping_change_us: 0.5,
                discontinuities: 0,
              },
            },
            observers: { complete: true, raw_input: { max_batch: 4 } },
            participants: [{ id: "vision" }],
          },
        },
      ];
      const calls: { command: string; args: unknown }[] = [];
      let channel: { onmessage: (value: unknown) => void } | null = null;
      // like native, a new crop reaches the page in the next snapshot
      const crop = (id: string, value: unknown) => {
        const run = snapshot.runs.find((r) => r.id === id) as { crop: unknown };
        run.crop = value;
        snapshot.library++;
        snapshot.seq++;
        channel?.onmessage(structuredClone(snapshot));
      };
      Object.assign(window, {
        isTauri: true,
        runCalls: calls,
        cropRun: crop,
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: () => 1,
          unregisterCallback: () => {},
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, args });
            if (command === "runs")
              return {
                revision: snapshot.library,
                runs: structuredClone(snapshot.runs),
              };
            if (command === "initialize")
              return {
                roots: [],
                snapshot,
                library_warnings: libraryWarnings,
              };
            if (command === "run_plot") return runPlot;
            // two targets on screen from 0.1 to 0.3 s
            if (command === "run_replay")
              return (window as unknown as { replayData: unknown }).replayData;
            if (command === "run_crowded")
              return [
                [runPlot.from_us + 100_000, runPlot.from_us + 300_000],
                [runPlot.from_us + 500_000, runPlot.from_us + 500_000],
              ];
            if (command === "subscribe") channel = (args as { channel: typeof channel }).channel;
            if (command === "set_crop") {
              const { id, crop: value } = args as { id: string; crop: unknown };
              crop(id, value);
              return null;
            }
            if (command === "plugin:event|listen") return 1;
          },
        },
      });
    },
    { fixture, runPlot, libraryWarnings },
  );
  await page.goto("/");
  await page.getByRole("tab", { name: /Runs/ }).click();
}

test("a library that couldn't be read opens its folder from the notice", async ({ page }) => {
  await open(page, ["the recording library couldn't be read"]);
  const notice = page.getByText("the recording library couldn't be read");
  await expect(notice).toBeVisible();
  await page.getByRole("button", { name: "Open folder" }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { runCalls: { command: string }[] }).runCalls.some(
          (call) => call.command === "open_folder",
        ),
      ),
    )
    .toBe(true);
});

const renames = (page: Page) =>
  page.evaluate(() =>
    (window as unknown as { runCalls: { command: string; args: unknown }[] }).runCalls
      .filter((call) => call.command === "rename_run")
      .map((call) => call.args),
  );

async function startRename(page: Page, name: string) {
  // the name itself opens its field
  const button = page.getByRole("button", { name: `Rename ${name}` });
  await button.hover();
  const box = (await button.boundingBox())!;
  await page.screenshot({
    path: "test-results/run-name-hover.png",
    clip: { x: box.x - 20, y: box.y - 20, width: 400, height: box.height + 40 },
  });
  await button.click();
  const input = page.getByRole("textbox", { name: `Name for ${name}` });
  await expect(input).toBeFocused();
  return input;
}

test("renaming a recording saves the trimmed name on Enter and nothing on Escape", async ({
  page,
}) => {
  await open(page);
  let input = await startRename(page, "first.pcap");
  // the whole name is selected, so typing replaces it
  await page.keyboard.type("  baseline, slow sweep  ");
  await page.keyboard.press("Enter");
  await expect(input).toHaveCount(0);
  expect(await renames(page)).toEqual([{ id: "first", name: "baseline, slow sweep" }]);
  input = await startRename(page, "second.pcap");
  await page.keyboard.type("discarded");
  await page.keyboard.press("Escape");
  await expect(input).toHaveCount(0);
  // a blank name is no name, it keeps the old one
  input = await startRename(page, "second.pcap");
  await input.fill("   ");
  await page.keyboard.press("Enter");
  expect(await renames(page)).toHaveLength(1);
  await expect(page.locator(".run-table").getByText("second.pcap", { exact: true })).toBeVisible();
});

test("recording clocks show what the recording saved and hand focus back to its menu", async ({
  page,
}) => {
  await open(page);
  const menu = page.getByRole("button", {
    name: "More actions for second.pcap",
  });
  await menu.click();
  await page.getByRole("menuitem", { name: "Recording clocks" }).click();
  const clocks = page.getByRole("dialog", { name: "Recording clocks" });
  await expect(clocks.getByText("second.pcap", { exact: true })).toBeVisible();
  await expect(clocks.getByRole("status")).toHaveText("the receipt clock drifted");
  const value = (term: string) =>
    clocks.locator("dt", { hasText: term }).locator("xpath=following-sibling::dd[1]");
  await expect(value("Clock anchors")).toHaveText("12");
  await expect(value("Largest anchor uncertainty")).toHaveText("3.2 µs");
  await expect(value("Largest UTC/QPC change")).toHaveText("0.5 µs");
  await expect(value("Clock discontinuities")).toHaveText("0");
  await expect(value("Raw Input largest batch")).toHaveText("4");
  await expect(value("Observer evidence")).toHaveText("Finalized");
  await page.keyboard.press("Escape");
  await expect(clocks).toHaveCount(0);
  await expect(menu).toBeFocused();
  // a recording without clock data says so instead of listing dashes
  await page.getByRole("button", { name: "More actions for first.pcap" }).click();
  await page.getByRole("menuitem", { name: "Recording clocks" }).click();
  await expect(
    clocks.getByText("this recording didn't save any clock data", {
      exact: true,
    }),
  ).toBeVisible();
});

const calls = (page: Page, command: string) =>
  page.evaluate(
    (command) =>
      (window as unknown as { runCalls: { command: string; args: unknown }[] }).runCalls
        .filter((call) => call.command === command)
        .map((call) => call.args as Record<string, unknown>),
    command,
  );

test("a crop is chosen over the whole recording and saved to the library", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await open(page);
  const menu = page.getByRole("button", {
    name: "More actions for first.pcap",
  });
  await menu.click();
  await page.getByRole("menuitem", { name: "Crop" }).click();
  const dialog = page.getByRole("dialog", { name: "Crop" });
  await expect(dialog.getByLabel(/Report interval timeline, whole recording/)).toBeVisible();
  expect((await calls(page, "run_plot")).at(-1)).toMatchObject({
    id: "first",
    uncropped: true,
  });
  // the fixture holds 0.84 s, the slider runs to the next 10 ms
  const start = dialog.getByRole("slider", { name: "Crop start" });
  const end = dialog.getByRole("slider", { name: "Crop end" });
  await start.focus();
  for (let i = 0; i < 10; i++) await page.keyboard.press("ArrowRight");
  await end.focus();
  for (let i = 0; i < 4; i++) await page.keyboard.press("ArrowLeft");
  await expect(dialog.getByText("0.70 s kept")).toBeVisible();
  // the two-target span starts where the start thumb stands at 0.1 s and ends where a thumb at
  // 0.3 s would, radix moving a thumb in by up to half its width
  const thumb = (await start.boundingBox())!;
  const track = (await dialog.locator('[data-slot="slider-track"]').boundingBox())!;
  const mark = (await dialog.locator(".crop-crowded > span").first().boundingBox())!;
  const pct = 0.3 / 0.84;
  const at = track.x + pct * track.width + (thumb.width / 2) * (1 - 2 * pct);
  expect(Math.abs(mark.x - (thumb.x + thumb.width / 2))).toBeLessThan(1);
  expect(Math.abs(mark.x + mark.width - at)).toBeLessThan(1);
  await page.screenshot({ path: "test-results/crop-dialog.png" });
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(menu).toBeFocused();
  const first = runPlot.from_us;
  expect((await calls(page, "set_crop")).at(-1)).toEqual({
    id: "first",
    crop: { from_us: first + 100_000, to_us: first + 800_000 },
  });
  await expect(page.getByText("Cropped to 0.70 s")).toBeVisible();
  await page.screenshot({ path: "test-results/crop-row.png" });

  // it opens on the saved crop, and Reset saves none
  await menu.click();
  await page.getByRole("menuitem", { name: "Crop" }).click();
  await expect(dialog.getByText("From 0.10 s")).toBeVisible();
  await dialog.getByRole("button", { name: "Reset" }).click();
  await expect(dialog.getByRole("button", { name: "Reset" })).toBeDisabled();
  await dialog.getByRole("button", { name: "Save" }).click();
  expect((await calls(page, "set_crop")).at(-1)).toEqual({
    id: "first",
    crop: null,
  });
  await expect(page.getByText("Cropped to 0.70 s")).toHaveCount(0);
});

test("a new crop reloads Compare's plots from the start of the kept span", async ({ page }) => {
  await open(page);
  await page
    .getByRole("row", { name: /first\.pcap/ })
    .getByRole("button", { name: "Inspect" })
    .click();
  const timeline = page.getByLabel(/Report interval timeline/);
  await expect(timeline.first()).toBeVisible();
  // a brushed range first
  const box = (await timeline.first().boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.4, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.7, box.y + box.height / 2);
  await page.mouse.up();
  await expect.poll(async () => (await calls(page, "run_plot")).at(-1)?.range).not.toBeNull();
  const before = (await calls(page, "run_plot")).length;
  await page.evaluate(
    (from) =>
      (window as unknown as { cropRun: (id: string, crop: unknown) => void }).cropRun("first", {
        from_us: from + 100_000,
        to_us: from + 700_000,
      }),
    runPlot.from_us,
  );
  await expect.poll(async () => (await calls(page, "run_plot")).length).toBeGreaterThan(before);
  // the range went with the old crop, no plot was asked for with it
  const after = (await calls(page, "run_plot")).slice(before);
  expect(after.every((args) => args.range === null)).toBe(true);
  expect(after.at(-1)).toMatchObject({ id: "first", range: null });
});

test("the table's header stays under the app's header while the recordings scroll", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 700 });
  await open(page);
  await page.evaluate(() => {
    const more = document.createElement("div");
    more.style.height = "3000px";
    document.body.append(more);
  });
  const header = page.getByRole("columnheader", { name: "Source" });
  const table = page.getByRole("table", { name: "Recordings" });
  const top = (await table.boundingBox())!.y;
  await page.evaluate((y) => window.scrollTo(0, y), top + 40);
  // right under the app's header, with its gap
  const app = (await page.locator(".app-header").boundingBox())!;
  await expect
    .poll(async () => (await header.boundingBox())!.y)
    .toBeCloseTo(app.y * 2 + app.height, 0);
});

test("a recording made with target capture says why it has no trajectory", async ({ page }) => {
  await open(page);
  await expect(
    page.getByText("target capture was on but none of the 4 clicks made a trial"),
  ).toBeVisible();
  // one made without it doesn't
  await expect(page.locator(".run-status-reason")).toHaveCount(2);
});

test("a recording made with target capture replays its screen, crosshair and trail", async ({
  page,
}) => {
  await page.addInitScript(() => {
    // a 90 degree screen, still for 100 ms, then turned 100 counts right, 10 degrees, by 200 ms,
    // with a target seen to the right and clicked at 250 ms
    (window as unknown as { replayData: unknown }).replayData = {
      width: 2000,
      height: 1000,
      horizontal_fov: 90,
      degrees: [0.1, 0.1],
      starting_pitch: 0,
      from_us: 0,
      mouse: Array.from({ length: 1000 }, (_, i) => [Math.max(0, Math.min(100, i - 100)), 0]),
      clicks: [250],
      frames: [
        { at_ms: 0, boxes: [[1150, 450, 1250, 550]] },
        { at_ms: 200, boxes: [[950, 450, 1050, 550]] },
      ],
    };
  });
  await open(page);
  // only a recording with target capture has a replay
  await page.getByRole("button", { name: "More actions for first.pcap" }).click();
  await expect(page.getByRole("menuitem", { name: "Replay" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "More actions for second.pcap" }).click();
  await page.getByRole("menuitem", { name: "Replay" }).click();
  const dialog = page.getByRole("dialog", { name: "Replay" });
  await expect(dialog.getByTestId("crosshair")).toBeVisible();
  await expect(dialog.getByTestId("detected-target")).toHaveCount(1);
  await expect(dialog.getByText("0.00 s / 1.00 s")).toBeVisible();
  // the time moves while it plays, and the turn leaves a trail
  await dialog.getByRole("button", { name: "Play" }).click();
  await expect(dialog.getByRole("button", { name: "Pause" })).toBeVisible();
  await expect
    .poll(async () => (await dialog.getByTestId("replay-trail").getAttribute("d"))?.includes("L"))
    .toBe(true);
  await dialog.getByRole("button", { name: "Pause" }).click();
  await expect(dialog.getByText("0.00 s / 1.00 s")).toHaveCount(0);
  await dialog.getByRole("switch", { name: "Trail" }).click();
  await expect(dialog.getByTestId("replay-trail")).toHaveCount(0);
});

test("the table's columns are dragged wider by their edges, remembered, and reset by a double click", async ({
  page,
}) => {
  await open(page);
  const width = (name: string) =>
    page
      .getByRole("columnheader", { name, exact: true })
      .evaluate((th) => th.getBoundingClientRect().width);
  const edges = page.getByRole("separator", { name: "Resize the columns" });
  const date = await width("Date");
  const edge = (await edges.first().boundingBox())!;
  const recording = await width("Recording");
  // the edge sits on the line between the first two columns
  const first = (await page.getByRole("columnheader", { name: "Recording" }).boundingBox())!;
  expect(edge.x + edge.width / 2).toBeCloseTo(first.x + first.width, 0);
  await page.mouse.move(edge.x + edge.width / 2, edge.y + 10);
  await page.mouse.down();
  await page.mouse.move(edge.x - 80, edge.y + 10, { steps: 4 });
  await page.mouse.up();
  // the two columns trade width, the table keeps its own
  expect(await width("Recording")).toBeCloseTo(recording - 80, -1);
  expect(await width("Date")).toBeCloseTo(date + 80, -1);
  await page.reload();
  await page.getByRole("tab", { name: /Runs/ }).click();
  expect(await width("Date")).toBeCloseTo(date + 80, -1);
  await edges.first().dblclick();
  expect(await width("Date")).toBeCloseTo(date, 0);
  // the arrow keys move a focused edge too
  await edges.first().focus();
  await page.keyboard.press("ArrowRight");
  expect(await width("Recording")).toBeGreaterThan(recording + 5);
});
