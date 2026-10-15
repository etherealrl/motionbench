import { numbers, runPlot, snapshot as fixture } from "../../ui/fixture";
import type { Page } from "@playwright/test";
import { expect, test } from "./settings-mock";

// the theme's data colors, as the browser computes them
const candidate = "rgb(86, 180, 239)";
const baseline = "rgb(233, 182, 111)";
const shuffled = "rgb(115, 123, 132)";

// a finished job whose result the app fetches, on the Compare tab, its judge changed by `judge`
async function showResult(page: Page, judge: Record<string, unknown> = {}) {
  await page.addInitScript(
    ({ fixture, runPlot, numbers, judge }) => {
      const snapshot = structuredClone(fixture);
      snapshot.live = true;
      (snapshot as { job: unknown }).job = {
        id: 42,
        status: "complete",
        progress: 1,
        error: null,
        result: null,
      };
      snapshot.runs = [
        { ...snapshot.runs[0], id: "baseline", name: "baseline.pcap" },
        { ...snapshot.runs[0], id: "candidate", name: "candidate.pcap" },
      ];
      localStorage.setItem("bench-baseline", JSON.stringify(["baseline"]));
      localStorage.setItem("bench-candidate", JSON.stringify(["candidate"]));
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
                revision: snapshot.library,
                runs: structuredClone(snapshot.runs),
              };
            if (command === "initialize") return { roots: [], snapshot };
            if (command === "run_plot") return runPlot;
            if (command === "run_numbers") return numbers;
            if (command === "plugin:event|listen") return 1;
            if (command === "analysis_result")
              return {
                schema: "test",
                judge_revision: "0000000",
                panel: "texture19",
                comparison: "independent populations",
                config: {
                  seed: 7,
                  folds: 5,
                  repeats: 2,
                  bootstrap: 200,
                  permutations: 2000,
                  width_ms: 160,
                },
                baseline: ["baseline"],
                candidate: ["candidate"],
                feature_names: ["speed", "jerk"],
                w1: [0.1, 0.05],
                real_features: [
                  [1, 1],
                  [1.5, 1],
                ],
                fake_features: [
                  [2, 2],
                  [2.5, 2],
                ],
                judge: {
                  auc: 0.84,
                  auc_ci: [0.72, 0.92],
                  p_value: 0.002,
                  permutation_auc: [0.43, 0.48, 0.5, 0.53, 0.57],
                  null_p95: 0.58,
                  n_groups: 4,
                  fold_auc: [
                    [0.81, 0.85],
                    [0.79, 0.84],
                  ],
                  repeat_auc: [0.83, 0.82],
                  permutation_error: null,
                  smallest_p: 0.014,
                  ...judge,
                },
                judge_unavailable: null,
                idle_dropped: [0, 0],
                alignment_shift_us: null,
                not_complete: [
                  {
                    path: "C:/runs/candidate.pcap",
                    kind: "interrupted",
                    message: "the app was closed while it was recording",
                  },
                ],
                metadata: {},
              };
          },
        },
      });
    },
    { fixture, runPlot, numbers, judge },
  );
  await page.goto("/");
  await page.getByRole("tab", { name: "Compare", exact: true }).click();
}

test("result charts draw each population and the null in their own color", async ({ page }) => {
  await showResult(page);
  const permutation = page.getByRole("img", {
    name: "Permutation AUC histogram and observed AUC",
  });
  await expect(permutation).toBeVisible();
  const fill = (selector: string) =>
    permutation
      .locator(selector)
      .first()
      .evaluate((node) => getComputedStyle(node).fill);
  const stroke = (index: number) =>
    permutation
      .locator("line")
      .nth(index)
      .evaluate((node) => getComputedStyle(node).stroke);
  expect(await fill("rect")).toBe(shuffled);
  // the observed AUC, then the chance line
  expect(await stroke(0)).toBe(candidate);
  expect(await stroke(1)).toBe(baseline);
  await page.locator(".feature-row").first().click();
  const distribution = page.getByRole("img", {
    name: "Distributions for speed",
  });
  // each bin draws the baseline bar first, then the candidate bar
  const fills = (nth: number) =>
    distribution
      .locator(`g > rect:nth-child(${nth})`)
      .evaluateAll((all) => [...new Set(all.map((node) => getComputedStyle(node).fill))]);
  expect(await fills(1)).toEqual([baseline]);
  expect(await fills(2)).toEqual([candidate]);
});

test("a result says which of its recordings didn't end complete", async ({ page }) => {
  await showResult(page);
  await expect(
    page.getByText(
      "candidate.pcap didn't end complete, the app was closed while it was recording, so read this result with that in mind",
    ),
  ).toBeVisible();
});

