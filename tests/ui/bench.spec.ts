import { connectedBox, numbers, runPlot, snapshot as fixture } from "../../ui/fixture";
import { expect, test, type SavedSettings } from "./settings-mock";
import type { Locator, Page } from "@playwright/test";
import type { Blocker } from "../../ui/types";
import { previewSettings } from "../../ui/lib/preview-settings";

async function chooseOption(trigger: Locator, value: string) {
  await trigger.click();
  await trigger
    .page()
    .getByRole("option")
    .and(trigger.page().locator("[data-value=" + JSON.stringify(value) + "]"))
    .click();
}

async function openRecordingPicker(page: Page, name: "Baseline" | "Candidate") {
  const group = page.getByRole("group", { name: `${name} recordings` });
  if (await group.isVisible()) return;
  await page.keyboard.press("Escape");
  await page
    .getByRole("button", {
      name: `Choose ${name.toLowerCase()} recordings`,
      exact: true,
    })
    .click();
}

test("timing evidence gates observer readiness and response calibration keeps uncertainty visible", async ({
  page,
}) => {
  // the longest walk through the app, it needs more than the default time when the suite runs
  // in parallel
  test.slow();
  await page.addInitScript((fixture) => {
    const profile = {
      provider: "CUDA",
      cuda_device: 0,
      cuda_graph: true,
      model: "model.onnx",
      scenario: "stationary calibration",
      adapter: 0,
      output: 0,
      fov: 103,
      fov_axis: "horizontal",
      sensitivity: 1.6,
      yaw: 0.07,
      pitch: 0.07,
      confidence: 0.5,
      target_class: 0,
      fps: 0,
      display_delay_ms: 0,
      max_frame_gap_ms: 50,
      max_residual_radius: 0.5,
    };
    (window as unknown as SavedSettings).seedSettings({ vision: profile });
    const observer = {
      raw_ready: false,
      kernel_ready: false,
      kernel_seen: false,
      recording: false,
      reports: 0,
      batches: 0,
      max_batch: 0,
      queue_losses: 0,
      kernel_losses: 0,
      os_loss_count_known: false,
      error: null as string | null,
      kernel_status: "Signed mouse-class observer is not installed",
      devices: {},
      kernel_devices: {},
    };
    const snapshot = {
      ...structuredClone(fixture),
      live: true,
      observers: observer,
    };
    // native decides what blocks Record; each step sets what it would say
    snapshot.readiness.record = {
      need: "observer_starting",
      message: "waiting for the input observer",
      waits: true,
    };
    const requests: { command: string; args: Record<string, unknown> }[] = [];
    let identifiable = false;
    const calibration = {
      estimated_delay_ms: 37,
      admissible_delay_ms: [0, 250],
      residual_counts: 1,
      radius_counts: 30,
      movement_span_counts: 500,
      frames: 500,
      span_ms: 5000,
      identifiable: false,
      reason: "multiple delays fit",
      residual_curve: [
        [0, 5],
        [37, 1],
        [250, 10],
      ],
      profile,
    };
    Object.assign(window, {
      isTauri: true,
      setObserverReady: () => {
        observer.raw_ready = true;
        snapshot.readiness.record = null;
      },
      setObserverError: () => {
        observer.error = "observer writer queue lost batches";
        snapshot.readiness.record = {
          need: "observer_failed",
          message:
            "the input observer failed, restart monitoring to record: observer writer queue lost batches",
          waits: false,
        };
      },
      endCapture: () => (snapshot.live = false),
      setCalibrationValid: () => (identifiable = true),
      timingRequests: requests,
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          requests.push({ command, args });
          if (command === "runs")
            return {
              revision: snapshot.library,
              runs: structuredClone(snapshot.runs),
            };
          if (command === "initialize") return { roots: [], snapshot };
          if (command === "subscribe") {
            const channel = args.channel as { onmessage: (v: unknown) => void };
            setInterval(() => channel.onmessage(structuredClone(snapshot)), 50);
          }
          if (command === "run_plot") return fixture.plot;
          if (command === "calibrate_response")
            return {
              ...calibration,
              identifiable,
              reason: identifiable ? null : "multiple delays fit",
              admissible_delay_ms: identifiable ? [35, 39] : [0, 250],
            };
        },
      },
    });
  }, fixture);
  await page.goto("/");
  await expect(page.getByText("Monitoring", { exact: true })).toHaveCount(1);
  await expect(page.locator(".global-session-status")).toHaveText("Monitoring");
  await expect(page.locator('.global-session-status [data-slot="badge"]')).toHaveAttribute(
    "data-variant",
    "success",
  );
  // what is off shows no badge: target capture is off and no box is connected
  await expect(page.locator(".setup-panel").locator('[data-slot="badge"]')).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Connect box", exact: true })).toBeVisible();
  await expect(page.locator(".section-intro").getByText("Monitoring", { exact: true })).toHaveCount(
    0,
  );
  await expect(page.getByRole("tab").locator("svg")).toHaveCount(0);
  await page.getByRole("tab", { name: "Compare", exact: true }).click();
  await expect(page.getByText("Monitoring", { exact: true })).toHaveCount(1);
  await expect(page.locator(".global-session-status")).toHaveText("Monitoring");
  await page.getByRole("tab", { name: "Capture", exact: true }).click();
  await page.locator(".app-header").screenshot({ path: "test-results/header-single-status.png" });
  const record = page.getByRole("button", { name: "Record", exact: true });
  const recordRow = page.locator(".record-row");
  await expect(record).toBeDisabled();
  await expect(recordRow).toContainText("waiting for the input observer");
  await page.evaluate(() =>
    (window as unknown as { setObserverReady: () => void }).setObserverReady(),
  );
  await expect(record).toBeEnabled();
  await page.evaluate(() =>
    (window as unknown as { setObserverError: () => void }).setObserverError(),
  );
  await expect(record).toBeDisabled();
  await expect(recordRow).toContainText(
    "the input observer failed, restart monitoring to record: observer writer queue lost batches",
  );
  await recordRow.screenshot({ path: "test-results/record-blocked.png" });
  await expect(
    page.locator(".section-intro").getByText("Observer error", { exact: true }),
  ).toHaveAttribute("data-variant", "warning");
  await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
  await expect(page.locator(".status-diagnostics").getByRole("alert")).toContainText(
    "lost batches",
  );
  await page.keyboard.press("Escape");
  // the clocks of a saved recording are read from its row in Runs
  await page.getByRole("tab", { name: /Runs/ }).click();
  await page.getByRole("button", { name: `More actions for ${fixture.runs[0].name}` }).click();
  await page.getByRole("menuitem", { name: "Recording clocks" }).click();
  const clocks = page.getByRole("dialog", { name: "Recording clocks" });
  await expect(clocks.getByText("Clock anchors", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Capture", exact: true }).click();
  await page.evaluate(() => (window as unknown as { endCapture: () => void }).endCapture());
  await openSection(page, "Target capture");
  await page.getByText("Response delay", { exact: true }).click();
  const estimate = page.getByRole("button", {
    name: "Estimate delay",
    exact: true,
  });
  await chooseOption(page.getByLabel("Calibration recording"), fixture.runs[0].id);
  await expect(estimate).toBeDisabled();
  await page.getByLabel("Stationary target", { exact: true }).check();
  await estimate.click();
  await expect(page.getByText("multiple delays fit", { exact: true })).toBeVisible();
  // the section keeps the estimate while it is folded
  await page.getByRole("button", { name: "Target capture", exact: true }).click();
  await openSection(page, "Target capture");
  await page.getByText("Response delay", { exact: true }).click();
  await expect(page.getByText("multiple delays fit", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Stationary target", { exact: true })).toBeChecked();
  const apply = page.getByRole("button", { name: "Use for future captures" });
  await expect(apply).toBeDisabled();
  await page.evaluate(() =>
    (window as unknown as { setCalibrationValid: () => void }).setCalibrationValid(),
  );
  await estimate.click();
  await expect(apply).toBeEnabled();
  await apply.click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as SavedSettings).savedSettings().vision.display_delay_ms,
      ),
    )
    .toBe(37);
  const request = await page.evaluate(
    () =>
      (
        window as unknown as {
          timingRequests: { command: string; args: unknown }[];
        }
      ).timingRequests.find((v) => v.command === "calibrate_response")?.args,
  );
  expect(request).toEqual({ id: fixture.runs[0].id, stationaryTarget: true });
  if (!(await page.getByLabel("UI scale").isVisible())) await page.getByLabel("Settings").click();
  for (const width of [1600, 1000]) {
    await page.setViewportSize({ width, height: 900 });
    for (const zoom of ["1", "1.25", "1.5"]) {
      await chooseOption(page.getByLabel("UI scale"), zoom);
      await expectControlsToFit(page);
    }
  }
  await page.setViewportSize({ width: 1600, height: 1100 });
  await chooseOption(page.getByLabel("UI scale"), "1.25");
  await page
    .getByRole("group", { name: "Response delay" })
    .screenshot({ path: "test-results/response-calibration.png" });
});

