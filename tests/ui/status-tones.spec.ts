import { connectedBox, snapshot as fixture } from "../../ui/fixture";
import { expect, test } from "./settings-mock";
import type { Page } from "@playwright/test";

const vision = {
  running: true,
  ready: true,
  event_ready: true,
  recording: false,
  error: null,
  provider: "CUDA",
  viewport: [2560, 1440],
  fps: 140,
  capture_fps: 150,
  frames: 10,
  skipped: 0,
  throttled: 0,
  frame_age_ms: 8,
  last: {
    source_us: 1,
    received_us: 1,
    completed_us: 1,
    pipeline_ms: 6,
    preprocess_ms: 1,
    inference_ms: 5,
    skipped: 0,
    detections: [],
  },
};
const experiment = {
  ready: true,
  armed: true,
  active: false,
  learning: false,
  hotkey_mask: 16,
  held_mask: 0,
  binding_seq: 0,
  phase: "Hold the hotkey",
  commands: 0,
  counts: [0, 0],
  target_counts: null,
  recording: false,
  error: null,
};

async function open(page: Page, patch: Record<string, unknown>) {
  await page.addInitScript(
    ({ fixture, patch }) => {
      const snapshot = { ...structuredClone(fixture), live: true, ...patch };
      const calls: { command: string; args: unknown }[] = [];
      Object.assign(window, {
        isTauri: true,
        calls,
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
            if (command === "initialize") return { roots: [], snapshot };
            if (command === "recordings_folders")
              return {
                running: "C:\\Users\\me\\Documents\\Motion Bench",
                fallback: "C:\\Users\\me\\Documents\\Motion Bench",
              };
            if (command === "plugin:dialog|open") return "D:\\Recordings";
            if (command === "check_update") return (patch as { update?: string }).update ?? null;
            // the real one ends the app
            if (command === "install_update") return new Promise(() => {});
            if (command === "run_plot") return fixture.plot;
            if (command === "plugin:event|listen") return 1;
          },
        },
      });
    },
    { fixture, patch },
  );
  await page.goto("/");
}

// the text color of each tone as the browser computes it, so a wrong class mapping fails too
const textColor = {
  success: "rgb(76, 195, 138)",
  warning: "rgb(224, 169, 74)",
  danger: "rgb(239, 111, 108)",
  muted: "rgb(161, 167, 176)",
};