test("too few recordings for p to show anything say so, and an interval the folds can't give isn't shown", async ({
  page,
}) => {
  await showResult(page, { smallest_p: 1 / 3, auc_ci: null });
  await expect(
    page.getByText(
      "with 4 recordings the shuffles can't give a p under 0.33, so p can't show a difference here however big it is. 4 recordings a side get it under 0.05",
    ),
  ).toBeVisible();
  await expect(
    page.getByText("no interval, each held-out part has only one recording a side"),
  ).toBeVisible();
  await expect(page.getByText(/95% CI/)).toHaveCount(0);
});

test("enough recordings for p show the interval and no warning", async ({ page }) => {
  await showResult(page);
  await expect(page.getByText("95% CI 0.720 to 0.920")).toBeVisible();
  await expect(page.getByText(/can't give a p under/)).toHaveCount(0);
});

test("every view of one recording is drawn again for the baseline, in its color", async ({
  page,
}) => {
  await showResult(page);
  await page.getByRole("tab", { name: "Timing" }).click();
  const title = (name: string) => page.getByRole("heading", { name, exact: true });
  for (const name of [
    "Displacement against interval, candidate",
    "Displacement against interval, baseline",
    "Bursts, candidate",
    "Bursts, baseline",
  ])
    await expect(title(name)).toBeVisible();
  await page.getByRole("tab", { name: "Movement" }).click();
  await expect(title("Counts over time, baseline")).toBeVisible();
  await page.getByText("Count-space path").click();
  await expect(title("Path, candidate")).toBeVisible();
  await expect(title("Path, baseline")).toBeVisible();
  await page.getByRole("tab", { name: "Relay" }).click();
  for (const name of ["Counts conserved", "Matching", "Offset variation"]) {
    await expect(title(`${name}, candidate`)).toBeVisible();
    await expect(title(`${name}, baseline`)).toBeVisible();
  }
  const bar = (side: string) =>
    page
      .locator(`[data-recording="${side}"] .delivery-track > div`)
      .first()
      .evaluate((node) => getComputedStyle(node).backgroundColor);
  expect(await bar("candidate")).toBe(candidate);
  expect(await bar("baseline")).toBe(baseline);
  await page.locator(".analysis-tabs").screenshot({ path: "test-results/relay-both.png" });
});

test("the recording picker sorts beside its search", async ({ page }) => {
  await showResult(page);
  await page.getByRole("button", { name: "Choose baseline recordings" }).click();
  const names = () =>
    page
      .getByRole("group", { name: "Baseline recordings" })
      .locator(".recording-choice > span")
      .evaluateAll((all) => all.map((node) => node.firstChild?.textContent));
  const sort = page.getByRole("combobox", { name: "Sort baseline recordings" });
  const search = page.getByRole("searchbox", {
    name: "Search baseline recordings",
  });
  // on the search's row, at its right
  const [s, o] = [(await search.boundingBox())!, (await sort.boundingBox())!];
  expect(Math.abs(s.y + s.height / 2 - (o.y + o.height / 2))).toBeLessThan(1);
  expect(o.x).toBeGreaterThan(s.x + s.width - 1);
  await sort.click();
  await page.getByRole("option", { name: "Name Z to A" }).click();
  expect(await names()).toEqual(["candidate.pcap", "baseline.pcap"]);
  await sort.click();
  await page.getByRole("option", { name: "Name A to Z" }).click();
  expect(await names()).toEqual(["baseline.pcap", "candidate.pcap"]);
  await page.locator(".recording-popover").screenshot({ path: "test-results/picker-sort.png" });
});

test("the app's header stays at the top while the page scrolls", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 600 });
  await showResult(page);
  const header = page.locator(".app-header");
  const before = (await header.boundingBox())!.y;
  await page.evaluate(() => window.scrollTo(0, 1500));
  await expect.poll(async () => (await header.boundingBox())!.y).toBeLessThanOrEqual(before);
  expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(500);
  await expect(page.getByRole("tab", { name: "Runs" })).toBeInViewport();
  await page.screenshot({ path: "test-results/sticky-header.png" });
});

test("the numbers table under its group headings gets one edge between each of its columns", async ({
  page,
}) => {
  await showResult(page);
  const table = page.locator(".numbers-table");
  await expect(table.getByRole("columnheader", { name: "Difference" })).toBeVisible();
  const columns = await table.locator("thead tr:last-child th").count();
  await expect(table.locator("col")).toHaveCount(columns);
  await expect(table.getByRole("separator", { name: "Resize the columns" })).toHaveCount(
    columns - 1,
  );
});
