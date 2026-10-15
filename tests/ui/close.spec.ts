import { snapshot as fixture } from "../../ui/fixture";
import { expect, test } from "./settings-mock";

test("native close flushes pending settings and destroys the window without another close request", async ({
  page,
}) => {
  await page.addInitScript((fixture) => {
    const callbacks = new Map<number, (event: unknown) => Promise<void>>();
    const events = new Map<string, number>();
    const commands: string[] = [];
    let sequence = 0;
    Object.assign(window, {
      closeCommands: commands,
      closeNative: async () => {
        const callback = callbacks.get(events.get("tauri://close-requested")!);
        await callback!({
          event: "tauri://close-requested",
          id: 1,
          payload: null,
        });
      },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: (callback: (event: unknown) => Promise<void>) => {
          callbacks.set(++sequence, callback);
          return sequence;
        },
        unregisterCallback: () => {},
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          commands.push(command);
          if (command === "runs")
            return {
              revision: fixture.library,
              runs: structuredClone(fixture.runs),
            };
          if (command === "initialize") return { roots: [], snapshot: fixture };
          if (command === "plugin:event|listen") {
            events.set(String(args.event), Number(args.handler));
            return 1;
          }
          if (command === "update_settings") {
            await new Promise((resolve) => setTimeout(resolve, 30));
            if ((window as unknown as { failCloseSave?: boolean }).failCloseSave)
              throw new Error("disk unavailable");
          }
          if (command === "run_plot") return fixture.plot;
        },
      },
      isTauri: true,
    });
  }, fixture);
  await page.goto("/");
  await page.getByLabel("Settings").click();
  await page.getByLabel("UI scale").click();
  await page
    .getByRole("option")
    .filter({ hasText: /^110%$/ })
    .click();
  await page.evaluate(() =>
    (window as unknown as { closeNative: () => Promise<void> }).closeNative(),
  );
  const commands = await page.evaluate(
    () => (window as unknown as { closeCommands: string[] }).closeCommands,
  );
  expect(commands).toContain("update_settings");
  expect(commands).toContain("plugin:window|destroy");
  expect(commands.indexOf("update_settings")).toBeLessThan(
    commands.indexOf("plugin:window|destroy"),
  );
  expect(commands).not.toContain("plugin:window|close");
  await page.evaluate(() =>
    (window as unknown as { closeNative: () => Promise<void> }).closeNative(),
  );
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { closeCommands: string[] }).closeCommands.filter(
          (c) => c === "plugin:window|destroy",
        ).length,
    ),
  ).toBe(2);
});