// each status badge says its state in words and in a color; a swapped color misleads as much as
// a wrong word, so every state is pinned here
const cases: {
  name: string;
  patch: Record<string, unknown>;
  where: (page: Page) => Promise<void>;
  scope: (page: Page) => ReturnType<Page["locator"]>;
  label: string;
  tone: keyof typeof textColor;
}[] = [
  {
    name: "monitoring in the header",
    patch: {},
    where: async () => {},
    scope: (page) => page.locator(".global-session-status"),
    label: "Monitoring",
    tone: "success",
  },
  {
    name: "an armed aim in the header",
    patch: { experiment },
    where: async () => {},
    scope: (page) => page.locator(".global-session-status"),
    label: "Experiment armed",
    tone: "warning",
  },
  {
    name: "a live view",
    patch: {},
    where: async () => {},
    scope: (page) => page.locator(".range-toolbar"),
    label: "Live",
    tone: "success",
  },
  {
    name: "recording in the header",
    patch: { recording: "run-1", recording_elapsed_ms: 1000 },
    where: async () => {},
    scope: (page) => page.locator(".global-session-status"),
    // with how long it has run, as beside the record button
    label: "Recording 00:01",
    tone: "danger",
  },
  {
    name: "a broken app in the header",
    patch: {
      readiness: {
        ...fixture.readiness,
        record: {
          need: "broken",
          message: "something inside the app broke, so restart it",
          waits: false,
        },
      },
    },
    where: async () => {},
    scope: (page) => page.locator(".global-session-status"),
    label: "Restart needed",
    tone: "danger",
  },
  {
    name: "a capture error in the header",
    patch: {
      health: { ...fixture.health, error: "the capture driver stopped" },
    },
    where: async () => {},
    scope: (page) => page.locator(".global-session-status"),
    label: "Capture needs attention",
    tone: "danger",
  },
  {
    name: "a connected Medius",
    patch: {
      box: {
        ...connectedBox,
        link: "connected",
        has_mouse: true,
        medius: {
          ...connectedBox.medius,
          render: {
            mode: "despiked",
            full: false,
            ready: true,
            effective: "rendered",
          },
        },
      },
    },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Box", exact: true }),
    label: "Medius",
    tone: "success",
  },
  {
    name: "a connected box named by its family and port",
    patch: {
      box: {
        ...connectedBox,
        link: "connected",
        has_mouse: true,
        port: "COM5",
        medius: {
          ...connectedBox.medius,
          render: {
            mode: "despiked",
            full: false,
            ready: true,
            effective: "rendered",
          },
        },
      },
    },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Box", exact: true }),
    label: "Medius COM5",
    tone: "success",
  },
  {
    name: "a Medius rendering your movement without a profile yet",
    patch: {
      box: {
        ...connectedBox,
        link: "connected",
        has_mouse: true,
        medius: {
          ...connectedBox.medius,
          render: { ...connectedBox.medius!.render, full: true },
        },
      },
    },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Medius", exact: true }),
    label: "Relay only",
    tone: "warning",
  },
  {
    name: "a box that stopped answering",
    patch: {
      box: {
        ...fixture.box,
        error: "Medius stopped answering on COM5.",
      },
    },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Box", exact: true }),
    label: "Needs attention",
    tone: "danger",
  },
  {
    name: "box input losses beside Diagnostics",
    patch: {
      box: {
        ...connectedBox,
        link: "connected",
        has_mouse: true,
        medius: {
          ...connectedBox.medius,
          health: { ...connectedBox.medius!.health, link_drops: 3 },
        },
      },
    },
    where: async () => {},
    scope: (page) => page.locator(".section-intro"),
    label: "Box input losses",
    tone: "warning",
  },
  {
    name: "a running target capture",
    patch: { vision },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Target capture" }),
    label: "Running",
    tone: "success",
  },
  {
    name: "a failing target capture",
    patch: { vision: { ...vision, error: "the model could not load" } },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Target capture" }),
    label: "Needs attention",
    tone: "danger",
  },
  {
    name: "a warming target model",
    patch: { vision: { ...vision, ready: false } },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Target capture" }),
    label: "Warming up…",
    tone: "warning",
  },
  {
    name: "targets being recorded",
    patch: { vision: { ...vision, recording: true } },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Target capture" }),
    label: "Recording targets",
    tone: "success",
  },
  {
    name: "a target capture without fresh frames",
    patch: { vision: { ...vision, last: null } },
    where: async () => {},
    scope: (page) => page.getByRole("region", { name: "Target capture" }),
    label: "Waiting for frames",
    tone: "warning",
  },
  {
    name: "a paused live view",
    patch: {
      range: { from_us: fixture.plot.from_us, to_us: fixture.plot.to_us },
    },
    where: async () => {},
    scope: (page) => page.locator(".range-toolbar"),
    label: "Paused",
    tone: "warning",
  },
  {
    name: "an armed aim",
    patch: { experiment },
    where: (page) => page.getByRole("tab", { name: "Experiment", exact: true }).click(),
    scope: (page) => page.getByRole("region", { name: "Linear aim experiment" }),
    label: "Armed",
    tone: "warning",
  },
  {
    name: "a moving aim",
    patch: { experiment: { ...experiment, active: true, phase: "Tracking" } },
    where: (page) => page.getByRole("tab", { name: "Experiment", exact: true }).click(),
    scope: (page) => page.getByRole("region", { name: "Linear aim experiment" }),
    label: "Moving",
    tone: "success",
  },
];

