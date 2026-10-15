import type { RunStatus } from "@/types";

/** how a recording ended, as a short badge */
export const outcome: Record<
  RunStatus["kind"],
  { label: string; tone: "success" | "warning" | "danger" }
> = {
  complete: { label: "Complete", tone: "success" },
  degraded: { label: "Degraded", tone: "warning" },
  interrupted: { label: "Interrupted", tone: "warning" },
  failed: { label: "Failed", tone: "danger" },
};
