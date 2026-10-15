import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { flushSettings, onSettingsError } from "@/lib/settings";
import { frameMailbox } from "@/lib/frame-mailbox";
import { native } from "@/lib/native";
import type { Frame, Listed, Run, Snapshot } from "@/types";
import { snapshot as preview } from "@/fixture";

const empty: Snapshot = structuredClone(preview);
empty.devices = [];
empty.selected = null;
empty.runs = [];
empty.plot = {
  ...empty.plot,
  reports: 0,
  readouts: { ...empty.plot.readouts, rate_hz: 0 },
  timeline: [],
  timeline_start_us: 0,
  timeline_step_us: 0,
  tabs: { timing: null, movement: null },
};

/** an error as a toast that stays until it is closed; the same message twice shows once */
function setError(message: string | null, action?: { label: string; onClick: () => void }) {
  if (message)
    toast.error(message, {
      id: message,
      duration: Infinity,
      closeButton: true,
      action,
    });
}

/** an armed aim that lost its box, and the box coming back, share one toast */
function useAwayToast(snapshot: Snapshot) {
  const away = !!snapshot.experiment?.away;
  const armed = !!snapshot.experiment?.armed;
  const was = useRef(false);
  useEffect(() => {
    if (away === was.current) return;
    was.current = away;
    if (away)
      toast.warning("the box or its mouse went away, aiming picks up again once they're back", {
        id: "aim-away",
      });
    else if (armed)
      toast.success("the box is back, press the aim button to aim again", {
        id: "aim-away",
      });
  }, [away, armed]);
}

/** the native side as the ui sees it: snapshots, USB roots, the library, command errors */
export function useSession() {
  const [libraryReady, setLibraryReady] = useState(!native);
  const removedRuns = useRef(new Set<string>());
  const pendingImports = useRef(new Map<string, Run>());
  // the library's runs, read only when its revision moves, and the list shown from them, which
  // stays the same array from frame to frame until either changes
  const listed: Run[] = native ? [] : preview.runs;
  const listing = useRef({
    revision: -1,
    runs: listed,
    shown: listed,
    asking: false,
  });
  const reshow = () => {
    const runs = listing.current.runs.filter((run) => !removedRuns.current.has(run.id));
    for (const [id, run] of pendingImports.current) {
      if (runs.some((r) => r.id === id)) pendingImports.current.delete(id);
      else runs.push(run);
    }
    listing.current.shown = runs;
  };
  const [snapshot, setSnapshot] = useState<Snapshot>(native ? empty : preview);
  const [roots, setRoots] = useState<string[]>([]),
    [root, setRoot] = useState("");
  const call = useCallback(async <T>(command: string, args?: Record<string, unknown>) => {
    try {
      return await invoke<T>(command, args);
    } catch (e) {
      setError(String(e));
      return null;
    }
  }, []);
  useEffect(() => {
    if (!native) return;
    let active = true;
    const readRuns = async () => {
      if (listing.current.asking) return;
      listing.current.asking = true;
      try {
        const listed = await invoke<Listed>("runs");
        if (!active) return;
        listing.current.revision = listed.revision;
        listing.current.runs = listed.runs;
        reshow();
        const runs = listing.current.shown;
        setSnapshot((previous) => ({ ...previous, runs }));
      } catch (e) {
        if (active) setError(String(e));
      } finally {
        listing.current.asking = false;
      }
    };
    const channel = new Channel<Frame>();
    const frames = frameMailbox<Frame>((value) => {
      if (active) {
        setSnapshot({ ...value, runs: listing.current.shown });
        if (value.library !== listing.current.revision) void readRuns();
      }
      void invoke("acknowledge", { seq: value.seq });
    });
    channel.onmessage = (frame) => frames.push(frame);
    void invoke<{
      roots: string[];
      root?: string;
      snapshot: Frame;
      folder: string;
      warnings?: string[];
      library_warnings?: string[];
    }>("initialize")
      .then(async (value) => {
        if (active) {
          setRoots(value.roots);
          setRoot(value.root ?? value.roots[0] ?? "");
          setSnapshot({ ...value.snapshot, runs: listing.current.shown });
          // the saved picks are checked against the runs once the library is ready, so the runs
          // come first or every pick would look gone
          await readRuns();
          if (!active) return;
          setLibraryReady(true);
          if (value.warnings?.length) setError(value.warnings.join("\n"));
          if (value.library_warnings?.length)
            setError(value.library_warnings.join("\n"), {
              label: "Open folder",
              onClick: () => void invoke("open_folder").catch((e) => setError(String(e))),
            });
          void invoke("subscribe", { channel }).catch((e) => {
            if (active) setError(String(e));
          });
        }
      })
      .catch((e) => {
        if (active) setError(String(e));
      });
    return () => {
      active = false;
      frames.dispose();
    };
  }, []);
  useEffect(() => onSettingsError(setError), []);
  useAwayToast(snapshot);
  useEffect(() => {
    if (snapshot.settings_unsaved) setError(snapshot.settings_unsaved);
  }, [snapshot.settings_unsaved]);
  useEffect(() => {
    if (!native) return;
    const window = getCurrentWebviewWindow();
    const off = window.onCloseRequested(async () => {
      await flushSettings();
    });
    return () => {
      void off.then((unsubscribe) => unsubscribe());
    };
  }, []);
  // a run imported or deleted here shows at once, until the native snapshots agree
  const addRun = (run: Run) => {
    removedRuns.current.delete(run.id);
    pendingImports.current.set(run.id, run);
    reshow();
    const runs = listing.current.shown;
    setSnapshot((previous) => ({ ...previous, runs }));
  };
  const dropRun = (id: string) => {
    removedRuns.current.add(id);
    pendingImports.current.delete(id);
    reshow();
    const runs = listing.current.shown;
    setSnapshot((previous) => ({ ...previous, runs }));
  };
  return {
    snapshot,
    roots,
    root,
    setRoot,
    libraryReady,
    setError,
    call,
    addRun,
    dropRun,
  };
}

export type Session = ReturnType<typeof useSession>;