for (const c of cases)
  test(`status tone: ${c.name}`, async ({ page }) => {
    await open(page, c.patch);
    await c.where(page);
    const badge = c
      .scope(page)
      .locator('[data-slot="badge"]')
      .filter({ hasText: new RegExp(`^${c.label}$`) });
    await expect(badge).toHaveAttribute("data-variant", c.tone);
    await expect(badge).toHaveCSS("color", textColor[c.tone]);
  });

test("a stopped target capture shows no badge", async ({ page }) => {
  await open(page, { vision: { ...vision, running: false } });
  await expect(
    page.getByRole("region", { name: "Target capture" }).locator('[data-slot="badge"]'),
  ).toHaveCount(0);
});

test("a section opens when it starts failing and folds again on request", async ({ page }) => {
  await open(page, {
    box: {
      ...fixture.box,
      error: "Medius stopped answering on COM5.",
    },
  });
  const section = page.getByRole("region", { name: "Box", exact: true });
  await expect(section.getByRole("alert")).toContainText("stopped answering");
  await page.getByRole("button", { name: "Box", exact: true }).click();
  await expect(section.getByRole("alert")).toHaveCount(0);
});

test("Medius settings stay a draft until applied, also while the section is folded", async ({
  page,
}) => {
  await open(page, {
    box: {
      ...connectedBox,
      link: "connected",
      has_mouse: true,
      medius: {
        ...connectedBox.medius,
        render: {
          mode: "despiked",
          full: false,
          ready: true,
          effective: "rendered",
        },
        emit: { ...connectedBox.medius!.emit, mode: "learned" },
      },
    },
  });
  const toggle = page.getByRole("button", { name: "Medius", exact: true });
  await toggle.click();
  const apply = page.getByRole("button", { name: "Apply settings" });
  await expect(apply).toBeDisabled();
  await page.getByLabel("Render mode").click();
  await page.getByRole("option", { name: "Stock" }).click();
  await expect(page.getByText("Not applied", { exact: true })).toBeVisible();
  await toggle.click();
  await toggle.click();
  await expect(page.getByLabel("Render mode")).toContainText("Stock");
  await expect(apply).toBeEnabled();
  await apply.click();
  const sent = await page.evaluate(() =>
    (
      window as unknown as {
        calls: { command: string; args: { command?: { action: string } } }[];
      }
    ).calls
      .filter((call) => call.command === "box_command")
      .map((call) => call.args.command!.action),
  );
  expect(sent).toEqual(["render", "emit", "spread", "riding", "imperfect"]);
});

test("a Medius with the render mode off says why it relays", async ({ page }) => {
  await open(page, {
    box: {
      ...connectedBox,
      link: "connected",
      has_mouse: true,
      port: "COM5",
      // nothing is rendered because nothing was asked for, and no profile is needed to relay
      medius: {
        ...connectedBox.medius,
        render: { mode: "off", full: false, ready: false, effective: "relay" },
      },
    },
  });
  const medius = page.getByRole("region", { name: "Medius", exact: true });
  await page.getByRole("button", { name: "Medius", exact: true }).click();
  // the box is connected and doing what it was told, so the header names it instead of warning
  await expect(
    page
      .getByRole("region", { name: "Box", exact: true })
      .getByText("Medius COM5", { exact: true }),
  ).toHaveAttribute("data-variant", "success");
  await expect(medius.getByText("Relay only", { exact: true })).toHaveAttribute(
    "data-variant",
    "muted",
  );
  await expect(
    medius.getByText("the render mode is off, so the box passes your movement through unchanged"),
  ).toBeVisible();
  // the profile is not needed while the mode is off, so nothing asks for movement
  await expect(medius.getByRole("alert")).toHaveCount(0);
});

