import type { ReactNode } from "react";
import { CircleAlert, TriangleAlert } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";

/** a message the user should read: a warning, or an error in red */
export function Notice({ error = false, children }: { error?: boolean; children: ReactNode }) {
  return (
    <Alert variant={error ? "destructive" : "warning"} role={error ? "alert" : "status"}>
      {error ? <CircleAlert /> : <TriangleAlert />}
      <AlertDescription>{children}</AlertDescription>
    </Alert>
  );
}
