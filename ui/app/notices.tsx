import { CircleAlert, LoaderCircle, TriangleAlert } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import type { Snapshot } from "@/types";

/** what holds while it holds: the library loading and capture's health. errors from commands
 * are toasts */
export function Notices({
  health,
  libraryReady,
}: {
  health: Snapshot["health"];
  libraryReady: boolean;
}) {
  return (
    <div className="notices">
      {!libraryReady && (
        <Alert role="status">
          <LoaderCircle />
          <AlertDescription>Loading saved recordings…</AlertDescription>
        </Alert>
      )}
      {health.error && (
        <Alert variant="destructive">
          <CircleAlert />
          <AlertDescription>{health.error}</AlertDescription>
        </Alert>
      )}
      {!health.error && health.notice && (
        <Alert variant="warning" role="status">
          <TriangleAlert />
          <AlertDescription>{health.notice}</AlertDescription>
        </Alert>
      )}
    </div>
  );
}