test("a Medius relaying physical movement with a ready profile says why", async ({ page }) => {
  await open(page, {
    box: {
      ...connectedBox,
      link: "connected",
      has_mouse: true,
      port: "COM5",
      // injected motion is rendered, the hand's own movement is not
      medius: {
        ...connectedBox.medius,
        render: {
          mode: "despiked",
          full: false,
          ready: true,
          effective: "relay",
        },
      },
    },
  });
  const medius = page.getByRole("region", { name: "Medius", exact: true });
  await page.getByRole("button", { name: "Medius", exact: true }).click();
  await expect(
    page
      .getByRole("region", { name: "Box", exact: true })
      .getByText("Medius COM5", { exact: true }),
  ).toHaveAttribute("data-variant", "success");
  await expect(medius.getByText("Relay only", { exact: true })).toHaveAttribute(
    "data-variant",
    "muted",
  );
  await expect(
    medius.getByText(
      "Render physical movement is off, so your mouse's own movement passes through unchanged",
    ),
  ).toBeVisible();
  // moving the mouse through the box would change nothing here
  await expect(medius.getByRole("alert")).toHaveCount(0);
});

test("a profile that isn't ready only matters when your own movement is rendered", async ({
  page,
}) => {
  await open(page, {
    box: {
      ...connectedBox,
      link: "connected",
      has_mouse: true,
      port: "COM5",
      medius: {
        ...connectedBox.medius,
        render: {
          mode: "despiked",
          full: false,
          ready: false,
          effective: "relay",
        },
      },
    },
  });
  const medius = page.getByRole("region", { name: "Medius", exact: true });
  await page.getByRole("button", { name: "Medius", exact: true }).click();
  await expect(
    page
      .getByRole("region", { name: "Box", exact: true })
      .getByText("Medius COM5", { exact: true }),
  ).toHaveAttribute("data-variant", "success");
  await expect(medius.getByRole("alert")).toHaveCount(0);
  await expect(medius.getByText(/renderer isn't active/)).toHaveCount(0);
});

test("a box other than a Medius shows none of the Medius settings", async ({ page }) => {
  const why =
    "only a Medius stamps your mouse's movement with its own clock, and that's what lines it up with the USB reports";
  await open(page, {
    box: {
      ...fixture.box,
      family: "MAKCU V4",
      name: "MAKCU V4",
      link: "connected",
      has_mouse: true,
      port: "COM11",
      features: { physical_input: why },
      medius: null,
    },
  });
  const section = page.getByRole("region", { name: "Box", exact: true });
  await page.getByRole("button", { name: "Box", exact: true }).click();
  await expect(section.getByText("MAKCU V4 COM11", { exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "Medius", exact: true })).toHaveCount(0);
  await expect(page.getByLabel("Render mode")).toHaveCount(0);
  // the box can't give it, so the row says why in place of the switch
  await expect(section.getByText(why)).toBeVisible();
  await expect(section.getByRole("switch")).toHaveCount(0);
});

test("a dropdown in a locked group doesn't open", async ({ page }) => {
  await open(page, { vision });
  await page.getByRole("button", { name: "Target capture", exact: true }).click();
  const provider = page.getByLabel("Inference provider");
  await expect(provider).toBeDisabled();
  await provider.click({ force: true });
  await expect(page.getByRole("listbox")).toHaveCount(0);
});

test("an armed aim that loses the box says so", async ({ page }) => {
  await open(page, {
    experiment: {
      ready: true,
      armed: true,
      active: false,
      learning: false,
      hotkey_mask: 8,
      held_mask: 0,
      binding_seq: 0,
      phase: "Waiting for the box to come back",
      commands: 0,
      counts: [0, 0],
      target_counts: null,
      recording: false,
      away: true,
      error: null,
    },
  });
  await expect(page.getByText(/the box or its mouse went away/)).toBeVisible();
});

test("the box picker lists every box and sends the pick", async ({ page }) => {
  await open(page, {
    box: {
      ...fixture.box,
      link: "none",
      picking: true,
      key: null,
      boxes: [
        { key: "MAC a", label: "Medius COM18", model: null, usable: true },
        {
          key: "MAKCU V4 COM11",
          label: "MAKCU V4 COM11",
          model: null,
          usable: true,
        },
        { key: "keys", label: "MAKCU V4 COM9", model: null, usable: false },
      ],
    },
  });
  const picker = page.getByLabel("Which box", { exact: true });
  await expect(picker).toContainText("Pick a box");
  await picker.click();
  await expect(page.getByRole("option", { name: "MAKCU V4 COM9" })).toBeDisabled();
  // the list holds boxes and nothing else
  await expect(page.getByRole("option")).toHaveCount(3);
  await page.getByRole("option", { name: "MAKCU V4 COM11" }).click();
  await page.getByRole("button", { name: "Look for boxes again" }).click();
  const sent = await page.evaluate(() =>
    (
      window as unknown as {
        calls: {
          command: string;
          args: { command?: { action: string; key?: string } };
        }[];
      }
    ).calls
      .filter((call) => call.command === "box_command")
      .map((call) => call.args.command),
  );
  expect(sent).toEqual([{ action: "pick", key: "MAKCU V4 COM11" }, { action: "refresh" }]);
});

test("a box standing in for the remembered one says so and can be kept", async ({ page }) => {
  await open(page, {
    box: {
      ...fixture.box,
      link: "connected",
      key: "MAKCU V4 COM11",
      remembered: "MAC a",
      notice: "using MAKCU V4 COM11 for now",
      boxes: [
        {
          key: "MAKCU V4 COM11",
          label: "MAKCU V4 COM11",
          model: null,
          usable: true,
        },
      ],
    },
  });
  // a warning that stays until it is closed, with the stand-in a click from being the default
  const warning = page.locator("[data-sonner-toast]").filter({
    hasText: "isn't plugged in",
  });
  await expect(warning).toBeVisible();
  await warning.getByRole("button", { name: "Use this box as default" }).click();
  const sent = await page.evaluate(() =>
    (
      window as unknown as {
        calls: { command: string; args: { command?: { action: string } } }[];
      }
    ).calls
      .filter((call) => call.command === "box_command")
      .map((call) => call.args.command),
  );
  expect(sent).toContainEqual({ action: "pick", key: "MAKCU V4 COM11" });
});

test("the cue volume saves on release and its icon mutes it", async ({ page }) => {
  await open(page, {});
  await page.getByLabel("Settings").click();
  await page.getByRole("button", { name: "Mute the cues" }).click();
  await expect(page.getByRole("button", { name: "Unmute the cues" })).toBeVisible();
  const saved = () =>
    page.evaluate(
      () =>
        (
          window as unknown as {
            savedSettings: () => {
              display: { volume: number; muted: boolean };
            };
          }
        ).savedSettings().display,
    );
  await expect.poll(saved).toMatchObject({ muted: true, volume: 0.6 });
  const slider = page.getByRole("slider", { name: "Cue volume" });
  await slider.focus();
  await page.keyboard.press("End");
  await expect.poll(saved).toMatchObject({ muted: false, volume: 1 });
});

test("the theme switches the page and its charts, and is saved", async ({ page }) => {
  await open(page, {});
  const background = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  const dark = await background();
  await page.getByLabel("Settings").click();
  await page.getByRole("combobox", { name: "Theme" }).click();
  await page.getByRole("option", { name: "Light" }).click();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  expect(await background()).not.toBe(dark);
  // the canvas charts draw their grid with this variable
  const grid = () =>
    page.evaluate(() =>
      getComputedStyle(document.documentElement).getPropertyValue("--color-chart-grid").trim(),
    );
  expect(await grid()).toBe("#e4e7eb");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              savedSettings: () => { display: { theme: string } };
            }
          ).savedSettings().display.theme,
      ),
    )
    .toBe("light");
  await page.getByRole("combobox", { name: "Theme" }).click();
  await page.getByRole("option", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  expect(await background()).toBe(dark);
});

