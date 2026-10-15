import { useState, type ReactNode } from "react";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { Button } from "@/components/ui/button";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Items } from "@/components/inline";
import { Notice } from "@/components/notice";
import { SettingRow, SettingSection } from "@/components/setting-row";
import { SetupSection } from "@/components/setup-section";
import { Summary } from "@/components/summary";
import type { Session } from "@/app/use-session";
import { fixHint } from "@/lib/text";
import type { MediusState, Snapshot } from "@/types";
import { useSend, type Send } from "./box";

const renderModes = {
  off: "Off",
  stock: "Stock",
  despiked: "Despiked",
  unsmoothed: "Unsmoothed",
};
const emitPaces = { learned: "Learned", interval: "Interval", fixed: "Fixed" };

// the box renders your movement only once a profile has armed, and only movement through the box
// arms one: the single render state a user can still do something about. with your movement
// relayed, nothing of yours waits on it
function profilePending(m: MediusState) {
  return m.render.full && m.render.mode !== "off" && !m.render.ready;
}

/** why the box passes the hand's motion through unchanged, or null while it renders it. the mode
 * being off is a setting, not a fault, and so is relaying physical movement */
function relayReason(m: MediusState) {
  if (m.render.mode === "off")
    return "the render mode is off, so the box passes your movement through unchanged";
  if (!profilePending(m) && m.render.effective === "relay")
    return "Render physical movement is off, so your mouse's own movement passes through unchanged";
  return null;
}

// the box's render settings as the user edits them, kept while the section is folded
type Draft = {
  mode: string;
  full: boolean;
  pace: string;
  hz: number;
  force: string;
  spread: number;
  riding: string;
  imperfect: boolean;
};

function readback(f: MediusState): Draft {
  return {
    mode: f.render.mode === "unknown" ? "despiked" : f.render.mode,
    full: f.render.full,
    pace: f.emit.mode === "unknown" ? "learned" : f.emit.mode,
    hz: f.emit.fixed_hz ?? 1000,
    force: f.emit.force_hz?.toString() ?? "",
    spread: f.spread.percent,
    riding: f.riding_ms?.toString() ?? "",
    imperfect: f.imperfect_allowed,
  };
}

// the section mounts with the box, so a box that comes back starts from its own readback
function useDraft(f: MediusState) {
  const [draft, setDraft] = useState(() => readback(f));
  const set = (patch: Partial<Draft>) => setDraft((old) => ({ ...old, ...patch }));
  const changed = JSON.stringify(draft) !== JSON.stringify(readback(f));
  return { draft, set, changed };
}

