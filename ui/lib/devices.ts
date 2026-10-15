import type { Device, SavedMouse } from "@/types";

export function matchingMouse(devices: Device[], saved: SavedMouse | null) {
  if (!saved || saved.vid === null || saved.pid === null) return null;
  const matches = devices.filter(
    (d) =>
      d.vid === saved.vid &&
      d.pid === saved.pid &&
      d.endpoint === saved.endpoint &&
      d.bus === saved.bus,
  );
  return matches.length === 1 ? matches[0] : null;
}