test("library restore gates saved selections and deleted runs survive restart", async ({
  page,
}) => {
  await page.addInitScript((fixture) => {
    let channel: { onmessage: (v: unknown) => void } | null = null;
    let initialized = false;
    const deleted = new Set<string>(JSON.parse(localStorage.getItem("test-deleted") ?? "[]"));
    const baseline = {
      ...fixture.runs[0],
      id: "baseline",
      name: "native.pcap",
      result: null,
    };
    const candidate = {
      ...fixture.runs[0],
      id: "candidate",
      name: "rendered.pcap",
      result: null,
    };
    let serverRuns = [baseline, candidate].filter((run) => !deleted.has(run.id));
    const snapshot = { ...structuredClone(fixture), runs: serverRuns };
    const requests: { command: string; args: Record<string, unknown> }[] = [];
    if (!localStorage.getItem("test-started")) {
      localStorage.setItem("test-started", "true");
      localStorage.setItem("bench-baseline", JSON.stringify(["baseline"]));
      localStorage.setItem("bench-candidate", JSON.stringify(["candidate", "missing-file"]));
    }
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          requests.push({ command, args });
          if (command === "runs") {
            // the runs take a moment, as across the native boundary
            await new Promise((resolve) => setTimeout(resolve, 150));
            return {
              revision: snapshot.library,
              runs: structuredClone(snapshot.runs),
            };
          }
          if (command === "initialize") {
            await new Promise((resolve) => setTimeout(resolve, 400));
            initialized = true;
            return { roots: [], snapshot };
          }
          if (command === "subscribe") {
            channel = args.channel as typeof channel;
            setInterval(() => channel?.onmessage(structuredClone(snapshot)), 50);
          }
          if (command === "run_plot") {
            if (!initialized || !serverRuns.some((run) => run.id === args.id))
              throw "run is unavailable";
            await new Promise((resolve) => setTimeout(resolve, 120));
            return fixture.plot;
          }
          if (command === "delete_run") {
            if ((window as unknown as { failDelete?: boolean }).failDelete)
              throw "could not save the recording library";
            deleted.add(args.id as string);
            localStorage.setItem("test-deleted", JSON.stringify([...deleted]));
            serverRuns = serverRuns.filter((run) => run.id !== args.id);
            setTimeout(() => {
              snapshot.runs = serverRuns;
              snapshot.library++;
            }, 350);
          }
          if (command === "plugin:dialog|open") return ["rendered.pcap"];
          if (command === "import_run") {
            deleted.delete(candidate.id);
            localStorage.setItem("test-deleted", JSON.stringify([...deleted]));
            serverRuns = [...serverRuns, candidate];
            setTimeout(() => {
              snapshot.runs = serverRuns;
              snapshot.library++;
            }, 350);
            return candidate;
          }
          if (command === "plugin:event|listen") return 1;
        },
      },
      isTauri: true,
      benchRequests: requests,
    });
  }, fixture);
  await page.goto("/");
  await expect(page).toHaveTitle("Motion Bench");
  await expect(page.getByRole("heading", { name: "Motion Bench", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Compare" }).click();
  // the recording pickers look like the selects beside them
  const background = (element: Locator) =>
    element.evaluate((node) => getComputedStyle(node).backgroundColor);
  expect(await background(page.locator(".population-trigger").first())).toBe(
    await background(page.getByLabel("Comparison mode")),
  );
  await openRecordingPicker(page, "Baseline");
  await expect(page.getByLabel("Baseline recordings").getByLabel("native.pcap")).toBeChecked();
  await openRecordingPicker(page, "Candidate");
  await expect(page.getByLabel("Candidate recordings").getByLabel("rendered.pcap")).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(page.locator(".metric-grid")).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("bench-candidate")!))).toEqual([
    "candidate",
  ]);
  expect(
    await page.evaluate(() => {
      const calls = (window as unknown as { benchRequests: { command: string }[] }).benchRequests;
      return (
        calls.findIndex((call) => call.command === "subscribe") <
        calls.findIndex((call) => call.command === "run_plot")
      );
    }),
  ).toBe(true);
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: /Runs/ }).click();
  await removeRecording(page, "rendered.pcap");
  const dialog = page.getByRole("alertdialog");
  await expect(dialog.getByRole("heading", { name: "Remove from library?" })).toBeVisible();
  await expect(
    dialog.getByText("The capture files and saved results stay on disk", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(recording(page, "rendered.pcap")).toBeVisible();
  await removeRecording(page, "rendered.pcap");
  await page.evaluate(() => Object.assign(window, { failDelete: true }));
  await dialog.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("could not save");
  await expect(dialog).toBeVisible();
  await expect(recording(page, "rendered.pcap")).toHaveCount(1);
  await page.evaluate(() => Object.assign(window, { failDelete: false }));
  await dialog.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(recording(page, "rendered.pcap")).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("bench-candidate")!))).toEqual(
    [],
  );
  await page.reload();
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: /Runs/ }).click();
  await expect(recording(page, "native.pcap")).toBeVisible();
  await expect(recording(page, "rendered.pcap")).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.getByRole("button", { name: "Import", exact: true }).click();
  await openRecordingPicker(page, "Candidate");
  await expect(page.getByLabel("Candidate recordings").getByLabel("rendered.pcap")).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(page.locator(".metric-grid")).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: /Runs/ }).click();
  await expect(recording(page, "rendered.pcap")).toBeVisible();
});

// opens a Capture setup section by its title, if it is folded
async function openSection(page: Page, name: "Target capture" | "Box" | "Medius") {
  const trigger = page.getByRole("button", { name, exact: true });
  if ((await trigger.getAttribute("aria-expanded")) !== "true") await trigger.click();
}

// a recording's name in the library table
function recording(page: Page, name: string) {
  // by class, not role: an open dialog hides the table from the accessibility tree
  return page.locator(".run-table").getByText(name, { exact: true });
}

async function removeRecording(page: Page, name: string) {
  await page.getByRole("button", { name: `More actions for ${name}` }).click();
  await page.getByRole("menuitem", { name: "Remove from library" }).click();
}

