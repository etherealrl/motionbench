import { useEffect, useRef } from "react";
import { RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Badge, type BadgeTone } from "@/components/ui/badge";
import { Notice } from "@/components/notice";
import { SettingRow, SettingSection } from "@/components/setting-row";
import { SetupSection } from "@/components/setup-section";
import type { Session } from "@/app/use-session";
import { fixHint } from "@/lib/text";
import type { BoxState, Snapshot } from "@/types";
import { RecordingButton } from "./recording-button";

export type Send = (command: Record<string, unknown>) => Promise<void>;

export function useSend(call: Session["call"]): Send {
  return async (command) => {
    await call("box_command", { command });
  };
}

/** what the box is called, its family first: a Medius goes by the name it was given, the others
 * by their model, which already starts with the family */
export function boxName(b: BoxState) {
  return b.name.startsWith(b.family) ? b.name : `${b.family} ${b.name}`.trim();
}

/** what the link is doing, in a short badge for the folded header; nothing while there's no box */
export function boxState(b: BoxState): { label: string; tone: BadgeTone } | null {
  if (b.error || (b.link === "connected" && b.has_mouse === false))
    return { label: "Needs attention", tone: "danger" };
  if (b.link !== "connected") return null;
  return { label: `${b.family} ${b.port ?? ""}`.trim(), tone: "success" };
}

/** every input the box said it lost, on any of its links */
export function boxLosses(b: BoxState) {
  const h = b.medius?.health;
  return b.losses + (h ? h.link_drops + h.host_drops + h.tx_drops : 0);
}

