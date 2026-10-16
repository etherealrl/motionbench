import { runPlot, snapshot as fixture } from "../../ui/fixture";
import { expect, test, type SavedSettings } from "./settings-mock";
import type { Page } from "@playwright/test";

// four recordings tagged two a side, which native lists the way it does any run
async function open(page: Page) {
  await page.addInitScript(
    ({ fixture, runPlot }) => {
      const snapshot = structuredClone(fixture);
      const base = snapshot.runs[0];
      snapshot.runs = (["baseline", "candidate", "baseline", "candidate"] as const).map(
        (side, i) => ({
          ...base,
          id: `run${i}`,
          name: `run${i}.pcap`,
          tag: { case: "assist" as const, side },
          started_us: (base.started_us ?? 0) + i,
        }),
      );
      const calls: { command: string; args: unknown }[] = [];
      let channel: { onmessage: (value: unknown) => void } | null = null;
      Object.assign(window, {
        isTauri: true,
        runCalls: calls,
        __TAURI_INTERNALS__: {
          metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
          transformCallback: () => 1,
          unregisterCallback: () => {},
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, args });
            if (command === "runs")
              return { revision: snapshot.library, runs: structuredClone(snapshot.runs) };
            if (command === "initialize") return { roots: [], snapshot, library_warnings: [] };
            if (command === "run_plot") return runPlot;
            if (command === "subscribe") channel = (args as { channel: typeof channel }).channel;
            if (command === "set_tag") {
              const { id, tag } = args as { id: string; tag: (typeof snapshot.runs)[0]["tag"] };
              snapshot.runs.find((r) => r.id === id)!.tag = tag;
              snapshot.library++;
              snapshot.seq++;
              channel?.onmessage(structuredClone(snapshot));
              return null;
            }
            if (command === "plugin:event|listen") return 1;
          },
        },
      });
    },
    { fixture, runPlot },
  );
  await page.goto("/");
}

test("picking the assist case shows the checklist and the side to record", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Native against an assist program" }).click();
  await expect(page.getByRole("heading", { name: "What are you comparing?" })).toBeHidden();
  const checklist = page.getByRole("region", { name: "Native against an assist program" });
  // it starts folded to its names, so it covers as little of the page as it can
  await expect(checklist.getByText("Recordings")).toBeVisible();
  await expect(checklist.getByText("2 without program and 2 with program")).toBeHidden();
  await checklist.getByRole("button", { name: "Show the hints" }).click();
  await expect(checklist.getByText("2 without program and 2 with program")).toBeVisible();
  await expect(checklist.getByText("about 0.33")).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Side" })).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as SavedSettings).savedSettings().display.purpose),
    )
    .toBe("assist");
});

test("closing the prompt counts as just looking and shows no checklist", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("region", { name: "Native against an assist program" })).toHaveCount(
    0,
  );
  await expect(page.getByRole("combobox", { name: "Side" })).toHaveCount(0);
});

test("a run's side shows in Runs and changes from its menu", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: /Runs/ }).click();
  await expect(page.getByText("With program")).toHaveCount(2);
  await page.getByRole("button", { name: "More actions for run1.pcap" }).click();
  await page.getByRole("menuitem", { name: "Side" }).click();
  // the way a hand does it, sliding over to the submenu
  const box = (await page.getByRole("menuitemradio", { name: "No side" }).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 10 });
  await page.mouse.down();
  await page.mouse.up();
  await expect(page.getByText("With program")).toHaveCount(1);
});

test("the tagged sides fill the comparison", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Native against an assist program" }).click();
  await page.getByRole("tab", { name: "Compare" }).click();
  await page.getByRole("button", { name: "Use the tagged sides" }).click();
  const saved = (key: string) => page.evaluate((key) => localStorage.getItem(key), key);
  await expect.poll(() => saved("bench-baseline")).toBe(JSON.stringify(["run0", "run2"]));
  await expect.poll(() => saved("bench-candidate")).toBe(JSON.stringify(["run1", "run3"]));
});

test("the checklist floats, a throw lands it in a corner and it folds to its names", async ({
  page,
}) => {
  await open(page);
  await page.getByRole("button", { name: "Native against an assist program" }).click();
  const panel = page.getByRole("region", { name: "Native against an assist program" });
  const viewport = page.viewportSize()!;
  const start = (await panel.boundingBox())!;
  // it starts in the bottom right corner
  expect(start.x + start.width).toBeGreaterThan(viewport.width - 40);
  expect(start.y + start.height).toBeGreaterThan(viewport.height - 40);
  // a short quick flick up and left carries it to the top left
  const grab = { x: start.x + 40, y: start.y + 12 };
  await page.mouse.move(grab.x, grab.y);
  await page.mouse.down();
  for (let i = 1; i <= 6; i++) await page.mouse.move(grab.x - i * 40, grab.y - i * 30);
  await page.mouse.up();
  await expect
    .poll(async () => {
      const box = (await panel.boundingBox())!;
      return [Math.round(box.x), Math.round(box.y)];
    })
    .toEqual([16, 16]);
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("checklist-corner")))
    .toBe(JSON.stringify({ right: false, bottom: false }));

  await expect(panel.getByText("name the scenario you play")).toBeHidden();
  await panel.getByRole("button", { name: "Show the hints" }).click();
  await expect(panel.getByText("name the scenario you play")).toBeVisible();
  await panel.getByRole("button", { name: "Hide the hints" }).click();
  await expect(panel.getByText("name the scenario you play")).toBeHidden();
  await expect(panel.getByText("Scenario")).toBeVisible();
});

test("another comparison has its own sides and checklist, and leaves the other tags out", async ({
  page,
}) => {
  await open(page);
  await page.getByRole("button", { name: "Medius with and without the renderer" }).click();
  const checklist = page.getByRole("region", { name: "Medius with and without the renderer" });
  await checklist.getByRole("button", { name: "Show the hints" }).click();
  await expect(checklist.getByText("Renderer for this recording")).toBeVisible();
  // the four recordings were tagged for the assist comparison, so none count here
  await expect(checklist.getByText("0 renderer off and 0 renderer on")).toBeVisible();
  await page.getByRole("combobox", { name: "Side" }).click();
  await expect(page.getByRole("option", { name: "Renderer on" })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Compare" }).click();
  await expect(page.getByRole("button", { name: "Use the tagged sides" })).toHaveCount(0);
});
