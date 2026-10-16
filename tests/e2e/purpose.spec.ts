import { expect, launch, saved, test } from "./app";

test("the assist case is picked once, saved through native and still there after a restart", async ({
  home,
}) => {
  let app = await launch(home);
  try {
    await app.click(await app.byText("Native against an assist program", "button"));
    await app.until(
      () => Promise.resolve(saved(home).display?.purpose === "assist"),
      "the purpose saved",
    );
    await app.click(await app.byLabel("Side"));
    await app.click(await app.byText("With program", "[role=option]"));
    await app.until(
      () => Promise.resolve(saved(home).recording?.side === "candidate"),
      "the side saved",
    );
    await app.close();

    app = await launch(home);
    await app.byText("Native against an assist program", "h3");
    expect(await app.text(await app.byLabel("Side"))).toBe("With program");
    const prompts = await app.run(
      `return [...document.querySelectorAll("h3")].filter((e) => e.textContent === "What are you comparing?").length`,
    );
    expect(prompts).toBe(0);
  } finally {
    await app.close();
  }
});
