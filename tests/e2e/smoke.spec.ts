import { expect, launch, test } from "./app";

test("the app starts on a fresh profile", async ({ home }) => {
  const app = await launch(home);
  try {
    const title = await app.byText("Motion Bench", "h1");
    expect(await app.text(title)).toBe("Motion Bench");
    expect(await app.run(`return !!window.__TAURI_INTERNALS__`)).toBe(true);
  } finally {
    await app.close();
  }
});
