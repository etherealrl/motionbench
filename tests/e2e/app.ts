// the real app as it ships, driven through a WebDriver server built into the e2e build. the
// native side is the real one, so a setting the UI sends that native refuses fails here, which
// the browser tests with their stand in for native can't see
//
//   bun run build:e2e
//   bun run test:e2e
import { execSync, spawn } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test as base } from "@playwright/test";

const exe = resolve("target/e2e/release/motion-bench.exe");
const fixtures = resolve("tests/fixtures");
const port = 4445;
const server = `http://127.0.0.1:${port}`;
// how W3C WebDriver names an element in a reply
const ELEMENT = "element-6066-11e4-a52f-4a5d23de5d58";

/** a profile of its own, so a test never touches your settings, recordings or WebView data */
function profile() {
  const home = mkdtempSync(join(tmpdir(), "motion-bench-e2e-"));
  const recordings = join(home, "recordings");
  mkdirSync(join(home, ".motion-bench"), { recursive: true });
  mkdirSync(recordings);
  for (const file of ["mouse-1000hz.pcap", "mouse-1000hz.json"])
    copyFileSync(join(fixtures, file), join(recordings, file));
  // the recordings folder is the only setting it starts with, the rest are the defaults
  writeFileSync(
    join(home, ".motion-bench", "config.json"),
    JSON.stringify({ version: 1, storage: { folder: recordings } }),
  );
  return home;
}

/** the settings native saved, read from the file */
export function saved(home: string) {
  return JSON.parse(readFileSync(join(home, ".motion-bench", "config.json"), "utf8"));
}

async function call(method: string, path: string, body?: unknown) {
  const reply = await fetch(server + path, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const json = (await reply.json()) as { value: unknown };
  if (!reply.ok) throw new Error(`${method} ${path}: ${JSON.stringify(json.value)}`);
  return json.value;
}

type Element = { [ELEMENT]: string; tag: string };

/** waits for `check` to give something truthy, retrying what throws, the way expect.poll does */
async function until<T>(
  check: () => Promise<T>,
  what: string,
  timeout = 10_000,
): Promise<NonNullable<T>> {
  const end = Date.now() + timeout;
  let last = "";
  for (;;) {
    try {
      const value = await check();
      if (value) return value as NonNullable<T>;
    } catch (e) {
      last = `: ${e instanceof Error ? e.message : JSON.stringify(e)}`;
    }
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}${last}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

export type App = Awaited<ReturnType<typeof launch>>;

/** starts the app on `home` and opens a WebDriver session on its window */
export async function launch(home: string) {
  const child = spawn(exe, [], {
    env: {
      ...process.env,
      USERPROFILE: home,
      WEBVIEW2_USER_DATA_FOLDER: join(home, "webview"),
      TAURI_WEBDRIVER_PORT: String(port),
    },
    stdio: "ignore",
  });
  await until(() => call("GET", "/status").then(() => true), "the app's WebDriver server", 30_000);
  const { sessionId } = (await call("POST", "/session", { capabilities: {} })) as {
    sessionId: string;
  };
  const session = `/session/${sessionId}`;
  /** runs `script` in the page, with `args` as `arguments` */
  const run = (script: string, ...args: unknown[]) =>
    call("POST", `${session}/execute/sync`, { script, args });
  // a script can't hand an element back through this server, so it tags the one it found and
  // WebDriver looks the tag up
  let tags = 0;
  /** the element `script` returns, waiting for it to be there */
  const element = (what: string, script: string, ...args: unknown[]) => {
    const tag = String(++tags);
    return until(async () => {
      const found = await run(
        `const e = (() => { ${script} })(...arguments); if (e) e.setAttribute("data-e2e", ${JSON.stringify(tag)}); return !!e;`,
        ...args,
      );
      if (!found) return null;
      const reply = (await call("POST", `${session}/element`, {
        using: "css selector",
        value: `[data-e2e="${tag}"]`,
      })) as Record<string, string>;
      // the id under whichever key this server uses for it
      return { [ELEMENT]: reply[ELEMENT] ?? Object.values(reply)[0], tag } as Element;
    }, what);
  };
  /** runs `body` with the element as `e` */
  const on = (target: Element, body: string) =>
    run(
      `const e = document.querySelector(${JSON.stringify(`[data-e2e="${target.tag}"]`)}); ${body}`,
    );
  return {
    run,
    on,
    /** the element a script finds, for what the other finders don't cover */
    find: element,
    /** a tab by its name, whatever count it shows beside it */
    tab: (name: string) =>
      element(
        `the ${name} tab`,
        `return [...document.querySelectorAll("[role=tab]")].find((e) => e.textContent.trim().startsWith(arguments[0])) ?? null;`,
        name,
      ),
    /** the element with this accessible label, the way a user finds it */
    byLabel: (label: string) =>
      element(
        `"${label}"`,
        `const want = arguments[0];
         const named = (e) => (e.getAttribute("aria-label") ?? e.labels?.[0]?.textContent ?? "").trim() === want;
         return [...document.querySelectorAll("input,button,[role=combobox],[role=slider]")].find(named) ?? null;`,
        label,
      ),
    /** the shown element whose whole text is `text` */
    byText: (text: string, selector = "*") =>
      element(
        `"${text}"`,
        `return [...document.querySelectorAll(arguments[1])].find((e) => e.textContent.trim() === arguments[0] && e.getClientRects().length > 0) ?? null;`,
        text,
        selector,
      ),
    click: async (target: Element) => {
      await call("POST", `${session}/element/${target[ELEMENT]}/click`, {});
    },
    /** replaces what's in a field the way typing does, through the setter React listens to, then
     * leaves it so it's saved */
    fill: async (target: Element, text: string) => {
      await on(
        target,
        `e.focus();
         Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(e, ${JSON.stringify(text)});
         e.dispatchEvent(new Event("input", { bubbles: true }));`,
      );
      // apart from the typing, like a person leaving the field, so React has the typed value by then
      await on(target, "e.blur()");
    },
    text: (target: Element) => on(target, "return e.textContent.trim()") as Promise<string>,
    value: (target: Element) => on(target, "return e.value") as Promise<string>,
    /** keys pressed where the focus is, one per entry, W3C key codes for the special ones */
    keys: async (keys: string[]) => {
      const actions = keys.flatMap((key) => [
        { type: "keyDown", value: key },
        { type: "keyUp", value: key },
      ]);
      await call("POST", `${session}/actions`, {
        actions: [{ type: "key", id: "keyboard", actions }],
      });
    },
    until,
    close: async () => {
      await call("DELETE", session).catch(() => {});
      // ending the session quits the app, this is for one that hangs, the whole tree since the app
      // starts helpers of its own
      try {
        execSync(`taskkill /PID ${child.pid} /T /F`, { stdio: "ignore" });
      } catch {
        // already gone
      }
      await until(
        () =>
          call("GET", "/status").then(
            () => false,
            () => true,
          ),
        "the app to close",
      );
    },
  };
}

export const test = base.extend<{ home: string }>({
  // eslint-disable-next-line no-empty-pattern
  home: async ({}, use) => use(profile()),
});
export { expect } from "@playwright/test";