async function expectControlsToFit(page: Page) {
  const clipped = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>("button, input, select")].flatMap((element) => {
      const panel = element.closest(".panel, .app-header, .range-toolbar, .section-intro");
      if (!panel || panel.classList.contains("run-table")) return [];
      const bounds = element.getBoundingClientRect();
      if (!bounds.width || !bounds.height) return [];
      const container = panel.getBoundingClientRect();
      return bounds.left < container.left - 1 || bounds.right > container.right + 1
        ? [element.getAttribute("aria-label") ?? element.textContent?.trim() ?? element.tagName]
        : [];
    }),
  );
  expect(clipped).toEqual([]);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
    ),
  ).toBe(true);
}
test("recorded preview charts, tabs, keyboard and display scales", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  await expect(page.locator(".brand").getByText("Preview", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop monitoring" })).toBeDisabled();
  // the timeline and the Timing tab's density; the Movement tab's two charts once it is chosen
  const heights = () =>
    page.locator("canvas").evaluateAll((all) => all.map((c) => getComputedStyle(c).height));
  await expect(page.locator("canvas")).toHaveCount(2);
  // sizes reach the stylesheet through custom properties, not inline properties
  expect(await heights()).toEqual(["200px", "180px"]);
  // the counts over time, and the path folded in its disclosure
  await page.getByRole("tab", { name: "Movement" }).click();
  await expect(page.locator("canvas")).toHaveCount(3);
  expect(await heights()).toEqual(["200px", "160px", "220px"]);
  await page.getByRole("tab", { name: "Timing" }).click();
  const bars = await page
    .locator(".delivery-track > div")
    .evaluateAll((all) =>
      all.map((bar) => [
        (bar as HTMLElement).offsetWidth / (bar.parentElement as HTMLElement).offsetWidth,
        parseFloat((bar as HTMLElement).style.getPropertyValue("--share")) / 100,
      ]),
    );
  expect(bars.length).toBeGreaterThan(0);
  for (const [drawn, share] of bars) expect(drawn).toBeCloseTo(share, 2);
  for (const width of [1000, 1172, 1600]) {
    await page.setViewportSize({ width, height: 1000 });
    // the labels keep clear space between them; line tabs sit their own boxes close together
    const tabs = await page
      .locator(".app-header")
      .getByRole("tab")
      .evaluateAll((tabs) =>
        tabs.map((tab) => {
          const range = document.createRange();
          range.selectNodeContents(tab);
          const rect = range.getBoundingClientRect();
          return { left: rect.left, right: rect.right };
        }),
      );
    for (let index = 1; index < tabs.length; index++)
      expect(tabs[index].left - tabs[index - 1].right).toBeGreaterThanOrEqual(12);
  }
  await page.setViewportSize({ width: 1172, height: 834 });
  await page.locator(".app-header").screenshot({ path: "test-results/navigation-spacing.png" });
  await page.screenshot({ path: "test-results/preview.png", fullPage: true });
  const capture = page.getByRole("tab", { name: "Capture" });
  await capture.focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "Compare", exact: true })).toBeVisible();
  await page.screenshot({ path: "test-results/compare.png", fullPage: true });
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: /Runs/ }).click();
  await expect(page.getByRole("button", { name: "Inspect" })).toBeVisible();
  await page.screenshot({ path: "test-results/runs.png", fullPage: true });
  await page.setViewportSize({ width: 1000, height: 720 });
  if (!(await page.getByLabel("UI scale").isVisible())) await page.getByLabel("Settings").click();
  for (const scale of ["1", "1.25", "1.5"]) {
    await chooseOption(page.getByLabel("UI scale"), scale);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1,
    );
    expect(overflow).toBe(false);
  }
  await page.keyboard.press("Escape");
  await expect(page.getByLabel("UI scale")).toBeHidden();
  await expect(page.getByRole("button", { name: "Diagnostics", exact: true })).toHaveCount(0);
  await page.getByRole("tab", { name: "Capture", exact: true }).click();
  await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
  await expect(page.getByText("Clock regressions", { exact: true })).toBeVisible();
  // every line of a group is a label and its value, no stray text between them
  expect(
    await page
      .locator(".status-diagnostics")
      .getByRole("region", { name: "Capture" })
      .locator("dl")
      .evaluate((dl) => [...dl.children].map((node) => node.nodeName)),
  ).toEqual(["DT", "DD", "DT", "DD", "DT", "DD", "DT", "DD"]);
  await page.keyboard.press("Escape");
  await expect(page.getByText("Clock regressions", { exact: true })).toBeHidden();
  await page.getByRole("tab", { name: "Experiment", exact: true }).click();
  await expect(
    page.getByText("arming only works in the desktop app", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Capture setup", exact: true })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("native command setup, brushing and cancellation", async ({ page }) => {
  await page.addInitScript(
    ({ fixture, runPlot, numbers }) => {
      const callbacks = new Map<number, (v: unknown) => void>();
      let callbackId = 0;
      let channel: { onmessage: (v: unknown) => void } | null = null;
      const snapshot = structuredClone(fixture);
      snapshot.live = true;
      snapshot.runs = [
        { ...snapshot.runs[0], id: "baseline", name: "baseline.pcap" },
        { ...snapshot.runs[0], id: "baseline2", name: "baseline2.pcap" },
        { ...snapshot.runs[0], id: "candidate", name: "candidate.pcap" },
      ];
      const requests: { command: string; args: Record<string, unknown> }[] = [];
      const api = {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: (callback: (v: unknown) => void) => {
          callbacks.set(++callbackId, callback);
          return callbackId;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          requests.push({ command, args });
          if (command === "runs")
            return {
              revision: snapshot.library,
              runs: structuredClone(snapshot.runs),
            };
          if (command === "initialize") return { roots: [], snapshot };
          if (command === "subscribe") {
            channel = args.channel as typeof channel;
            setInterval(() => channel?.onmessage(structuredClone(snapshot)), 100);
            return;
          }
          if (command === "run_plot") return runPlot;
          if (command === "run_numbers")
            return {
              ...numbers,
              rate_hz: { baseline: 1000, baseline2: 1200 }[args.id as string] ?? 2000,
            };
          if (command === "analyze") {
            // the first run is cancelled below, the second one finishes at once
            const id = (snapshot.job as { id?: number } | null)?.id === 1 ? 2 : 1;
            snapshot.job = {
              id,
              status: id === 1 ? "running" : "complete",
              progress: id === 1 ? 0.2 : 1,
              error: null,
              result: null,
            } as never;
            channel?.onmessage(structuredClone(snapshot));
            return id;
          }
          if (command === "analysis_result")
            return {
              schema: "test",
              judge_revision: "0000000",
              panel: "texture19",
              comparison: "independent populations",
              config: {
                seed: 7,
                folds: 5,
                repeats: 5,
                bootstrap: 200,
                permutations: 2000,
                width_ms: 160,
              },
              baseline: ["baseline"],
              candidate: ["candidate"],
              feature_names: ["speed", "jerk"],
              w1: [0.1, 0.05],
              real_features: [[1, 1]],
              fake_features: [[2, 2]],
              judge: null,
              judge_unavailable: "needs more groups",
              idle_dropped: [0, 0],
              alignment_shift_us: null,
              not_complete: [],
              metadata: {},
            };
          if (command === "plugin:dialog|save") return "out.json";
          if (command === "cancel_analysis") {
            snapshot.job = {
              id: 1,
              status: "cancelled",
              progress: 0.2,
              error: "analysis cancelled",
              result: null,
            } as never;
            channel?.onmessage(structuredClone(snapshot));
          }
          if (command === "plugin:event|listen") return 1;
        },
      };
      Object.assign(window, {
        __TAURI_INTERNALS__: api,
        isTauri: true,
        benchRequests: requests,
      });
    },
    { fixture, runPlot, numbers },
  );
  await page.goto("/");
  const liveCanvas = page.locator("canvas").first();
  await expect(liveCanvas).toBeVisible();
  await liveCanvas.scrollIntoViewIfNeeded();
  const liveRect = (await liveCanvas.boundingBox())!;
  await page.mouse.move(liveRect.x + liveRect.width * 0.253, liveRect.y + 40);
  await page.mouse.down({ button: "right" });
  await page.mouse.move(liveRect.x + liveRect.width * 0.637, liveRect.y + 40);
  await page.mouse.up({ button: "right" });
  await page.keyboard.press("Escape");
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { benchRequests: { command: string }[] }).benchRequests.filter(
          (r) => r.command === "set_range",
        ).length,
    ),
  ).toBe(0);
  await page.mouse.move(liveRect.x + liveRect.width * 0.253, liveRect.y + 40);
  await page.mouse.down();
  await page.mouse.move(liveRect.x + liveRect.width * 0.637, liveRect.y + 40);
  await page.mouse.up();
  const liveRange = await page.evaluate(
    () =>
      (
        window as unknown as {
          benchRequests: {
            command: string;
            args: { range?: { from_us: number; to_us: number } };
          }[];
        }
      ).benchRequests.find((r) => r.command === "set_range")?.args.range,
  );
  expect(Number.isSafeInteger(liveRange?.from_us)).toBe(true);
  expect(Number.isSafeInteger(liveRange?.to_us)).toBe(true);
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Compare" }).click();
  const baselines = page.getByRole("group", { name: "Baseline recordings" });
  const candidates = page.getByRole("group", { name: "Candidate recordings" });
  await openRecordingPicker(page, "Baseline");
  await baselines.getByRole("checkbox", { name: "baseline.pcap" }).check();
  await openRecordingPicker(page, "Baseline");
  await baselines.getByRole("checkbox", { name: "baseline2.pcap" }).check();
  await openRecordingPicker(page, "Candidate");
  await candidates.getByRole("checkbox", { name: "candidate.pcap" }).check();
  await openRecordingPicker(page, "Baseline");
  await expect(baselines.getByRole("checkbox", { name: "baseline.pcap" })).toBeChecked();
  await openRecordingPicker(page, "Baseline");
  await expect(baselines.getByRole("checkbox", { name: "baseline2.pcap" })).toBeChecked();
  await page.reload();
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Compare" }).click();
  await openRecordingPicker(page, "Baseline");
  await expect(baselines.getByRole("checkbox", { name: "baseline.pcap" })).toBeChecked();
  await openRecordingPicker(page, "Baseline");
  await expect(baselines.getByRole("checkbox", { name: "baseline2.pcap" })).toBeChecked();
  await openRecordingPicker(page, "Candidate");
  await expect(candidates.getByRole("checkbox", { name: "candidate.pcap" })).toBeChecked();
  await page.keyboard.press("Escape");
  const canvas = page.locator("canvas").first();
  await expect(canvas).toBeVisible();
  await expect(page.locator(".chart-scope")).toContainText(
    "the analysis uses all 3 selected recordings",
  );
  await chooseOption(page.getByRole("combobox", { name: "Baseline chart recording" }), "baseline2");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              benchRequests: { command: string; args: { id?: string } }[];
            }
          ).benchRequests
            .filter((r) => r.command === "run_plot")
            .at(-1)?.args.id,
      ),
    )
    .toBe("baseline2");
  await expect(page.locator(".delivery-track > i").first()).toBeVisible();
  const markers = await page
    .locator(".delivery-track > i")
    .evaluateAll((all) =>
      all.map((mark) => [
        (mark as HTMLElement).offsetLeft / (mark.parentElement as HTMLElement).offsetWidth,
        parseFloat((mark as HTMLElement).style.getPropertyValue("--base")) / 100,
      ]),
    );
  for (const [drawn, base] of markers) expect(drawn).toBeCloseTo(base, 2);
  await canvas.scrollIntoViewIfNeeded();
  const rect = (await canvas.boundingBox())!;
  await page.mouse.move(rect.x + rect.width * 0.2, rect.y + 40);
  await page.mouse.down();
  await page.mouse.move(rect.x + rect.width * 0.6, rect.y + 40);
  await page.mouse.up();
  await expect(page.getByPlaceholder("from s")).not.toHaveValue("");
  await page.getByRole("button", { name: "Run comparison" }).click();
  await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByText("analysis cancelled", { exact: true })).toBeVisible();
  // the cancel names its job, so it can never stop a newer one
  const cancel = await page.evaluate(
    () =>
      (
        window as unknown as {
          benchRequests: { command: string; args: { id?: number } }[];
        }
      ).benchRequests.find((r) => r.command === "cancel_analysis")?.args,
  );
  expect(cancel).toEqual({ id: 1 });
  // a finished run is fetched and exported by its own id, never whichever job is current
  await page.getByRole("button", { name: "Run comparison" }).click();
  // the export sits in the tab row beside the legend, and the judge's revision links its commit
  await expect(
    page.locator(".analysis-tabs-head").getByRole("button", { name: "Export analysis" }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: /^ABCurves / })).toHaveAttribute(
    "href",
    "https://github.com/optima-manent/ABCurves/commit/0000000",
  );
  await page.locator(".analysis-tabs").screenshot({ path: "test-results/comparison-head.png" });
  await page.getByRole("button", { name: "Export analysis" }).click();
  const named = await page.evaluate(() =>
    (
      window as unknown as {
        benchRequests: { command: string; args: Record<string, unknown> }[];
      }
    ).benchRequests.filter((r) => ["analysis_result", "export_result"].includes(r.command)),
  );
  expect(named.find((r) => r.command === "analysis_result")?.args).toEqual({
    id: 2,
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              benchRequests: { command: string; args: { source?: unknown } }[];
            }
          ).benchRequests.find((r) => r.command === "export_result")?.args.source,
      ),
    )
    .toEqual({ kind: "job", id: 2 });
  // each feature bar is drawn at its share of the largest distance
  const distances = await page
    .locator(".feature-row i")
    .evaluateAll((all) =>
      all.map(
        (bar) => (bar as HTMLElement).offsetWidth / (bar.parentElement as HTMLElement).offsetWidth,
      ),
    );
  expect(distances).toHaveLength(2);
  expect(distances[0]).toBeCloseTo(1, 2);
  expect(distances[1]).toBeCloseTo(0.5, 2);
  // the finished result opened the Comparison tab: an AUC it could not give says why, and
  // every source recording's numbers stand beside each condition's median
  const auc = page.locator(".metric-grid > div").filter({ hasText: "Held-out AUC" });
  await expect(auc.locator("strong")).toHaveText("Unavailable");
  await expect(auc.locator("small")).toContainText("1 recording against 1 recording");
  await expect(auc.locator("small")).toContainText("needs more groups");
  const rate = page.getByRole("row").filter({ hasText: "Active report rate" });
  await expect(rate.getByRole("cell")).toHaveText([
    "Active report rate",
    "1,000 Hz",
    "1,200 Hz",
    "1,100 Hz",
    "2,000 Hz",
    "2,000 Hz",
    "+900 Hz",
  ]);
  // shares are apart in percentage points, and a difference that rounds to nothing has no sign
  const bunched = page.getByRole("row").filter({ hasText: "Bunched" });
  await expect(bunched.getByRole("cell").last()).toHaveText("0.0 pp");
  await expect(page.getByRole("row").filter({ hasText: "Moving intervals" })).toContainText(
    String(numbers.moving),
  );
  const request = await page.evaluate(
    () =>
      (
        window as unknown as {
          benchRequests: {
            command: string;
            args: { request?: { range: number[]; matching: boolean } };
          }[];
        }
      ).benchRequests.find((r) => r.command === "analyze")?.args.request,
  );
  expect(request?.range[1]).toBeGreaterThan(request!.range[0]);
  expect(request?.matching).toBe(false);
  expect((request as unknown as { baseline: string[] }).baseline).toEqual([
    "baseline",
    "baseline2",
  ]);
});

