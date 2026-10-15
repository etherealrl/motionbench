import { Badge } from "@/components/ui/badge";
import type { Session } from "@/app/use-session";
import type { Capture } from "./use-capture";
import { SessionPanel } from "./session-panel";
import { CaptureSetup } from "./setup/capture-setup";
import { LiveView } from "./live-view";
import { Diagnostics, diagnosticsWarnings } from "./diagnostics";

export function CaptureView({ session, capture }: { session: Session; capture: Capture }) {
  const { snapshot, call } = session;
  return (
    <>
      <div className="section-intro">
        <h2>Capture</h2>
        <div className="status-strip">
          {diagnosticsWarnings(snapshot).map((reason) => (
            <Badge key={reason} variant="warning">
              {reason}
            </Badge>
          ))}
          <Diagnostics snapshot={snapshot} />
        </div>
      </div>
      <SessionPanel session={session} capture={capture} />
      <CaptureSetup
        snapshot={snapshot}
        call={call}
        busy={capture.busy}
        sections={capture.sections}
        onSectionsChange={capture.setSections}
      />
      <LiveView snapshot={snapshot} call={call} />
    </>
  );
}
