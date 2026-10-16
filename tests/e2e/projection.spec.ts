import { expect, launch, saved, test, type App } from "./app";

const games = ["Counter-Strike 2", "Apex Legends", "Overwatch 2", "Valorant"];

// the driver's clicks are synthetic and now and then race the list opening or closing, so a pick
// is tried again until the game shows. a game that won't stay picked still fails every try
async function pick(app: App, name: string) {
  await app.until(
    async () => {
      const open = await app.run(`return document.querySelectorAll("[role=option]").length > 0`);
      if (!open) await app.click(await app.byLabel("Game"));
      const option = await app.until(
        () =>
          app.run(
            `return [...document.querySelectorAll("[role=option]")].some((e) => e.textContent.trim() === arguments[0])`,
            name,
          ),
        "the list",
        2000,
      );
      if (option) await app.click(await app.byText(name, "[role=option]"));
      return app.until(
        async () => (await app.text(await app.byLabel("Game"))) === name,
        name,
        2000,
      );
    },
    `${name} to be picked`,
    15_000,
  );
}

test("every game saves through native, and the game and its sensitivity survive a restart", async ({
  home,
}) => {
  let app = await launch(home);
  try {
    const section = await app.byText("Target capture", "button");
    if ((await app.on(section, `return e.getAttribute("aria-expanded")`)) !== "true")
      await app.click(section);
    // CS2 and Apex share their numbers, so going between them is the case that broke
    for (const name of [...games, "Apex Legends", "Counter-Strike 2", "Apex Legends"]) {
      await pick(app, name);
      // and it stays picked once the settings come back from native
      await new Promise((r) => setTimeout(r, 300));
      expect(await app.text(await app.byLabel("Game"))).toBe(name);
    }
    // native wrote it, which it only does for settings it accepts
    await app.until(() => Promise.resolve(saved(home).targets?.game === "apex"), "apex saved");
    expect(saved(home).vision).toMatchObject({ fov_axis: "horizontal_4x3", yaw: 0.022 });
    await app.fill(await app.byLabel("FOV"), "104");
    await app.fill(await app.byLabel("Sensitivity"), "1.25");
    await app.until(
      () => Promise.resolve(saved(home).vision.sensitivity === 1.25),
      "the typed sensitivity",
    );
    await app.click(await app.byLabel("Save the sensitivity for Apex Legends"));
    await app.until(
      () => Promise.resolve(saved(home).targets?.sensitivities?.apex === 1.25),
      "the sensitivity saved",
    );
    expect(saved(home).vision.fov).toBe(104);
    await app.close();

    app = await launch(home);
    const restored = await app.byText("Target capture", "button");
    if ((await app.on(restored, `return e.getAttribute("aria-expanded")`)) !== "true")
      await app.click(restored);
    expect(await app.text(await app.byLabel("Game"))).toBe("Apex Legends");
    expect(await app.value(await app.byLabel("Sensitivity"))).toBe("1.25");
    expect(await app.value(await app.byLabel("FOV"))).toBe("104");
  } finally {
    await app.close();
  }
});