test("recording search preserves selection and keeps comparison sides exclusive", async ({
  page,
}) => {
  await page.goto("/");
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Compare", exact: true }).click();
  const baseline = page.getByRole("group", { name: "Baseline recordings" });
  const candidate = page.getByRole("group", { name: "Candidate recordings" });
  const candidateSearch = page.getByRole("searchbox", {
    name: "Search candidate recordings",
  });
  await openRecordingPicker(page, "Candidate");
  await candidate.getByRole("checkbox").check();
  await candidateSearch.fill("no matching recording");
  await expect(candidate.getByText("No matching recordings")).toBeVisible();
  await candidateSearch.clear();
  await expect(candidate.getByRole("checkbox")).toBeChecked();
  await openRecordingPicker(page, "Baseline");
  await baseline.getByRole("checkbox").check();
  await openRecordingPicker(page, "Candidate");
  await expect(candidate.getByRole("checkbox")).not.toBeChecked();
  await openRecordingPicker(page, "Baseline");
  await expect(baseline.getByRole("checkbox")).toBeChecked();
  await page.keyboard.press("Escape");
  await chooseOption(page.getByLabel("Comparison mode"), "paired");
  await expect(page.getByRole("heading", { name: "Physical input", exact: true })).toBeVisible();
  await openRecordingPicker(page, "Candidate");
  await candidate.getByRole("checkbox").check();
  await expect(
    page.getByText(
      "mouse-1000hz.pcap can't be compared this way, it has no physical input, the box didn't record the mouse",
    ),
  ).toBeVisible();
});

test("vision FPS, recording while the model warms up, circle geometry and stale frames", async ({
  page,
}) => {
  await page.addInitScript((fixture) => {
    let channel: { onmessage: (value: unknown) => void } | null = null;
    (window as unknown as SavedSettings).seedSettings({
      vision: { scenario: "Single-target task", model: "C:/models/heads.onnx" },
    });
    const snapshot = {
      ...structuredClone(fixture),
      live: true,
      vision: {
        running: true,
        event_ready: true,
        ready: false,
        recording: false,
        error: null,
        provider: "CUDA",
        transport: "D3D11 to CUDA device tensor, CUDA graph",
        optimization_warning: null,
        viewport: [2560, 1440],
        fps: 143.5,
        capture_fps: 152.0,
        analyzed: { average: 141.25, worst: 98.4 },
        pipeline: { average: 6.5, worst: 11.04 },
        frames: 500,
        skipped: 4,
        throttled: 12,
        frame_age_ms: 8,
        last: {
          source_us: 1,
          received_us: 1,
          completed_us: 1,
          pipeline_ms: 6.9,
          preprocess_ms: 1.3,
          inference_ms: 5.6,
          skipped: 0,
          detections: [{ bounds: [1000, 500, 1200, 700], confidence: 0.96, class: 0 }],
        },
      },
    };
    // what native says for each state the test sets; a model is saved, so stop and start are free
    const unnamed: Blocker = {
      need: "scenario_unnamed",
      message:
        "name the scenario in target capture setup, or stop target capture to record mouse reports only",
      waits: false,
    };
    // a model warming up never holds a recording back
    snapshot.readiness.record = null;
    snapshot.readiness.screen_capture = null;
    Object.assign(window, {
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
              revision: snapshot.library,
              runs: structuredClone(snapshot.runs),
            };
          if (command === "initialize") return { roots: [], snapshot };
          if (command === "subscribe") channel = args.channel as typeof channel;
          if (command === "model_classes")
            return args.path === "C:/models/heads.onnx" ? ["body", "head"] : [];
          if (command === "screen_stop") {
            snapshot.vision.running = false;
            snapshot.vision.ready = false;
            channel?.onmessage(structuredClone(snapshot));
          }
          if (command === "plugin:event|listen") return 1;
        },
      },
      isTauri: true,
      setVisionState: (age: number, ready: boolean, eventReady = true) => {
        snapshot.vision.frame_age_ms = age;
        snapshot.vision.ready = ready;
        snapshot.vision.event_ready = eventReady;
        snapshot.readiness.record = eventReady ? null : unnamed;
        channel?.onmessage(structuredClone(snapshot));
      },
    });
  }, fixture);
  await page.goto("/");
  await openSection(page, "Target capture");
  const vision = page.getByRole("region", { name: "Target capture" });
  await expect(page.getByRole("button", { name: "Record", exact: true })).toBeEnabled();
  await expect(vision.getByText("Waiting for capture", { exact: true })).toBeVisible();
  await expect(vision.locator(".target-empty > svg")).toHaveCSS("width", "18px");
  await expect(vision.locator(".target-empty > svg")).toHaveCSS("position", "static");
  await vision.screenshot({ path: "test-results/vision-warmup.png" });
  // a rate without a current frame is unknown, not the last value
  await expect(vision.locator(".rate-readout tbody td").first()).toHaveText("-");
  await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
  await expect(
    page.locator(".status-diagnostics").getByText("D3D11 to CUDA device tensor, CUDA graph"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(vision.getByLabel("Frame rate limit")).toHaveValue("");
  await expect(vision.getByLabel("Frame rate limit")).toBeDisabled();
  await page.evaluate(() =>
    (
      window as unknown as {
        setVisionState: (age: number, ready: boolean) => void;
      }
    ).setVisionState(8, true),
  );
  await expect(page.getByRole("button", { name: "Record", exact: true })).toBeEnabled();
  await page.evaluate(() =>
    (
      window as unknown as {
        setVisionState: (age: number, ready: boolean, eventReady: boolean) => void;
      }
    ).setVisionState(8, true, false),
  );
  await expect(page.getByRole("button", { name: "Record", exact: true })).toBeDisabled();
  await expect(
    page.getByText("name the scenario in target capture setup", {
      exact: false,
    }),
  ).toBeVisible();
  await page.evaluate(() =>
    (
      window as unknown as {
        setVisionState: (age: number, ready: boolean) => void;
      }
    ).setVisionState(8, true),
  );
  // now, the ten second average and the 1% lows, side by side
  const rates = vision.locator(".rate-readout tbody tr");
  const cells = (row: number) => rates.nth(row).locator("th, td");
  await expect(cells(0)).toHaveText(["Analyzed", "143.5", "141.3", "98.4", "fps"]);
  await expect(cells(1)).toHaveText(["Captured", "152.0", "-", "-", "fps"]);
  await expect(cells(2)).toHaveText(["Pipeline", "6.9", "6.5", "11.0", "ms"]);
  await vision.screenshot({ path: "test-results/vision-rates.png" });
  const circle = vision.getByTestId("detected-target").locator("circle").first();
  await expect(circle).toHaveAttribute("cx", "1100");
  await expect(circle).toHaveAttribute("cy", "600");
  await expect(circle).toHaveAttribute("r", "100");
  await expect(vision.locator("svg[role=img]")).toHaveAttribute("viewBox", "0 0 2560 1440");
  const compactPreview = (await vision.locator(".target-stage").boundingBox())!;
  expect(compactPreview.width / compactPreview.height).toBeCloseTo(2560 / 1440, 1);
  expect(compactPreview.width).toBeLessThanOrEqual(360 * 1.25 + 1);
  await vision.getByRole("button", { name: "Expand target preview" }).click();
  const expandedPreview = (await vision.locator(".target-stage").boundingBox())!;
  expect(expandedPreview.width).toBeGreaterThan(compactPreview.width * 1.5);
  await vision.getByRole("button", { name: "Compact target preview" }).click();
  await expect(vision.getByText("Running", { exact: true })).toHaveAttribute(
    "data-variant",
    "success",
  );
  await vision.screenshot({
    path: "test-results/vision-live.png",
  });
  await page.setViewportSize({ width: 1000, height: 720 });
  if (!(await page.getByLabel("UI scale").isVisible())) await page.getByLabel("Settings").click();
  for (const scale of ["1", "1.25", "1.5"]) {
    await chooseOption(page.getByLabel("UI scale"), scale);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
      ),
    ).toBe(true);
    await expectControlsToFit(page);
  }
  await page.evaluate(() =>
    (
      window as unknown as {
        setVisionState: (age: number, ready: boolean) => void;
      }
    ).setVisionState(800, true),
  );
  await expect(vision.getByTestId("detected-target")).toHaveCount(0);
  await expect(vision.getByText("No recent screen frame")).toBeVisible();
  await expect(vision.locator(".target-empty > svg")).toHaveCSS("width", "18px");
  await vision.screenshot({ path: "test-results/vision-stale.png" });
  await vision.getByRole("button", { name: "Stop target capture" }).click();
  // off says nothing, there is no grey badge to tell apart from a green one
  await expect(vision.locator('[data-slot="badge"]')).toHaveCount(0);
  await expect(vision.getByRole("button", { name: "Start target capture" })).toBeVisible();
  await expect(vision.getByLabel("Frame rate limit")).toBeEnabled();
  await vision.getByText("Detector and hardware", { exact: true }).click();
  // the model's own class names stand in for the class index
  await chooseOption(vision.getByLabel("Target class"), "1");
  await expect(vision.getByLabel("Target class")).toHaveText("head");
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as SavedSettings).savedSettings().vision.target_class),
    )
    .toBe(1);
  await expect(vision.getByLabel("Sensitivity", { exact: true })).toBeVisible();
  const graph = vision.getByLabel(/CUDA graph/);
  await expect(graph).toBeChecked();
  await expect(graph).toBeEnabled();
  await graph.uncheck();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as SavedSettings).savedSettings().vision.cuda_graph),
    )
    .toBe(false);
  await graph.check();
  await vision.getByLabel("Frame rate limit").fill("1000");
  await vision.getByLabel("Frame rate limit").blur();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as SavedSettings).savedSettings().vision.fps),
    )
    .toBe(1000);
  await expect(vision.getByLabel("Frame rate limit")).not.toHaveAttribute("max");
  await vision.getByLabel("Frame rate limit").fill("0");
  await vision.getByLabel("Frame rate limit").blur();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as SavedSettings).savedSettings().vision.fps),
    )
    .toBe(0);
  await chooseOption(vision.getByLabel("Inference provider"), "TensorRT");
  await expect(vision.getByText(/first start builds the engine/i)).toBeVisible();
  const fp16 = vision.getByLabel("FP16 precision");
  await expect(fp16).toBeChecked();
  await expect(graph).toBeEnabled();
  await fp16.uncheck();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as SavedSettings).savedSettings().vision.tensorrt_fp16,
      ),
    )
    .toBe(false);
  await chooseOption(vision.getByLabel("Inference provider"), "DirectML");
  await expect(fp16).toHaveCount(0);
  await expect(graph).toBeDisabled();
});