/** the box, whichever one it is: which one, and what it adds to a recording */
export function BoxSection({
  snapshot,
  switching,
  native,
  busy,
  call,
  open,
  onOpenChange,
}: {
  snapshot: Snapshot;
  /** the box shown is the one being left, until the next one is ready */
  switching: boolean;
  native: boolean;
  busy: boolean;
  call: Session["call"];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const b = snapshot.box;
  const send = useSend(call);
  const state = boxState(b);
  const connected = b.link === "connected";
  useStandInToast(b, send);
  return (
    <SetupSection
      title="Box"
      open={open}
      onOpenChange={onOpenChange}
      status={state && <Badge variant={state.tone}>{state.label}</Badge>}
      hint={fixHint(snapshot.readiness.box_connect) ?? undefined}
      action={
        <BoxActions
          box={b}
          switching={switching}
          disabled={!native || snapshot.readiness.box_connect !== null}
          send={send}
        />
      }
    >
      <Notices box={b} />
      {connected ? (
        <>
          <Identity box={b} />
          <SettingSection title="Recording">
            <div className="setting-group">
              <RecordingButton
                snapshot={snapshot}
                native={native}
                busy={busy}
                quiet={switching}
                command={call}
              />
              <PhysicalInput
                box={b}
                disabled={!native || snapshot.readiness.box_settings !== null}
                send={send}
              />
            </div>
          </SettingSection>
          <Losses box={b} />
        </>
      ) : (
        !b.error &&
        !b.unusable?.length && (
          <p className="field-description">
            a box is what lets a mouse button start and stop your recordings, and it's what the aim
            controller moves the mouse through. plug one in and it connects by itself
          </p>
        )
      )}
    </SetupSection>
  );
}

/** which box this is, above its settings whatever the width, so it sits in one place on every
 * box. the header badge and the picker already say its port */
function Identity({ box: b }: { box: BoxState }) {
  return (
    <SettingSection title="Connected box">
      <dl className="readout">
        <dt>Box</dt>
        <dd>{boxName(b)}</dd>
        <dt>Firmware</dt>
        <dd>{b.firmware || "-"}</dd>
      </dl>
    </SettingSection>
  );
}

// it changes what a recording holds, so a box that can't give it says why in its place
function PhysicalInput({
  box: b,
  disabled,
  send,
}: {
  box: BoxState;
  disabled: boolean;
  send: Send;
}) {
  const why = b.features.physical_input;
  return (
    <SettingRow
      id={why ? undefined : "record-physical-input"}
      label="Record physical input"
      description={
        why ??
        "records your mouse's movement before the box renders it, so you can compare what goes in with what comes out"
      }
    >
      {!why && (
        <Switch
          id="record-physical-input"
          disabled={disabled}
          checked={b.watch}
          onCheckedChange={(enabled) => void send({ action: "watch", enabled })}
        />
      )}
    </SettingRow>
  );
}

function Losses({ box: b }: { box: BoxState }) {
  const h = b.medius?.health;
  return (
    <SettingSection title="Losses">
      <dl className="readout">
        <dt>Input</dt>
        <dd>{b.losses}</dd>
        {h && (
          <>
            <dt>Link</dt>
            <dd>{h.link_drops}</dd>
            <dt>Host</dt>
            <dd>{h.host_drops}</dd>
            <dt>TX</dt>
            <dd>{h.tx_drops}</dd>
          </>
        )}
      </dl>
    </SettingSection>
  );
}

/** which box, and connecting or letting go of it */
function BoxActions({
  box: b,
  switching,
  disabled,
  send,
}: {
  box: BoxState;
  switching: boolean;
  disabled: boolean;
  send: Send;
}) {
  const connected = b.link === "connected";
  return (
    <div className="box-actions">
      <BoxPicker box={b} disabled={disabled} send={send} />
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Look for boxes again"
            disabled={disabled}
            onClick={() => void send({ action: "refresh" })}
          >
            <RefreshCw />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Look for boxes again</TooltipContent>
      </Tooltip>
      <Button
        variant="outline"
        size="sm"
        aria-label={connected ? "Disconnect box" : "Connect box"}
        disabled={disabled}
        onClick={() => void send({ action: connected ? "disconnect" : "connect" })}
      >
        {switching || b.link === "connecting"
          ? "Connecting…"
          : connected
            ? "Disconnect"
            : "Connect"}
      </Button>
    </div>
  );
}

/** which box to use, out of every box the last scan saw */
function BoxPicker({ box: b, disabled, send }: { box: BoxState; disabled: boolean; send: Send }) {
  return (
    <Dropdown
      aria-label="Which box"
      size="sm"
      value={b.key ?? ""}
      disabled={disabled}
      onValueChange={(key) => void send({ action: "pick", key })}
      placeholder={b.picking ? "Pick a box" : "No box"}
    >
      {(b.boxes ?? []).map((row) => (
        <DropdownOption key={row.key} value={row.key} disabled={!row.usable}>
          {row.model ? `${row.label} ${row.model}` : row.label}
        </DropdownOption>
      ))}
    </Dropdown>
  );
}

/** the box used last time isn't there and another stands in: a warning that stays until it's
 * closed, with the stand-in one click from being the default */
function useStandInToast(b: BoxState, send: Send) {
  const shown = useRef<string | null>(null);
  const notice = b.notice ?? null;
  useEffect(() => {
    if (notice === shown.current) return;
    shown.current = notice;
    if (!notice) return;
    toast.warning("your usual box isn't plugged in", {
      description: notice,
      id: "box-stand-in",
      duration: Infinity,
      closeButton: true,
      action: {
        label: "Use this box as default",
        onClick: () => void send({ action: "pick", key: b.key }),
      },
    });
  }, [notice]);
  useEffect(() => {
    if (b.key && b.key === b.remembered) toast.dismiss("box-stand-in");
  }, [b.key, b.remembered]);
}

function Notices({ box: b }: { box: BoxState }) {
  return (
    <>
      {b.error && <Notice error>{b.error}</Notice>}
      {b.link === "connected" && b.has_mouse === false && (
        <Notice error>there's no mouse behind the box, plug one into it</Notice>
      )}
      {(b.unusable ?? []).map((row) => (
        <p key={row.port} className="field-description">
          {row.port} can't be used, {row.why}
        </p>
      ))}
    </>
  );
}