test("a new recordings folder is saved, offers the restart, and goes back to the default", async ({
  page,
}) => {
  await open(page, {});
  await page.getByLabel("Settings").click();
  const popover = page.locator(".display-settings");
  await expect(popover.getByText("Motion Bench", { exact: false })).toBeVisible();
  await expect(popover.getByRole("button", { name: "Restart" })).toHaveCount(0);
  await popover.getByRole("button", { name: "Change" }).click();
  await expect(popover.getByText("D:\\Recordings")).toBeVisible();
  const saved = () =>
    page.evaluate(
      () =>
        (
          window as unknown as {
            savedSettings: () => { storage: { folder: string | null } };
          }
        ).savedSettings().storage.folder,
    );
  expect(await saved()).toBe("D:\\Recordings");
  // the running app keeps its folder until it starts again
  await expect(popover.getByRole("button", { name: "Restart" })).toBeVisible();
  await popover.getByRole("button", { name: "Restart" }).click();
  expect(
    await page.evaluate(() =>
      (window as unknown as { calls: { command: string }[] }).calls.some(
        (call) => call.command === "restart_app",
      ),
    ),
  ).toBe(true);
  await popover.getByRole("button", { name: "Default" }).click();
  expect(await saved()).toBeNull();
  await expect(popover.getByRole("button", { name: "Restart" })).toHaveCount(0);
});