/** a Medius's own settings, the renderer, beside the box section */
export function MediusSection({
  medius: m,
  snapshot,
  native,
  call,
  open,
  onOpenChange,
}: {
  medius: MediusState;
  snapshot: Snapshot;
  native: boolean;
  call: Session["call"];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const disabled = !native || snapshot.readiness.box_settings !== null;
  const box = useDraft(m);
  const send = useSend(call);
  const reason = relayReason(m);
  return (
    <SetupSection
      title="Medius"
      open={open}
      onOpenChange={onOpenChange}
      status={profilePending(m) && <Badge variant="warning">Relay only</Badge>}
      hint={fixHint(snapshot.readiness.box_settings) ?? undefined}
      aside={<Readback medius={m} />}
    >
      {profilePending(m) && (
        <Notice>
          the renderer isn't active yet, move the mouse through the box until its profile is ready
        </Notice>
      )}
      {reason && <p className="field-description">{reason}</p>}
      <BoxSettings box={box} send={send} disabled={disabled} />
    </SetupSection>
  );
}

// what the box reports it is doing, which can differ from the settings until they are applied
function Readback({ medius: f }: { medius: MediusState }) {
  const rendered = f.render.effective === "rendered";
  const profileTone = f.render.ready ? "success" : f.render.mode === "off" ? "muted" : "warning";
  return (
    <SettingSection title="Reported by the box">
      <dl className="readout">
        <dt>Output</dt>
        <dd>
          <Badge variant={rendered ? "success" : profilePending(f) ? "warning" : "muted"}>
            {rendered ? "Rendered" : "Relay only"}
          </Badge>
        </dd>
        <dt>Profile</dt>
        <dd>
          <Badge variant={profileTone}>{f.render.ready ? "Ready" : "Not ready"}</Badge>
        </dd>
      </dl>
    </SettingSection>
  );
}

// the box's render settings, a draft until Apply sends them
function BoxSettings({
  box: { draft, set, changed },
  send,
  disabled,
}: {
  box: ReturnType<typeof useDraft>;
  send: Send;
  disabled: boolean;
}) {
  const [pending, setPending] = useState(false);
  const apply = async () => {
    setPending(true);
    try {
      await send({ action: "render", mode: draft.mode, full: draft.full });
      await send({
        action: "emit",
        mode: draft.pace,
        hz: draft.hz,
        force: draft.force ? Number(draft.force) : null,
      });
      await send({ action: "spread", percent: draft.spread });
      await send({
        action: "riding",
        milliseconds: draft.riding ? Number(draft.riding) : null,
      });
      await send({ action: "imperfect", enabled: draft.imperfect });
    } finally {
      setPending(false);
    }
  };
  return (
    <SettingSection title="Rendering">
      <fieldset disabled={disabled || pending} className="setting-group">
        <SettingRow
          id="render-mode"
          label="Render mode"
          description="how injected motion gets shaped. Off sends it as an even fill, the others follow the report pattern of the mouse you're moving"
        >
          <Dropdown id="render-mode" value={draft.mode} onValueChange={(mode) => set({ mode })}>
            {Object.entries(renderModes).map(([value, label]) => (
              <DropdownOption key={value} value={value}>
                {label}
              </DropdownOption>
            ))}
          </Dropdown>
        </SettingRow>
        <SettingRow
          id="render-physical"
          label="Render physical movement"
          description="sends your mouse's own movement through the renderer too, though that adds a little latency to it"
        >
          <Switch
            id="render-physical"
            checked={draft.full}
            onCheckedChange={(full) => set({ full })}
          />
        </SettingRow>
        <details className="advanced-settings">
          <Summary>Timing and compatibility</Summary>
          <TimingSettings draft={draft} set={set} />
        </details>
        <div className="setting-actions">
          {changed && <span className="field-description">Not applied</span>}
          <Button variant="outline" disabled={!changed || pending} onClick={() => void apply()}>
            {pending ? "Sending settings…" : "Apply settings"}
          </Button>
        </div>
      </fieldset>
    </SettingSection>
  );
}

function TimingSettings({ draft, set }: { draft: Draft; set: (patch: Partial<Draft>) => void }) {
  return (
    <div className="setting-group">
      <SettingRow
        id="emit-pace"
        label="Emit pace"
        description="paces injected motion either to the report rate the mouse is measured at (Learned), the poll rate it declares (Interval), or a fixed rate"
      >
        <Dropdown id="emit-pace" value={draft.pace} onValueChange={(pace) => set({ pace })}>
          {Object.entries(emitPaces).map(([value, label]) => (
            <DropdownOption key={value} value={value}>
              {label}
            </DropdownOption>
          ))}
        </Dropdown>
      </SettingRow>
      {draft.pace === "fixed" && (
        <SettingRow id="fixed-rate" label="Fixed emit rate">
          <Unit unit="Hz">
            <InputGroupInput
              id="fixed-rate"
              type="number"
              min="1"
              max="8000"
              value={draft.hz}
              onChange={(e) => set({ hz: Number(e.target.value) })}
            />
          </Unit>
        </SettingRow>
      )}
      <SettingRow
        id="forced-rate"
        label="Forced wire rate"
        description="leave it empty to keep the mouse's own rate. it needs imperfect clones allowed, and changing it reconnects the box"
      >
        <Unit unit="Hz">
          <InputGroupInput
            id="forced-rate"
            type="number"
            min="1"
            max="8000"
            value={draft.force}
            onChange={(e) => set({ force: e.target.value })}
          />
        </Unit>
      </SettingRow>
      <SettingRow
        id="spread"
        label="Spread"
        description="how much of the command interval each injected move gets spread across, 0 just sends it all at once"
      >
        <Unit unit="%">
          <InputGroupInput
            id="spread"
            type="number"
            min="0"
            max="100"
            value={draft.spread}
            onChange={(e) => set({ spread: Number(e.target.value) })}
          />
        </Unit>
      </SettingRow>
      <SettingRow
        id="riding"
        label="Movement riding"
        description="injected motion waits up to this long for you to physically move the mouse, leave it empty to turn that off"
      >
        <Unit unit="ms">
          <InputGroupInput
            id="riding"
            type="number"
            min="1"
            max="60000"
            value={draft.riding}
            onChange={(e) => set({ riding: e.target.value })}
          />
        </Unit>
      </SettingRow>
      <SettingRow
        id="imperfect"
        label="Allow imperfect clones"
        description="lets the box clone a mouse it can't copy exactly, and that's also what allows forcing the wire rate"
      >
        <Switch
          id="imperfect"
          checked={draft.imperfect}
          onCheckedChange={(imperfect) => set({ imperfect })}
        />
      </SettingRow>
    </div>
  );
}

function Unit({ unit, children }: { unit: string; children: ReactNode }) {
  return (
    <InputGroup>
      {children}
      <InputGroupAddon align="inline-end">{unit}</InputGroupAddon>
    </InputGroup>
  );
}

/** what only a Medius says about itself, for the diagnostics */
export function MediusDetails({ medius: f }: { medius: MediusState }) {
  return (
    <dl className="readout">
      {f.clone && (
        <>
          <dt>Mouse</dt>
          <dd>{f.clone}</dd>
        </>
      )}
      <dt>Firmware</dt>
      <dd>
        <Items>
          <span>
            v{f.firmware.device_major}.{f.firmware.device_minor}.{f.firmware.device_patch}
          </span>
          <span>protocol {f.firmware.protocol}</span>
        </Items>
      </dd>
      <dt>Emit</dt>
      <dd>
        <Items>
          <span>{emitPaces[f.emit.mode as keyof typeof emitPaces] ?? f.emit.mode}</span>
          <span>{f.emit.resolved_hz} Hz</span>
        </Items>
      </dd>
      <dt>Advertised rate</dt>
      <dd>{f.emit.advertised_hz} Hz</dd>
      <dt>Spread</dt>
      <dd>
        <Items>
          <span>{f.spread.percent}%</span>
          <span>{f.spread.span_us} µs</span>
        </Items>
      </dd>
      <dt>Movement riding</dt>
      <dd>{f.riding_ms === null ? "Off" : `${f.riding_ms} ms`}</dd>
    </dl>
  );
}
