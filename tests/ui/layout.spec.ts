import { connectedBox, snapshot as fixture } from "../../ui/fixture";
import { expect, test } from "./settings-mock";
import type { Locator, Page } from "@playwright/test";

// the spacing every screen shares, measured, so a stylesheet that drifts from it fails here

const top = async (locator: Locator) => (await locator.boundingBox())!.y;
const left = async (locator: Locator) => (await locator.boundingBox())!.x;

/** the desktop app with a Medius connected, so every setup section has its settings */
async function connected(page: Page, patch: Record<string, unknown> = {}) {
  await page.addInitScript(
    ({ fixture, box, patch }) => {
      const snapshot = {
        ...structuredClone(fixture),
        live: true,
        box,
        ...patch,
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
            if (command === "runs") return { revision: snapshot.library, runs: snapshot.runs };
            if (command === "initialize") return { roots: [], snapshot };
            if (command === "plugin:event|listen") return 1;
          },
        },
      });
    },
    { fixture, box: connectedBox, patch },
  );
  await page.goto("/");
}

async function openSections(page: Page) {
  const triggers = page.locator(".setup-section-trigger[aria-expanded=false]");
  while ((await triggers.count()) > 0) await triggers.first().click();
}

test("every tab puts its title and its first card in the same place", async ({ page }) => {
  await page.goto("/");
  const places = [];
  for (const tab of ["Capture", "Compare", "Experiment", "Runs"]) {
    await page.getByRole("tab", { name: new RegExp(`^${tab}`) }).click();
    const intro = page.locator(".section-intro").first();
    const card = page.locator(".section-intro + * .panel, .section-intro + .panel").first();
    places.push({
      tab,
      title: [await left(intro), await top(intro)],
      card: [await left(card), await top(card)],
    });
  }
  for (const place of places.slice(1)) {
    expect(place.title, place.tab).toEqual(places[0].title);
    expect(place.card, place.tab).toEqual(places[0].card);
  }
});

test("a section title sits the same distance above whatever it heads", async ({ page }) => {
  await connected(page);
  await openSections(page);
  const sections = page.locator(".setting-section");
  const count = await sections.count();
  expect(count).toBeGreaterThan(1);
  const gaps = new Set<number>();
  for (let i = 0; i < count; i++) {
    const section = sections.nth(i);
    if (!(await section.isVisible())) continue;
    const title = (await section.locator("h4").boundingBox())!;
    const next = await top(section.locator("h4 + *"));
    gaps.add(Math.round((next - title.y - title.height) * 10) / 10);
  }
  expect([...gaps]).toHaveLength(1);
});

test("readout rows keep one pitch, badges or not", async ({ page }) => {
  // without the driver's row, the Raw Input badge sits between rows of plain text
  await connected(page, { dev_mode: false });
  await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
  // a group's readouts one under another count as one list
  const readouts = page.locator(".status-diagnostics .diagnostics-group");
  await expect(readouts.first()).toBeVisible();
  // from the middle of one row to the next, between rows of one line; a label or value that
  // wraps makes its row taller, as it should
  const rows = await readouts.evaluateAll((lists) =>
    lists.map((list) =>
      [...list.querySelectorAll("dt")].map((dt) => {
        const cells = [dt, dt.nextElementSibling!].map((e) => e.getBoundingClientRect());
        const line = parseFloat(getComputedStyle(dt).lineHeight);
        return {
          middle: (cells[0].top + cells[0].bottom) / 2,
          single: cells.every((c) => c.height < line * 1.5),
        };
      }),
    ),
  );
  const pitches = new Set<number>();
  for (const list of rows)
    for (let i = 1; i < list.length; i++)
      if (list[i - 1].single && list[i].single)
        pitches.add(Math.round((list[i].middle - list[i - 1].middle) * 10) / 10);
  expect(pitches.size).toBeGreaterThan(0);
  expect([...pitches]).toHaveLength(1);
});
