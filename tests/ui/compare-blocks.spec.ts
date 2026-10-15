import { numbers, runPlot, snapshot as fixture } from "../../ui/fixture";
import { expect, test } from "./settings-mock";

// three recordings natively listed, two with their box input and one without
test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ({ fixture, runPlot, numbers }) => {
      const snapshot = structuredClone(fixture);
      const run = snapshot.runs[0];
      const paired = { ...run.refusals, paired: null };
      snapshot.runs = [
        {
          ...run,
          id: "first",
          name: "first.pcap",
          paired: true,
          refusals: paired,
        },
        {
          ...run,
          id: "second",
          name: "second.pcap",
          paired: true,
          refusals: paired,
        },
        { ...run, id: "unpaired", name: "unpaired.pcap" },
      ];
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
          },
        },
      });
    },
    { fixture, runPlot, numbers },
  );
  await page.goto("/");
  await page.getByRole("tab", { name: "Compare", exact: true }).click();
  const mode = page.getByLabel("Comparison mode");
  await mode.click();
  await page.locator('[role=option][data-value="paired"]').click();
  await page.getByRole("button", { name: "Choose candidate recordings", exact: true }).click();
});

test("a paired comparison takes one recording, and only one with its box input", async ({
  page,
}) => {
  const candidate = page.getByRole("group", { name: "Candidate recordings" });
  const pick = (name: string) => candidate.getByRole("checkbox", { name: new RegExp(name) });
  const run = page.getByRole("button", { name: "Run comparison" });

  await pick("first.pcap").check();
  await pick("second.pcap").check();
  await page.keyboard.press("Escape");
  await expect(
    page.getByText(
      "a paired comparison reads one recording against its own box input, so pick just one, 2 are selected",
    ),
  ).toBeVisible();
  await expect(run).toBeDisabled();

  await page.getByRole("button", { name: "Choose candidate recordings", exact: true }).click();
  await pick("second.pcap").uncheck();
  await page.keyboard.press("Escape");
  await expect(page.getByText("pick just one")).toHaveCount(0);
  await expect(run).toBeEnabled();

  await page.getByRole("button", { name: "Choose candidate recordings", exact: true }).click();
  await pick("first.pcap").uncheck();
  await pick("unpaired.pcap").check();
  await page.keyboard.press("Escape");
  await expect(
    page.getByText(
      "unpaired.pcap can't be compared this way, it has no physical input, the box didn't record the mouse",
    ),
  ).toBeVisible();
  await expect(run).toBeDisabled();
});