test("a connected box and long labels fit at supported display scales", async ({ page }) => {
  await page.addInitScript(
    (fixture) => {
      const snapshot = structuredClone(fixture);
      snapshot.live = true;
      Object.assign(snapshot.box, {
        link: "connected",
        name: "Medius mouse control port",
        has_mouse: true,
        medius: {
          ...fixture.box.medius,
          clone: "Wireless gaming mouse composite interface",
          render: {
            ...snapshot.box.medius!.render,
            mode: "despiked",
            full: true,
            ready: true,
            effective: "rendered",
          },
        },
      });
      snapshot.runs[0].name =
        "native-baseline-with-a-long-session-and-device-description-20261002.pcap";
      Object.assign(window, {
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
            if (command === "initialize") return { roots: ["USBPcap1"], snapshot };
            if (command === "run_plot") return fixture.plot;
            if (command === "plugin:event|listen") return 1;
          },
        },
        isTauri: true,
      });
    },
    { ...fixture, box: connectedBox },
  );
  await page.goto("/");
  await openSection(page, "Box");
  await openSection(page, "Medius");
  const boxPanel = page.getByRole("region", { name: "Box", exact: true });
  const mediusPanel = page.getByRole("region", { name: "Medius", exact: true });
  // connected shows in the section header, in green
  await expect(boxPanel.getByText("Medius", { exact: true })).toHaveAttribute(
    "data-variant",
    "success",
  );
  // the readback says what the box confirms, apart from the settings below it
  await expect(mediusPanel.getByText("Rendered", { exact: true })).toHaveAttribute(
    "data-variant",
    "success",
  );
  await expect(mediusPanel.getByText("Ready", { exact: true })).toHaveAttribute(
    "data-variant",
    "success",
  );
  await expect(boxPanel.getByLabel("Record physical input")).toBeVisible();
  await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
  const surface = await page.locator(".status-diagnostics").evaluate((el) => {
    const css = getComputedStyle(el);
    return { border: css.borderTopColor, blur: css.backdropFilter };
  });
  expect(surface.border).not.toBe("rgb(232, 232, 232)");
  expect(surface.blur).toContain("blur");
  await page.keyboard.press("Escape");
  const timing = page.getByText("Timing and compatibility", { exact: true });
  await timing.click();
  await expect(page.getByLabel("Forced wire rate")).toBeVisible();
  for (const width of [1600, 1440, 1000]) {
    await page.setViewportSize({ width, height: 900 });
    if (!(await page.getByLabel("UI scale").isVisible())) await page.getByLabel("Settings").click();
    for (const scale of ["1", "1.25", "1.5"]) {
      await chooseOption(page.getByLabel("UI scale"), scale);
      await expectControlsToFit(page);
      const box = (await boxPanel.boundingBox())!;
      const medius = (await mediusPanel.boundingBox())!;
      const setup = (await page.locator(".setup-panel").boundingBox())!;
      // the two sections share a row when there's room and fill the card's width either way
      const across = Math.abs(box.y - medius.y) < 1 ? box.width + medius.width + 1 : box.width;
      expect(setup.width - across).toBeGreaterThanOrEqual(0);
      expect(setup.width - across).toBeLessThanOrEqual(2);
      if (across === box.width) expect(Math.abs(medius.width - box.width)).toBeLessThan(1);
      // with nothing reported beside them, the settings take the section's whole width
      for (const panel of [boxPanel, mediusPanel])
        expect(
          await panel.evaluate((section) => {
            const body = section.querySelector<HTMLElement>(".setup-section-body")!;
            const main = section.querySelector<HTMLElement>(".setup-section-main")!;
            const css = getComputedStyle(body);
            return (
              body.clientWidth -
              parseFloat(css.paddingLeft) -
              parseFloat(css.paddingRight) -
              main.offsetWidth
            );
          }),
        ).toBeLessThan(2);
    }
  }
  await page.screenshot({
    path: "test-results/capture-connected-narrow.png",
    fullPage: true,
  });
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Compare" }).click();
  const candidates = page.getByRole("group", { name: "Candidate recordings" });
  await openRecordingPicker(page, "Candidate");
  const popup = await page.locator(".recording-popover").boundingBox();
  expect(popup!.x).toBeGreaterThanOrEqual(0);
  expect(popup!.x + popup!.width).toBeLessThanOrEqual(1000);
  expect(popup!.y + popup!.height).toBeLessThanOrEqual(900);
  await candidates.getByRole("checkbox").first().check();
  await page.keyboard.press("Escape");
  await expectControlsToFit(page);
  await page.screenshot({
    path: "test-results/compare-long-name.png",
    fullPage: true,
  });
});

