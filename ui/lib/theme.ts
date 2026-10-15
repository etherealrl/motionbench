import { useLayoutEffect, useSyncExternalStore } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { native } from "@/lib/native";
import type { Theme } from "@/types";

const systemLight = () => window.matchMedia("(prefers-color-scheme: light)");

/** puts the theme on the page and the window's title bar, and keeps following the system's while
 * it's set to. before the first paint, so a saved light theme never flashes dark */
export function useTheme(theme: Theme) {
  useLayoutEffect(() => {
    const apply = () =>
      document.documentElement.classList.toggle(
        "dark",
        theme === "dark" || (theme === "system" && !systemLight().matches),
      );
    apply();
    if (native) void titleBar(theme);
    if (theme !== "system") return;
    const query = systemLight();
    query.addEventListener("change", apply);
    return () => query.removeEventListener("change", apply);
  }, [theme]);
}

/** the native title bar only matches the page, so a window that can't take it keeps its own */
async function titleBar(theme: Theme) {
  try {
    await getCurrentWindow().setTheme(theme === "system" ? null : theme);
  } catch {
    // nothing to do, the page itself is already in the theme
  }
}

function subscribe(changed: () => void) {
  const observer = new MutationObserver(changed);
  observer.observe(document.documentElement, { attributeFilter: ["class"] });
  return () => observer.disconnect();
}

/** whether the page is dark right now, for what can't read the theme from css */
export function useDark() {
  return useSyncExternalStore(subscribe, () => document.documentElement.classList.contains("dark"));
}
