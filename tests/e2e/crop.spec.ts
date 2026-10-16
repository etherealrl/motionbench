import { expect, launch, test } from "./app";

const ARROW_RIGHT = "";

test("a crop is saved by native and still there after a restart", async ({ home }) => {
  let app = await launch(home);
  try {
    await app.click(await app.tab("Runs"));
    await app.click(await app.byText("Crop", "button"));
    const start = await app.byLabel("Crop start");
    await app.on(start, "e.focus()");
    // ten steps of 10 ms in from the start
    await app.keys(Array(10).fill(ARROW_RIGHT));
    await app.byText("From 0.10 s");
    await app.click(await app.byText("Save", "button"));
    await app.until(
      async () => (await app.run(`return document.body.innerText.includes("Cropped to")`)) === true,
      "the crop in the table",
    );
    await app.close();

    app = await launch(home);
    await app.click(await app.tab("Runs"));
    expect(
      await app.until(
        async () =>
          (await app.run(`return document.body.innerText.includes("Cropped to")`)) === true,
        "the crop after a restart",
      ),
    ).toBe(true);
  } finally {
    await app.close();
  }
});