test("the mouse endpoint list names each endpoint and marks the one a box clones", async ({
  page,
}) => {
  await page.addInitScript(
    (fixture) => {
      const snapshot = structuredClone(fixture);
      snapshot.live = true;
      snapshot.devices = [
        {
          ...snapshot.devices[0],
          id: "1:30:129",
          address: 30,
          kind: "mouse",
          vid: 14139,
          pid: 4569,
          name: "Wireless mouse 8k dongle-L",
          box_name: "Medius - medius",
        },
        {
          ...snapshot.devices[0],
          id: "1:12:129",
          address: 12,
          kind: "keyboard",
          vid: 14139,
          pid: 4544,
          name: "ATK 68 RX",
        },
        {
          ...snapshot.devices[0],
          id: "1:15:129",
          address: 15,
          kind: "other",
          vid: 4810,
          pid: 21513,
        },
      ];
      Object.assign(window, {
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
            if (command === "initialize") return { roots: ["USBPcap1"], snapshot };
            if (command === "run_plot") return fixture.plot;
            if (command === "plugin:event|listen") return 1;
          },
        },
        isTauri: true,
      });
    },
    { ...fixture, box: connectedBox },
  );
  await page.goto("/");
  const picker = page.getByLabel("Mouse endpoint");
  await picker.click();
  // only the mice at first, the rest once asked for
  const options = page.getByRole("option");
  await expect(options).toHaveCount(1);
  // an open menu leaves the page's scrollbar where it was
  const body = page.locator("body[data-scroll-locked]");
  await expect(body).toHaveCSS("overflow", "visible");
  await expect(body).toHaveCSS("margin-right", "0px");
  await page.keyboard.press("Escape");
  await page.getByLabel("Other devices").click();
  await picker.click();
  // one row per endpoint, the hint to move the mouse isn't one
  await expect(options).toHaveCount(3);
  await expect(page.locator(".dropdown-content")).not.toContainText("Move your mouse");
  await page.screenshot({ path: ".review/shots/endpoints.png" });
  await expect(options.nth(0).getByRole("img", { name: "mouse" })).toBeVisible();
  await expect(options.nth(1).getByRole("img", { name: "keyboard" })).toBeVisible();
  await expect(options.nth(2).getByRole("img", { name: "other device" })).toBeVisible();
  // the words a box's own name can be confused with are gone from the row
  await expect(page.locator(".dropdown-content")).not.toContainText("keyboard");
  // what Windows calls each device, with the ids a hover away
  await expect(options.nth(0)).toContainText("Wireless mouse 8k dongle-L");
  await expect(options.nth(0).locator("[title]").first()).toHaveAttribute("title", "373b:11d9");
  await expect(options.nth(1)).toContainText("ATK 68 RX");
  // a device Windows did not name keeps its ids, and the box's clone says which box it is
  await expect(options.nth(2)).toContainText("12ca:5409");
  // the box goes by its kind and its own name
  await expect(options.nth(0)).toContainText("Medius - medius");
  await expect(options.nth(1)).not.toContainText("Medius");
});

test("linear experiment learns a Medius mouse hotkey and locks recording settings", async ({
  page,
}) => {
  await page.addInitScript(
    (fixture) => {
      let channel: { onmessage: (snapshot: unknown) => void } | null = null;
      const commands: { command: string; args: unknown }[] = [];
      const snapshot = {
        ...structuredClone(fixture),
        live: true,
        box: {
          ...structuredClone(fixture.box),
          link: "connected",
          has_mouse: true,
          pending_commands: 0,
        },
        vision: {
          running: true,
          ready: true,
          event_ready: false,
          frames: 0,
          skipped: 0,
          recording: false,
          error: null,
          last: null,
        },
        experiment: {
          ready: true,
          armed: false,
          active: false,
          learning: false,
          hotkey_mask: 8,
          held_mask: 0,
          binding_seq: 0,
          phase: "Disarmed",
          commands: 0,
          counts: [0, 0],
          target_counts: null,
          recording: false,
          error: null,
        },
      };
      // what native says at each step: the scenario has no name until the test names it
      snapshot.readiness.arm = {
        need: "scenario_unnamed",
        message: "name the scenario in target capture setup to arm",
        waits: false,
      };
      snapshot.readiness.record = {
        need: "scenario_unnamed",
        message:
          "name the scenario in target capture setup, or stop target capture to record mouse reports only",
        waits: false,
      };
      snapshot.readiness.aim_button = null;
      snapshot.readiness.screen_capture = null;
      Object.assign(window, {
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: () => 1,
          unregisterCallback: () => {},
          invoke: async (command: string, args: Record<string, unknown> = {}) => {
            commands.push({ command, args });
            if (command === "runs")
              return {
                revision: snapshot.library,
                runs: structuredClone(snapshot.runs),
              };
            if (command === "initialize") return { roots: [], snapshot };
            if (command === "subscribe") channel = args.channel as typeof channel;
            if (
              command === "update_settings" &&
              (window as unknown as { refuseSettings?: boolean }).refuseSettings
            )
              throw new Error("the deadzone is between 0 and 100 mouse counts");
            if (command === "experiment_learn") snapshot.experiment.learning = !args.cancel;
            if (command === "experiment_arm") {
              snapshot.experiment.armed = true;
              snapshot.experiment.phase = "Hold the hotkey";
            }
            if (command === "experiment_disarm") {
              snapshot.experiment.armed = false;
              snapshot.experiment.phase = "Disarmed";
              // the recording goes on, so arming again waits for it
              snapshot.readiness.arm = {
                need: "recording",
                message: "arm the aim before starting a recording",
                waits: false,
              };
            }
            if (command === "recording_toggle") snapshot.recording = "linear-trial";
            if (command === "plugin:event|listen") return 1;
            if (
              [
                "experiment_learn",
                "experiment_arm",
                "experiment_disarm",
                "recording_toggle",
              ].includes(command)
            )
              channel?.onmessage(structuredClone(snapshot));
          },
        },
        isTauri: true,
        // native holds the gate while a stop is on its way, so Finish waits
        recordingBusy: (busy: boolean) => {
          snapshot.readiness.record = busy
            ? {
                need: "busy",
                message: "the last action is still finishing, wait for it",
                waits: true,
              }
            : null;
          channel?.onmessage(structuredClone(snapshot));
        },
        allowExperiment: () => {
          snapshot.vision.event_ready = true;
          snapshot.readiness.arm = null;
          snapshot.readiness.record = null;
          channel?.onmessage(structuredClone(snapshot));
        },
        mediusButton: (mask: number) => {
          snapshot.experiment.held_mask = mask;
          if (snapshot.experiment.learning && mask) {
            snapshot.experiment.hotkey_mask = mask;
            snapshot.experiment.binding_seq++;
            snapshot.experiment.learning = false;
          }
          channel?.onmessage(structuredClone(snapshot));
        },
        experimentCommands: commands,
      });
    },
    { ...fixture, box: connectedBox },
  );
  await page.goto("/");
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Experiment", exact: true }).click();
  const panel = page.getByRole("region", { name: "Linear aim experiment" });
  const arm = panel.getByRole("button", { name: "Arm", exact: true });
  await expect(arm).toBeDisabled();
  await expect(
    panel.getByText("name the scenario in target capture setup to arm", {
      exact: true,
    }),
  ).toBeVisible();
  await panel
    .locator(".experiment-footer")
    .screenshot({ path: "test-results/experiment-arm-blocked.png" });
  await panel.getByRole("button", { name: "Capture setup", exact: true }).click();
  for (const name of ["Target capture", "Medius"])
    await expect(page.getByRole("button", { name, exact: true })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  await page.getByRole("tab", { name: "Experiment", exact: true }).click();
  await page.evaluate(() =>
    (window as unknown as { allowExperiment: () => void }).allowExperiment(),
  );
  await expect(arm).toBeEnabled();
  const slider = panel.getByRole("slider", { name: "Linear speed" });
  const speedInput = panel.getByRole("spinbutton", {
    name: "Speed in counts per second",
  });
  await speedInput.fill("2300");
  await speedInput.press("Enter");
  await expect(slider).toHaveAttribute("aria-valuenow", "2300");
  await speedInput.fill("2000");
  await speedInput.press("Enter");
  await slider.focus();
  await slider.press("ArrowRight");
  await expect(slider).toHaveAttribute("aria-valuenow", "2100");
  const deadzone = panel.getByRole("slider", { name: "Linear deadzone" });
  await expect(deadzone).toHaveAttribute("aria-valuenow", "12");
  await deadzone.focus();
  await deadzone.press("ArrowRight");
  await expect(deadzone).toHaveAttribute("aria-valuenow", "13");
  await panel.locator(".aim-binding").screenshot({ path: "test-results/aim-binding-assigned.png" });
  await panel.getByRole("button", { name: /^Aim button:/ }).click();
  await expect(panel.getByRole("button", { name: "Cancel aim button binding" })).toBeVisible();
  // the listening picker keeps its ring colored border
  await expect(panel.getByRole("button", { name: "Cancel aim button binding" })).toHaveCSS(
    "border-top-color",
    "rgb(45, 163, 232)",
  );
  await page.keyboard.press("F8");
  await expect(panel.getByRole("button", { name: "Cancel aim button binding" })).toBeVisible();
  await page.evaluate(() =>
    (window as unknown as { mediusButton: (mask: number) => void }).mediusButton(16),
  );
  await expect(panel.getByText("Side 2", { exact: true })).toBeVisible();
  await page.evaluate(() =>
    (window as unknown as { mediusButton: (mask: number) => void }).mediusButton(0),
  );
  // arming uses the saved settings, so a change native refuses stops it
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as SavedSettings).savedSettings().aim.deadzone_counts),
    )
    .toBe(13);
  await page.evaluate(() => Object.assign(window, { refuseSettings: true }));
  await deadzone.press("ArrowRight");
  await arm.click();
  await expect(page.locator("[data-sonner-toast]")).toContainText("couldn't save the settings");
  // an error stays until it is closed
  await page.getByRole("button", { name: "Close toast" }).click();
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
  await expect(deadzone).toHaveAttribute("aria-valuenow", "13");
  expect(
    await page.evaluate(() =>
      (window as unknown as { experimentCommands: { command: string }[] }).experimentCommands.some(
        (c) => c.command === "experiment_arm",
      ),
    ),
  ).toBe(false);
  await page.evaluate(() => Object.assign(window, { refuseSettings: false }));
  await arm.click();
  await expect(slider).toBeDisabled();
  await expect(panel.getByText("disarm first if you want to change the settings")).toBeVisible();
  await expect(deadzone).toBeDisabled();
  // the armed values stay readable but cannot change: the binding reads as text, with nothing to press
  await expect(speedInput).toHaveAttribute("readonly", "");
  await expect(panel.getByRole("button", { name: /^Aim button:/ })).toHaveCount(0);
  await expect(panel.locator(".aim-binding")).toHaveText("Side 2");
  await expect(
    page.locator(".global-session-status").getByText("Experiment armed", { exact: true }),
  ).toHaveAttribute("data-variant", "warning");
  // arming takes no settings of its own, it uses the saved ones
  const sent = await page.evaluate(
    () =>
      (
        window as unknown as {
          experimentCommands: { command: string; args: unknown }[];
        }
      ).experimentCommands,
  );
  expect(sent.find((c) => c.command === "experiment_arm")?.args).toEqual({});
  expect(
    await page.evaluate(() => (window as unknown as SavedSettings).savedSettings().aim),
  ).toEqual({ speed: 2100, deadzone_counts: 13, button: 8 });
  if (!(await page.getByLabel("UI scale").isVisible())) await page.getByLabel("Settings").click();
  for (const width of [1600, 1000]) {
    await page.setViewportSize({ width, height: 900 });
    if (!(await page.getByLabel("UI scale").isVisible())) await page.getByLabel("Settings").click();
    for (const scale of ["1", "1.25", "1.5"]) {
      await chooseOption(page.getByLabel("UI scale"), scale);
      await expectControlsToFit(page);
    }
  }
  await page.setViewportSize({ width: 1600, height: 1000 });
  await chooseOption(page.getByLabel("UI scale"), "1.25");
  await panel.screenshot({ path: "test-results/linear-experiment.png" });
  await panel.getByRole("button", { name: "Record" }).click();
  const finish = panel.getByRole("button", { name: "Finish recording" });
  await expect(finish).toBeEnabled();
  const busy = (value: boolean) =>
    page.evaluate(
      (value) =>
        (window as unknown as { recordingBusy: (busy: boolean) => void }).recordingBusy(value),
      value,
    );
  await busy(true);
  await expect(finish).toBeDisabled();
  await busy(false);
  await expect(finish).toBeEnabled();
  await panel.getByRole("button", { name: "Disarm", exact: true }).click();
  await expect(arm).toBeDisabled();
  await expect(slider).toBeDisabled();
});

