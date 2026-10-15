import { useEffect, useRef } from "react";
import { native } from "@/lib/native";
import type { SetupSections } from "@/types";
import type { Session } from "@/app/use-session";
import type { BoxState, Snapshot } from "@/types";
import { TargetSection } from "./vision";
import { BoxSection, boxState } from "./box";
import { MediusSection } from "./medius";

/** while one box gives way to another, the one being left stays on screen, so the sections
 * don't collapse for the moment in between */
function useSteadyBox(box: BoxState) {
  const last = useRef<BoxState | null>(null);
  const changing = box.link === "connecting";
  if (box.link === "connected") last.current = box;
  else if (!changing) last.current = null;
  const switching = box.link !== "connected" && changing && !!last.current;
  return { box: switching ? last.current! : box, switching };
}

/** the optional parts of a recording, each a section that folds */
export function CaptureSetup({
  snapshot,
  call,
  busy,
  sections,
  onSectionsChange,
}: {
  snapshot: Snapshot;
  call: Session["call"];
  busy: boolean;
  sections: SetupSections;
  onSectionsChange: (sections: SetupSections) => void;
}) {
  const targetsFailed = !!snapshot.vision?.error;
  const boxFailed = boxState(snapshot.box)?.tone === "danger";
  // a section opens when it starts failing, so the error is never folded away unseen; it can
  // be folded again after
  useEffect(() => {
    if ((targetsFailed && !sections.targets) || (boxFailed && !sections.box))
      onSectionsChange({
        ...sections,
        targets: sections.targets || targetsFailed,
        box: sections.box || boxFailed,
      });
  }, [targetsFailed, boxFailed]);
  const steady = useSteadyBox(snapshot.box);
  const shown = { ...snapshot, box: steady.box };
  const medius = steady.box.link === "connected" && steady.box.medius;
  return (
    <div className="panel setup-panel">
      <TargetSection
        status={snapshot.vision}
        recording={!!snapshot.recording}
        blocker={snapshot.readiness.screen_capture}
        native={native}
        command={call}
        runs={snapshot.runs}
        open={sections.targets}
        onOpenChange={(targets) => onSectionsChange({ ...sections, targets })}
      />
      <div className="box-pair">
        <div className="box-pair-row">
          <BoxSection
            snapshot={shown}
            switching={steady.switching}
            native={native}
            busy={busy}
            call={call}
            open={sections.box}
            onOpenChange={(box) => onSectionsChange({ ...sections, box })}
          />
          {medius && (
            <MediusSection
              medius={medius}
              snapshot={shown}
              native={native}
              call={call}
              open={sections.box_settings}
              onOpenChange={(box_settings) => onSectionsChange({ ...sections, box_settings })}
            />
          )}
        </div>
      </div>
    </div>
  );
}
