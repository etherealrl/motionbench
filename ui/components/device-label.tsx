import { CircleHelp, Keyboard, Mouse, Usb } from "lucide-react";
import { Items } from "@/components/inline";
import { Badge } from "@/components/ui/badge";
import { hex } from "@/lib/plots";
import type { Snapshot } from "@/types";

// vendor and product ids, or null when the device did not say
export function deviceIds(device: Snapshot["devices"][number]) {
  return device.vid === null || device.pid === null
    ? null
    : `${hex(device.vid)}:${hex(device.pid)}`;
}

// what an endpoint carries: a capture only knows its interface class, and a box that
// clones a mouse with a keyboard on it files both under the class the clone reports. the kind stays
// the icon's name for a screen reader, never a word on the row, because a box is named after the
// device it clones
const kinds = {
  mouse: { Icon: Mouse, name: "mouse" },
  keyboard: { Icon: Keyboard, name: "keyboard" },
  unknown: { Icon: CircleHelp, name: "unknown device" },
};

function Kind({ kind }: { kind: string }) {
  const { Icon, name } = kinds[kind as keyof typeof kinds] ?? {
    Icon: Usb,
    name: `${kind} device`,
  };
  return (
    <span className="device-kind">
      <Icon role="img" aria-label={name} className="size-4 text-foreground" />
    </span>
  );
}

/** the name a person recognizes the device by, with the ids a hover away, since the evidence and
 * the descriptor backups are keyed by them. a device with neither shows nothing */
function Named({ device }: { device: Snapshot["devices"][number] }) {
  const ids = deviceIds(device);
  if (device.name) return <span title={ids ?? undefined}>{device.name}</span>;
  return ids ? <span>{ids}</span> : null;
}

/** the box this mouse goes through, by its kind and its own name. a Medius or a Ferrum says
 * what's plugged into it, any other box shows up here once a click on it reaches the endpoint */
function BoxBadge({ name }: { name: string }) {
  return <Badge variant="info">{name}</Badge>;
}

export function DeviceLabel({ device }: { device: Snapshot["devices"][number] }) {
  return (
    <Items>
      <Kind kind={device.kind} />
      <Named device={device} />
      <span className="device-where">
        bus {device.bus} / {device.address} / 0x{device.endpoint.toString(16)}
      </span>
      {device.box_name && <BoxBadge name={device.box_name} />}
    </Items>
  );
}