test("recordings table keeps padded cells and accessible actions at each UI scale", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "Runs", exact: false }).click();
  for (const width of [1172, 1000, 1600]) {
    await page.setViewportSize({ width, height: 834 });
    for (const scale of ["1", "1.25", "1.5"]) {
      await page.getByLabel("Settings").click();
      await chooseOption(page.getByLabel("UI scale"), scale);
      await page.keyboard.press("Escape");
      await page.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      const layout = await page.locator(".run-table").evaluate((card) => {
        const scroll = card.querySelector<HTMLElement>('[data-slot="table-container"]')!;
        scroll.scrollLeft = 0;
        const cell = card.querySelector("tbody td")!;
        const name = cell.querySelector(".run-name")!;
        const actions = card.querySelector(".run-actions")!;
        const cardRect = card.getBoundingClientRect();
        const nameRect = name.getBoundingClientRect();
        const actionsRect = actions.getBoundingClientRect();
        return {
          padding: parseFloat(getComputedStyle(cell).paddingLeft),
          nameInset: nameRect.left - cardRect.left,
          actionsRight: actionsRect.right,
          cardRight: cardRect.right,
          pageFits:
            document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
          scrollFits: scroll.clientWidth <= card.clientWidth,
          scrolls: scroll.scrollWidth > scroll.clientWidth,
        };
      });
      expect(layout.padding).toBeGreaterThanOrEqual(12);
      expect(layout.nameInset).toBeGreaterThanOrEqual(12);
      expect(layout.actionsRight).toBeLessThanOrEqual(layout.cardRight);
      expect(layout.pageFits).toBe(true);
      expect(layout.scrollFits).toBe(true);
      expect(layout.scrolls).toBe(false);
      await page.getByRole("button", { name: "Inspect", exact: true }).focus();
      await expect(page.getByRole("button", { name: "Inspect", exact: true })).toBeFocused();
    }
  }
  await page.setViewportSize({ width: 1172, height: 834 });
  await page.getByLabel("Settings").click();
  await chooseOption(page.getByLabel("UI scale"), "1.25");
  await page.keyboard.press("Escape");
  await page.screenshot({ path: "test-results/recordings-1172.png" });
});

test("expanded capture controls keep spacing and custom menus work with keyboard at every scale", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1172, height: 834 });
  await page.goto("/");
  await openSection(page, "Target capture");
  await page.getByText("Response delay", { exact: true }).click();
  await expect(page.locator("select:visible")).toHaveCount(0);
  await expect(page.getByText("USB timestamps are preserved.", { exact: false })).toHaveCount(0);
  for (const scale of ["1", "1.25", "1.5"]) {
    await page.getByLabel("Settings").click();
    await chooseOption(page.getByLabel("UI scale"), scale);
    await page.keyboard.press("Escape");
    const recording = page.getByLabel("Calibration recording");
    await recording.click();
    const menu = page.getByRole("listbox");
    const bounds = (await menu.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(1172);
    expect(bounds.y).toBeGreaterThanOrEqual(0);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(834);
    await page.keyboard.press("Home");
    await expect(menu.getByRole("option").first()).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(recording).toContainText(fixture.runs[0].name);
  }
  await page
    .getByRole("region", { name: "Target capture" })
    .screenshot({ path: "test-results/target-capture-expanded.png" });
});

test("release UI hides an absent optional driver without blocking recording", async ({ page }) => {
  await page.addInitScript((fixture) => {
    const snapshot = {
      ...structuredClone(fixture),
      live: true,
      dev_mode: false,
      observers: {
        raw_ready: true,
        kernel_ready: false,
        kernel_seen: false,
        recording: false,
        reports: 0,
        batches: 0,
        max_batch: 0,
        queue_losses: 0,
        kernel_losses: 0,
        os_loss_count_known: false,
        error: null,
        kernel_status: "Optional observer is not installed",
        devices: {},
        kernel_devices: {},
      },
    };
    // native lets it record: the raw observer is all a recording needs
    snapshot.readiness.record = null;
    let channel: { onmessage: (snapshot: unknown) => void } | null = null;
    Object.assign(window, {
      setOptionalDriver: (development: boolean, loaded: boolean) => {
        snapshot.dev_mode = development;
        snapshot.observers.kernel_ready = loaded;
        snapshot.observers.kernel_seen ||= loaded;
        channel?.onmessage(structuredClone(snapshot));
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
              revision: snapshot.library,
              runs: structuredClone(snapshot.runs),
            };
          if (command === "initialize") return { roots: ["USBPcap1"], snapshot };
          if (command === "subscribe") channel = args.channel as typeof channel;
          if (command === "run_plot") return fixture.plot;
          if (command === "plugin:event|listen") return 1;
        },
      },
      isTauri: true,
    });
  }, fixture);
  await page.goto("/");
  await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
  const observers = page.locator(".status-diagnostics").getByRole("region", { name: "Observers" });
  const driver = observers.getByText("Mouse-class observer", { exact: true });
  await expect(driver).toHaveCount(0);
  await expect(observers).toContainText("Raw InputReady");
  await expect(page.getByRole("button", { name: "Record", exact: true })).toBeEnabled();
  await page.evaluate(() =>
    (
      window as unknown as {
        setOptionalDriver: (dev: boolean, loaded: boolean) => void;
      }
    ).setOptionalDriver(true, false),
  );
  await expect(observers).toContainText("Mouse-class observerOptional observer is not installed");
  await page.evaluate(() =>
    (
      window as unknown as {
        setOptionalDriver: (dev: boolean, loaded: boolean) => void;
      }
    ).setOptionalDriver(false, true),
  );
  await expect(observers).toContainText("Mouse-class observerReady");
  await page.evaluate(() =>
    (
      window as unknown as {
        setOptionalDriver: (dev: boolean, loaded: boolean) => void;
      }
    ).setOptionalDriver(false, false),
  );
  await expect(driver).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Record", exact: true })).toBeEnabled();
});