test("a newer release is offered at start and installs on request", async ({ page }) => {
  await open(page, { update: "0.4.0" });
  const toast = page.locator("[data-sonner-toast]", {
    hasText: "Motion Bench 0.4.0 is out",
  });
  await expect(toast).toBeVisible();
  await toast.getByRole("button", { name: "Update and restart" }).click();
  await expect(page.getByText("Downloading the update…")).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as unknown as { calls: { command: string }[] }).calls.some(
        (call) => call.command === "install_update",
      ),
    ),
  ).toBe(true);
});

test("a port the scan couldn't use says why under the picker", async ({ page }) => {
  const why = "COM5 is already open in another program, close it there first";
  await open(page, {
    box: { ...fixture.box, unusable: [{ port: "COM5", why }] },
  });
  await page.getByRole("button", { name: "Box", exact: true }).click();
  await expect(page.getByRole("region", { name: "Box", exact: true }).getByText(why)).toBeVisible();
});

test("a report rate above 1 kHz warns that the judges don't know it", async ({ page }) => {
  await open(page, {
    plot: {
      ...fixture.plot,
      readouts: { ...fixture.plot.readouts, rate_hz: 8000 },
    },
  });
  await expect(page.locator(".metric-warning")).toContainText("trained on 1 kHz");
});

test("the recordings sort by name, date and box", async ({ page }) => {
  const run = fixture.runs[0];
  const at = (day: number) => Date.UTC(2026, 9, day) * 1000;
  await open(page, {
    runs: [
      {
        ...run,
        id: "a",
        name: "alpha",
        started_us: at(2),
        metadata: { box: { family: "MAKCU V4", name: "MAKCU V4" } },
      },
      {
        ...run,
        id: "b",
        name: "bravo",
        started_us: at(9),
        metadata: { box: { family: "Medius", name: "box1" } },
      },
      { ...run, id: "c", name: "charlie", started_us: null, metadata: {} },
    ],
  });
  await page.getByRole("tab", { name: /^Runs/ }).click();
  const names = () => page.locator(".run-name").allTextContents();
  // newest first, and one with no date goes last
  expect(await names()).toEqual(["bravo", "alpha", "charlie"]);
  await page.getByRole("button", { name: "Recording" }).click();
  expect(await names()).toEqual(["alpha", "bravo", "charlie"]);
  await page.getByRole("button", { name: "Recording" }).click();
  expect(await names()).toEqual(["charlie", "bravo", "alpha"]);
  await page.getByRole("button", { name: "Box", exact: true }).click();
  expect(await names()).toEqual(["alpha", "bravo", "charlie"]);
  await expect(page.getByRole("columnheader", { name: "Box" })).toHaveAttribute(
    "aria-sort",
    "ascending",
  );
});
