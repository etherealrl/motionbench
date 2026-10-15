import { connectedBox, snapshot as fixture } from "../../ui/fixture";
import { expect, test } from "./settings-mock";

test("Medius recording button picks, cancels and records without frontend polling", async ({
  page,
}) => {
  // the longest walk through the app, it needs more than the default time when the suite runs
  // in parallel
  test.slow();
  await page.addInitScript(
    (fixture) => {
      const status = {
        hotkey_mask: null as number | null,
        ready: true,
        learning: false,
        held_mask: 0,
        binding_seq: 0,
        busy: false,
        message: null as string | null,
        error: null as string | null,
      };
      const snapshot = {
        ...structuredClone(fixture),
        live: true,
        recording: null as string | null,
        recording_hotkey: status,
      };
      Object.assign(snapshot.box, {
        link: "connected",
        has_mouse: true,
        pending_commands: 0,
        error: null,
      });
      // what native says at each step, set where the step happens
      const blocker = (need: string, message: string, waits = false) => ({
        need,
        message,
        waits,
      });
      const readiness = snapshot.readiness as unknown as Record<
        string,
        ReturnType<typeof blocker> | null
      >;
      Object.assign(readiness, {
        record: null,
        recording_button: null,
        clear_recording_button: null,
        box_connect: null,
        box_settings: null,
      });
      const commands: string[] = [];
      let channel: { onmessage: (v: unknown) => void } | null = null;
      let stalled = false;
      let completed = 0;
      const publish = () => {
        if (!stalled) channel?.onmessage(structuredClone(snapshot));
      };
      const toggle = () => {
        if (status.busy || status.learning) return;
        if (!snapshot.recording && !snapshot.live) {
          status.error = "start monitoring to record";
          status.message = "Recording request failed";
          publish();
          return;
        }
        if (snapshot.recording) {
          const id = snapshot.recording;
          snapshot.recording = null;
          completed++;
          snapshot.runs.push({
            ...structuredClone(fixture.runs[0]),
            id,
            name: `${id}.pcap`,
          });
          snapshot.library++;
          status.message = "saved to Runs, USB monitoring keeps running";
          readiness.recording_button = null;
          readiness.clear_recording_button = null;
          readiness.box_connect = null;
          readiness.box_settings = null;
        } else {
          readiness.box_connect = blocker(
            "recording",
            "finish recording before connecting, disconnecting or switching the box",
          );
          readiness.box_settings = blocker(
            "recording",
            "finish recording before changing the box's settings",
          );
          readiness.recording_button = blocker(
            "recording",
            "finish recording before changing its button",
          );
          readiness.clear_recording_button = blocker(
            "recording",
            "finish recording before clearing its button",
          );
          snapshot.recording = completed ? `mouse-trial-${completed + 1}` : "mouse-trial";
          status.message = "Recording mouse-trial";
        }
        publish();
      };
      Object.assign(window, {
        isTauri: true,
        recordingCommands: commands,
        mediusButtons: (mask: number) => {
          const pressed = mask & ~status.held_mask;
          status.held_mask = mask;
          if (!status.ready) return;
          if (status.learning) {
            if (pressed === 8)
              status.error = "that button is the aim hold binding, press a different mouse button";
            else if (pressed && (pressed & (pressed - 1)) === 0) {
              status.hotkey_mask = pressed;
              status.binding_seq++;
              status.learning = false;
              status.error = null;
            }
          } else if (status.hotkey_mask && pressed & status.hotkey_mask) toggle();
          publish();
        },
        stallPresentation: (value: boolean) => {
          stalled = value;
          publish();
        },
        monitoring: (value: boolean) => {
          snapshot.live = value;
          readiness.record = value
            ? null
            : blocker("capture_stopped", "start monitoring to record");
          status.error = null;
          publish();
        },
        recordingBusy: (value: boolean) => {
          status.busy = value;
          readiness.recording_button = value
            ? blocker("busy", "the last action is still finishing, wait for it")
            : null;
          publish();
        },
        recordingConnected: (value: boolean) => {
          snapshot.box.link = value ? "connected" : "none";
          readiness.recording_button = value
            ? null
            : blocker("no_box", "connect a box to pick a button");
          publish();
        },
        __TAURI_INTERNALS__: {
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
          },
          transformCallback: () => 1,
          unregisterCallback: () => {},
          invoke: async (command: string, args: Record<string, unknown> = {}) => {
            commands.push(command);
            if (command === "runs")
              return {
                revision: snapshot.library,
                runs: structuredClone(snapshot.runs),
              };
            if (command === "initialize") return { roots: [], snapshot };
            if (command === "subscribe") {
              channel = args.channel as typeof channel;
              setInterval(publish, 50);
            }
            if (command === "recording_hotkey_learn") {
              status.learning = !args.cancel;
              status.error = null;
              publish();
              return structuredClone(status);
            }
            if (command === "recording_hotkey_disable") {
              status.hotkey_mask = null;
              publish();
            }
            if (command === "recording_toggle") {
              toggle();
              return "mouse-trial";
            }
            if (command === "run_plot") return fixture.plot;
            if (command === "plugin:event|listen") return 1;
          },
        },
      });
    },
    { ...fixture, box: connectedBox },
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Box", exact: true }).click();
  const panel = page.getByRole("region", { name: "Recording mouse button" });
  const picker = panel.getByRole("button", {
    name: /^(Recording button:|Cancel recording button binding)/,
  });
  const buttons = (mask: number) =>
    page.evaluate(
      (mask) =>
        (window as unknown as { mediusButtons: (mask: number) => void }).mediusButtons(mask),
      mask,
    );
  await expect(panel.locator(".mouse-binding-picker")).toHaveText("Unset");
  await picker.click();
  await expect(panel).toContainText(
    "press and release the button you want to use, on the mouse plugged into the box",
  );
  await page.keyboard.press("F8");
  await expect(panel.locator(".mouse-binding-picker")).toHaveText("Press a button…");
  await buttons(16);
  await expect(panel.locator(".mouse-binding-picker")).toHaveText("Side 2");
  await expect(picker).toBeEnabled();
  await panel.screenshot({
    path: "test-results/recording-binding-assigned.png",
  });
  await expect(panel.getByRole("button", { name: "Clear recording button" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);
  await buttons(16);
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);
  await buttons(0);
  await picker.click();
  await panel.getByRole("button", { name: "Cancel recording button binding" }).click();
  await expect(panel.locator(".mouse-binding-picker")).toHaveText("Side 2");
  await picker.click();
  await expect(panel).toContainText("press and release");
  await page.getByRole("tab", { name: "Compare", exact: true }).click();
  await page.getByRole("tab", { name: "Capture", exact: true }).click();
  await expect(picker).toBeEnabled();
  await expect(panel.locator(".mouse-binding-picker")).toHaveText("Side 2");
  // folding the section cancels a pick too, nothing is left listening out of sight
  await picker.click();
  await expect(panel).toContainText("press and release");
  const section = page.getByRole("button", { name: "Box", exact: true });
  await section.click();
  await section.click();
  await expect(picker).toBeEnabled();
  await expect(panel.locator(".mouse-binding-picker")).toHaveText("Side 2");
  await panel.getByRole("button", { name: "Clear recording button", exact: true }).click();
  await expect(panel.locator(".mouse-binding-picker")).toHaveText("Unset");
  await picker.click();
  await buttons(16);
  await buttons(0);
  await expect(panel.locator(".mouse-binding-picker")).toHaveText("Side 2");
  for (const width of [1000, 1600]) {
    await page.setViewportSize({ width, height: 1200 });
    await page.getByLabel("Settings").click();
    for (const scale of [1, 1.25, 1.5]) {
      await page.locator("body").evaluate((body, scale) => {
        body.style.zoom = String(scale);
      }, scale);
      expect(await panel.evaluate((panel) => panel.scrollWidth > panel.clientWidth + 1)).toBe(
        false,
      );
      const centres = await panel.locator("button").evaluateAll((buttons) =>
        buttons.map((b) => {
          const box = b.getBoundingClientRect();
          return box.y + box.height / 2;
        }),
      );
      expect(centres).toHaveLength(2);
      expect(Math.max(...centres) - Math.min(...centres)).toBeLessThan(2);
    }
  }
  await page.locator("body").evaluate((body) => {
    body.style.zoom = "1";
  });
  await page.evaluate(() =>
    (window as unknown as { recordingBusy: (v: boolean) => void }).recordingBusy(true),
  );
  await expect(picker).toBeDisabled();
  await page.evaluate(() =>
    (window as unknown as { recordingBusy: (v: boolean) => void }).recordingBusy(false),
  );
  await page.evaluate(() =>
    (window as unknown as { recordingConnected: (v: boolean) => void }).recordingConnected(false),
  );
  // the binding needs the box, so without it there is nothing to press
  await expect(picker).toHaveCount(0);
  await page.evaluate(() =>
    (window as unknown as { recordingConnected: (v: boolean) => void }).recordingConnected(true),
  );
  await expect(picker).toBeEnabled();
  await page.evaluate(() =>
    (window as unknown as { stallPresentation: (v: boolean) => void }).stallPresentation(true),
  );
  await buttons(16);
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);
  await page.evaluate(() =>
    (window as unknown as { stallPresentation: (v: boolean) => void }).stallPresentation(false),
  );
  await expect(page.getByRole("button", { name: "Finish recording" })).toBeVisible();
  await expect(picker).toBeDisabled();
  const boxRegion = page.getByRole("region", {
    name: "Box",
    exact: true,
  });
  await expect(boxRegion.getByRole("button", { name: "Disconnect box" })).toBeDisabled();
  await expect(boxRegion.getByLabel("Record physical input")).toBeDisabled();
  await expect(page.getByRole("region", { name: "Medius", exact: true })).toContainText(
    "finish recording before changing the box's settings",
  );
  await expect(panel).toContainText("finish recording before changing its button");
  await boxRegion.screenshot({ path: "test-results/box-recording.png" });
  await buttons(16);
  await expect(page.getByRole("button", { name: "Finish recording" })).toBeVisible();
  await buttons(0);
  await buttons(16);
  await expect(panel).toContainText("saved to Runs");
  await expect(page.getByRole("button", { name: "Stop monitoring", exact: true })).toBeVisible();
  await buttons(0);
  await buttons(16);
  await expect(page.getByRole("button", { name: "Finish recording" })).toBeVisible();
  await buttons(0);
  await buttons(16);
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);
  await expect(panel).toContainText("saved to Runs");
  await page.evaluate(() =>
    (window as unknown as { monitoring: (v: boolean) => void }).monitoring(false),
  );
  await buttons(0);
  await buttons(16);
  await expect(panel).toContainText("start monitoring to record");
  await expect(page.getByRole("button", { name: "Record", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Finish recording" })).toHaveCount(0);
  await expect(page.getByRole("tab", { name: "Capture" })).toHaveAttribute("data-state", "active");
  await page.getByRole("tab", { name: "Runs" }).click();
  const library = page.getByRole("table", { name: "Recordings" });
  await expect(library.getByText("mouse-trial.pcap", { exact: true })).toBeVisible();
  await expect(library.getByText("mouse-trial-2.pcap", { exact: true })).toBeVisible();
  const commands = await page.evaluate(
    () => (window as unknown as { recordingCommands: string[] }).recordingCommands,
  );
  expect(commands).not.toContain("recording_toggle");
  expect(commands).not.toContain("screen_events");
  expect(commands).not.toContain("import_run");
});
