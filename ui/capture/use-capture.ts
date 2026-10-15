import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { matchingMouse } from "@/lib/devices";
import { loadSettings, updateSettings, useSettings } from "@/lib/settings";
import { native } from "@/lib/native";
import type { Session } from "@/app/use-session";
import type { SetupSections } from "@/types";

/** native actions and their busy flag; also chooses the saved mouse again when it shows up */
export function useCapture({ snapshot, root, setRoot, libraryReady, call, setError }: Session) {
  const settings = useSettings();
  const restoredMouse = useRef<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [decoderNotice, setDecoderNotice] = useState<string | null>(null);
  const sections = settings.display.sections;
  const setSections = (sections: SetupSections) =>
    updateSettings({ display: { ...settings.display, sections } });
  const action = async (command: string, args?: Record<string, unknown>) => {
    setBusy(true);
    try {
      const response = await invoke<string | null>(command, args);
      if (command === "select_device") setDecoderNotice(response);
      // native saved the root and the mouse, the one to restore next time
      if (command === "select_device" || command === "start_capture") await loadSettings();
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    if (!snapshot.live) restoredMouse.current = null;
    if (
      !native ||
      !libraryReady ||
      !snapshot.live ||
      snapshot.selected ||
      snapshot.recording ||
      busy ||
      root !== settings.capture.root
    )
      return;
    const device = matchingMouse(snapshot.devices, settings.capture.mouse);
    if (!device || restoredMouse.current === device.id) return;
    restoredMouse.current = device.id;
    void action("select_device", { id: device.id });
  }, [
    libraryReady,
    snapshot.live,
    snapshot.selected,
    snapshot.recording,
    snapshot.devices,
    busy,
    root,
    settings.capture,
  ]);
  const chooseRoot = (value: string) => {
    // saved once capture starts on it
    setRoot(value);
    restoredMouse.current = null;
  };
  const stopRecord = async () => {
    setBusy(true);
    try {
      await call<string>("recording_toggle", { recording: false });
    } finally {
      setBusy(false);
    }
  };
  // a recording it ends is listed in Runs natively, with its target events
  const stopLive = async () => {
    setBusy(true);
    try {
      await call<string | null>("stop_capture");
    } finally {
      setBusy(false);
    }
  };
  return {
    busy,
    action,
    chooseRoot,
    stopRecord,
    stopLive,
    decoderNotice,
    sections,
    setSections,
    // arming needs both, the box for the moves and target capture for the target
    openSetup: () => setSections({ targets: true, box: true, box_settings: true }),
  };
}

export type Capture = ReturnType<typeof useCapture>;
