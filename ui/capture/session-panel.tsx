import { useState } from "react";
import { Circle, CircleStop, Play, Square } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { DeviceLabel } from "@/components/device-label";
import { native } from "@/lib/native";
import { buttonName } from "@/lib/mouse-buttons";
import type { Session } from "@/app/use-session";
import type { Snapshot } from "@/types";
import type { Capture } from "./use-capture";
import { blockerHint } from "@/lib/text";
import { recordingTime } from "./recording";
import { useSettings } from "@/lib/settings";
import { isCase } from "@/lib/sides";
import { SidePick } from "./comparing";

/** where reports come from, then recording them */
export function SessionPanel({ session, capture }: { session: Session; capture: Capture }) {
  return (
    <section className="panel capture-session" aria-label="Recording session">
      <SourceRow session={session} capture={capture} />
      {capture.decoderNotice && <p className="control-note">{capture.decoderNotice}</p>}
      <RecordRow session={session} capture={capture} />
    </section>
  );
}

function SourceRow({ session, capture }: { session: Session; capture: Capture }) {
  const { snapshot, roots, root } = session;
  const { busy, action } = capture;
  return (
    <div className="session-row">
      <label className="usb-root">
        USB root
        <Dropdown
          value={root}
          disabled={snapshot.live}
          onValueChange={capture.chooseRoot}
          placeholder={roots.length ? "Choose a root hub" : "USBPcap not available"}
        >
          {roots.map((r) => (
            <DropdownOption key={r}>{r}</DropdownOption>
          ))}
        </Dropdown>
      </label>
      <DevicePicker snapshot={snapshot} capture={capture} />
      <Button
        variant="outline"
        disabled={!native || busy || (!snapshot.live && !root)}
        onClick={() =>
          void (snapshot.live ? capture.stopLive() : action("start_capture", { root }))
        }
      >
        {snapshot.live ? <Square /> : <Play />}
        {snapshot.live ? "Stop monitoring" : "Start monitoring"}
      </Button>
    </div>
  );
}

// the mice, and whatever is selected even if it isn't one, so the pick never disappears
function mice(snapshot: Snapshot) {
  return snapshot.devices.filter((d) => d.kind === "mouse" || d.id === snapshot.selected);
}

function pickerLocked(snapshot: Snapshot, capture: Capture) {
  return !native || !snapshot.live || !!snapshot.recording || capture.busy;
}

/** the endpoint to record. only mice by default, since a keyboard or a headset is never the
 * point, with the rest a tick away for a mouse that reports itself as something else */
function DevicePicker({ snapshot, capture }: { snapshot: Snapshot; capture: Capture }) {
  const [everything, setEverything] = useState(false);
  const shown = everything ? snapshot.devices : mice(snapshot);
  return (
    <div className="device-picker">
      <div className="device-picker-head">
        <span id="mouse-endpoint">Mouse endpoint</span>
        <Label>
          <Checkbox
            checked={everything}
            onCheckedChange={(checked) => setEverything(checked === true)}
          />
          Other devices
        </Label>
      </div>
      <Dropdown
        aria-label="Mouse endpoint"
        value={snapshot.selected ?? ""}
        disabled={pickerLocked(snapshot, capture)}
        onValueChange={(value) => void capture.action("select_device", { id: value })}
        placeholder="Move your mouse, then select its endpoint"
      >
        {shown.map((d) => (
          <DropdownOption key={d.id} value={d.id}>
            <DeviceLabel device={d} />
          </DropdownOption>
        ))}
      </Dropdown>
    </div>
  );
}

// what stands in the way of recording, or how the bound box button starts it
function recordHint(snapshot: Snapshot) {
  if (!native) return "monitoring and recording only work in the desktop app";
  const blocker = blockerHint(snapshot.readiness.record);
  if (blocker) return blocker;
  if (snapshot.readiness.record) return null;
  const hotkey = snapshot.recording_hotkey;
  if (snapshot.recording || !hotkey?.ready || !hotkey.hotkey_mask) return null;
  return `${buttonName(hotkey.hotkey_mask)} starts and stops recording`;
}

function RecordRow({ session, capture }: { session: Session; capture: Capture }) {
  const { snapshot } = session;
  const { busy, action } = capture;
  const blocked = snapshot.readiness.record !== null;
  const purpose = useSettings().display.purpose;
  return (
    <div className="session-row record-row">
      {isCase(purpose) && <SidePick kind={purpose} disabled={!!snapshot.recording} />}
      <Button
        variant={!snapshot.recording && (!native || blocked) ? "outline" : "default"}
        disabled={!native || busy || blocked}
        onClick={() =>
          void (snapshot.recording
            ? capture.stopRecord()
            : action("recording_toggle", { recording: true }))
        }
      >
        {snapshot.recording ? <CircleStop /> : <Circle />}
        {snapshot.recording ? "Finish recording" : "Record"}
      </Button>
      {snapshot.recording && (
        <Badge variant="danger" aria-label="Recording duration">
          {recordingTime(snapshot.recording_elapsed_ms ?? 0)}
        </Badge>
      )}
      <p className="field-description" role="status">
        {recordHint(snapshot)}
      </p>
    </div>
  );
}
