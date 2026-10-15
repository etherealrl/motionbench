import { useEffect, useRef, useState } from "react";
import { MouseBinding } from "@/components/mouse-binding";
import { SettingRow } from "@/components/setting-row";
import { Notice } from "@/components/notice";
import { fixHint } from "@/lib/text";
import type { Snapshot } from "@/types";

type Command = (name: string, args?: Record<string, unknown>) => Promise<unknown>;

const IDLE = "starts and stops a recording so you don't have to leave the game";

function hint(snapshot: Snapshot) {
  const status = snapshot.recording_hotkey;
  if (status?.learning)
    return status.ready
      ? "press and release the button you want to use, on the mouse plugged into the box"
      : "Waiting for the box…";
  const { recording_button, clear_recording_button } = snapshot.readiness;
  return fixHint(recording_button ?? clear_recording_button) ?? status?.message ?? IDLE;
}

// picking listens for the next press; leaving the view mid-pick cancels it, so no press is
// taken later without the user watching
function usePicker(command: Command, learning: boolean) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const choosing = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    choosing.current = learning;
  }, [learning]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (choosing.current) void command("recording_hotkey_learn", { cancel: true });
    };
  }, [command]);
  async function pick() {
    setPending(true);
    setError(null);
    try {
      const result = await command("recording_hotkey_learn", {
        cancel: learning,
      });
      if (result === null) {
        setError("couldn't set the recording button");
        return;
      }
      choosing.current = !learning;
      if (!mounted.current && !learning) {
        choosing.current = false;
        await command("recording_hotkey_learn", { cancel: true });
      }
    } finally {
      setPending(false);
    }
  }
  return { pending, error, pick };
}

/** the button on the mouse behind the box that starts and stops a recording, chosen by pressing it */
export function RecordingButton({
  snapshot,
  native,
  busy,
  quiet = false,
  command,
}: {
  snapshot: Snapshot;
  native: boolean;
  busy: boolean;
  /** one box is giving way to another, so the moment with none in between says nothing */
  quiet?: boolean;
  command: Command;
}) {
  const status = snapshot.recording_hotkey;
  const learning = !!status?.learning;
  const picker = usePicker(command, learning);
  const held = !native || busy || picker.pending;
  const { recording_button, clear_recording_button } = snapshot.readiness;
  const problem = picker.error || (!quiet && status?.error);
  return (
    <section aria-label="Recording mouse button">
      <SettingRow
        label="Recording button"
        description={<span role="status">{quiet ? IDLE : hint(snapshot)}</span>}
      >
        <MouseBinding
          label="Recording button"
          mask={status?.hotkey_mask}
          learning={learning}
          disabled={held || recording_button !== null}
          clearDisabled={held || clear_recording_button !== null}
          onPick={() => void picker.pick()}
          onClear={() => void command("recording_hotkey_disable")}
        />
      </SettingRow>
      {problem && <Notice error>{problem}</Notice>}
    </section>
  );
}
