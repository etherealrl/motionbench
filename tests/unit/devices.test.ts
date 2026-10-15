import { expect, test } from "vite-plus/test";
import { matchingMouse } from "@/lib/devices";
import type { Device } from "@/types";
const mouse = { vid: 1234, pid: 5678, bus: 1, endpoint: 129 };
const device = {
  ...mouse,
  id: "1:47:129",
  address: 47,
  kind: "Mouse",
  interval: 1,
  decoder: "auto",
  reports: 0,
} as Device;
test("saved mouse matches a new USB address", () => {
  expect(matchingMouse([device], mouse)?.id).toBe("1:47:129");
});
test("missing or ambiguous identities never select another mouse", () => {
  expect(matchingMouse([{ ...device, vid: 999 }], mouse)).toBeNull();
  expect(matchingMouse([device, { ...device, id: "1:48:129", address: 48 }], mouse)).toBeNull();
  expect(matchingMouse([device], { ...mouse, vid: null, pid: null })).toBeNull();
});
