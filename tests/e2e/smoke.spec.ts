import { expect, launch, test } from "./app";

test("the app starts on a fresh profile", async ({ home }) => {
  const app = await launch(home);
  try {
    const title = await app.byText("Motion Bench", "h1");
    expect(await app.text(title)).toBe("Motion Bench");
    expect(await app.run(`return !!window.__TAURI_INTERNALS__`)).toBe(true);
    // the displays come from Windows, each by its number and size
    const section = await app.byText("Target capture", "button");
    if ((await app.on(section, `return e.getAttribute("aria-expanded")`)) !== "true")
      await app.click(section);
    await app.until(
      async () => /^Display \d+, \d+ × \d+/.test(await app.text(await app.byLabel("Display"))),
      "the display list",
    );
  } finally {
    await app.close();
  }
});