test("native settings restore the aim settings and select the saved mouse after enumeration", async ({
  page,
}) => {
  await page.addInitScript(
    ([fixture, defaults]) => {
      let config = JSON.parse(localStorage.getItem("test-config-file") ?? "null") ?? {
        ...defaults,
        capture: {
          root: "USBPcap2",
          mouse: { vid: 1234, pid: 5678, bus: 1, endpoint: 129 },
        },
        aim: { speed: 3500, deadzone_counts: 19, button: 16 },
        display: {
          zoom: 1.1,
          sections: { targets: false, box: false, box_settings: false },
          volume: 0.6,
          muted: false,
          theme: "system",
        },
      };
      const device = {
        ...fixture.devices[0],
        id: "1:47:129",
        address: 47,
        vid: 1234,
        pid: 5678,
      };
      const snapshot = {
        ...structuredClone(fixture),
        live: true,
        selected: null as string | null,
        devices: [] as typeof fixture.devices,
        // the aim button is the box's, as the aim controller reports it
        experiment: {
          ready: false,
          armed: false,
          active: false,
          learning: false,
          hotkey_mask: 16,
          held_mask: 0,
          binding_seq: 0,
          phase: "",
          commands: 0,
          counts: [0, 0],
          target_counts: null,
          recording: false,
          error: null,
        },
      };
      const requests: { command: string; args: Record<string, unknown> }[] = [];
      let channel: { onmessage: (value: unknown) => void } | null = null;
      Object.assign(window, {
        configRequests: requests,
        enumerateMouse: () => {
          snapshot.devices = [device];
          channel?.onmessage(structuredClone(snapshot));
        },
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: () => 1,
          unregisterCallback: () => {},
          invoke: async (command: string, args: Record<string, unknown> = {}) => {
            requests.push({ command, args });
            if (command === "runs")
              return {
                revision: snapshot.library,
                runs: structuredClone(snapshot.runs),
              };
            if (command === "initialize")
              return {
                roots: ["USBPcap1", "USBPcap2"],
                root: config.capture.root,
                snapshot,
              };
            if (command === "settings") return config;
            if (command === "subscribe") channel = args.channel as typeof channel;
            if (command === "select_device") {
              snapshot.selected = String(args.id);
              channel?.onmessage(structuredClone(snapshot));
              return null;
            }
            if (command === "update_settings") {
              if ((window as unknown as { failConfigSave?: boolean }).failConfigSave)
                throw new Error("aim speed is between 100 and 20,000 counts/s");
              const patch = args.patch as Record<string, unknown>;
              config = {
                ...config,
                ...patch,
                aim: { ...config.aim, ...patch.aim! },
              };
              localStorage.setItem("test-config-file", JSON.stringify(config));
              return config;
            }
            if (command === "run_plot") return fixture.plot;
            if (command === "plugin:event|listen") return 1;
          },
        },
        isTauri: true,
      });
    },
    [fixture, previewSettings] as const,
  );
  await page.goto("/");
  await expect(page.getByLabel("USB root")).toContainText("USBPcap2");
  await page.getByRole("button", { name: "Box", exact: true }).click();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { configRequests: { command: string }[] }).configRequests.filter(
          (r) => r.command === "select_device",
        ).length,
    ),
  ).toBe(0);
  await page.evaluate(() => (window as unknown as { enumerateMouse: () => void }).enumerateMouse());
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              configRequests: {
                command: string;
                args: Record<string, unknown>;
              }[];
            }
          ).configRequests.find((r) => r.command === "select_device")?.args,
      ),
    )
    .toEqual({ id: "1:47:129" });
  await page.getByRole("tab", { name: "Experiment", exact: true }).click();
  const speed = page.getByRole("spinbutton", {
    name: "Speed in counts per second",
  });
  await expect(speed).toHaveValue("3500");
  await expect(page.getByRole("spinbutton", { name: "Deadzone in counts" })).toHaveValue("19");
  await expect(page.getByText("Aim button", { exact: true })).toBeVisible();
  await expect(page.getByText("Recording button", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Side 2", { exact: true })).toBeVisible();
  const speedTrack = page.locator('.experiment-panel [data-slot="slider"]').first();
  const track = await speedTrack.boundingBox();
  for (const fraction of [0.1, 0.9, 0.5]) {
    await page.mouse.move(track!.x + track!.width * fraction, track!.y + track!.height / 2);
    await page.mouse.down();
    await page.mouse.move(track!.x + track!.width * (1 - fraction), track!.y + track!.height / 2);
    await page.mouse.up();
    await page.waitForTimeout(350);
  }
  const savedSpeeds = await page.evaluate(() =>
    (
      window as unknown as {
        configRequests: {
          command: string;
          args: { patch?: { aim?: { speed: number } } };
        }[];
      }
    ).configRequests
      .filter((r) => r.command === "update_settings" && r.args.patch?.aim)
      .map((r) => r.args.patch!.aim!.speed),
  );
  expect(savedSpeeds.length).toBeGreaterThan(0);
  expect(
    savedSpeeds.every((value) => Number.isFinite(value) && value >= 100 && value <= 20000),
  ).toBe(true);
  await speed.fill("4100");
  await speed.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() => JSON.parse(localStorage.getItem("test-config-file") ?? "{}").aim?.speed),
    )
    .toBe(4100);
  await page.reload();
  // which setup sections are open is remembered like any other preference
  await expect(page.getByRole("button", { name: "Box", exact: true })).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await page.getByRole("tab", { name: "Experiment", exact: true }).click();
  await expect(speed).toHaveValue("4100");
  await page.evaluate(() => Object.assign(window, { failConfigSave: true }));
  await speed.fill("4500");
  await speed.press("Enter");
  await expect(page.locator("[data-sonner-toast]")).toContainText("couldn't save the settings");
  // a refused change shows the saved value again
  await expect(speed).toHaveValue("4100");
  expect(
    await page.evaluate(() => JSON.parse(localStorage.getItem("test-config-file")!).aim.speed),
  ).toBe(4100);
  await expect(page.getByRole("spinbutton", { name: "Deadzone in counts" })).toHaveValue("19");
});

test("the recordings table shows how each recording ended and why", async ({ page }) => {
  await page.addInitScript((fixture) => {
    const snapshot = structuredClone(fixture);
    const run = snapshot.runs[0];
    snapshot.runs = [
      {
        ...run,
        id: "done",
        name: "done.pcap",
        status: { kind: "complete", reason: null, message: null },
      },
      {
        ...run,
        id: "lost",
        name: "lost.pcap",
        panels: [],
        missing: {
          texture19: "it failed: selected device disconnected",
        },
        status: {
          kind: "failed",
          reason: "device_lost",
          message: "selected device disconnected",
        },
      },
      {
        ...run,
        id: "drift",
        name: "drift.pcap",
        status: {
          kind: "degraded",
          reason: null,
          message:
            "the box's clock could not be lined up with the wire's for this recording, something recorded beside the usb capture failed",
        },
      },
      {
        ...run,
        id: "cut",
        name: "cut.pcap",
        status: {
          kind: "interrupted",
          reason: null,
          message: "the app closed while it was recording",
        },
      },
    ];
    let channel: { onmessage: (v: unknown) => void } | null = null;
    Object.assign(window, {
      isTauri: true,
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => undefined,
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          if (command === "runs")
            return {
              revision: snapshot.library,
              runs: structuredClone(snapshot.runs),
            };
          if (command === "initialize") return { roots: [], snapshot };
          if (command === "subscribe") {
            channel = args.channel as typeof channel;
            setInterval(() => channel?.onmessage(structuredClone(snapshot)), 100);
            return;
          }
          if (command === "run_plot") return fixture.plot;
          if (command === "plugin:event|listen") return 1;
        },
      },
    });
  }, fixture);
  await page.goto("/");
  await page.getByRole("tab", { name: "Runs", exact: false }).click();
  const table = page.getByRole("table", { name: "Recordings" });
  const row = (name: string) =>
    table.getByRole("row").filter({ has: page.getByText(name, { exact: true }) });
  await expect(row("done.pcap")).toContainText("Complete");
  await expect(row("lost.pcap")).toContainText("Failed");
  await expect(row("lost.pcap")).toContainText("Evidence only");
  await expect(row("lost.pcap")).toContainText("selected device disconnected");
  await expect(row("drift.pcap")).toContainText("Degraded");
  // a long reason wraps instead of being cut off
  const reason = row("drift.pcap").locator(".run-status-reason");
  await expect(reason).toContainText("usb capture failed");
  expect(await reason.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await expect(row("cut.pcap")).toContainText("Interrupted");
  await expect(row("cut.pcap")).toContainText("the app closed while it was recording");
  // the outcome's color: green when complete, red when failed, amber in between
  for (const [name, tone] of [
    ["done.pcap", "success"],
    ["lost.pcap", "danger"],
    ["drift.pcap", "warning"],
    ["cut.pcap", "warning"],
  ])
    await expect(row(name).locator('[data-slot="badge"]')).toHaveAttribute("data-variant", tone);
  await table.screenshot({ path: "test-results/runs-status.png" });
});

test("a problem that does not stop capture is a notice, not a capture failure", async ({
  page,
}) => {
  await page.addInitScript((fixture) => {
    const snapshot = structuredClone(fixture);
    snapshot.live = true;
    snapshot.health.error = null;
    (snapshot.health as { notice?: string }).notice =
      "the selected device was unplugged, select it again";
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
              revision: snapshot.library,
              runs: structuredClone(snapshot.runs),
            };
          if (command === "initialize") return { roots: [], snapshot };
          if (command === "subscribe") {
            const channel = args.channel as { onmessage: (v: unknown) => void };
            setInterval(() => channel.onmessage(structuredClone(snapshot)), 50);
          }
          if (command === "run_plot") return fixture.plot;
        },
      },
    });
  }, fixture);
  await page.goto("/");
  await expect(page.getByRole("status").filter({ hasText: "unplugged" })).toHaveText(
    "the selected device was unplugged, select it again",
  );
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.locator(".global-session-status")).toHaveText("Monitoring");
  await page
    .getByRole("status")
    .filter({ hasText: "unplugged" })
    .screenshot({ path: "test-results/notice.png" });
});
