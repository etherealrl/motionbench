import { useEffect, useState } from "react";
import { Pause, Play, Usb } from "lucide-react";
import { Badge, type BadgeTone } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { AnalysisViews } from "@/charts/views";
import { captureRange } from "@/lib/plots";
import { native } from "@/lib/native";
import type { Session } from "@/app/use-session";
import type { Snapshot, Tab } from "@/types";

// whether the view follows the clock or holds a paused range; the preview's sample is neither
function viewState(snapshot: Snapshot): { label: string; tone: BadgeTone } | null {
  if (!native) return null;
  return snapshot.range ? { label: "Paused", tone: "warning" } : { label: "Live", tone: "success" };
}

export function LiveView({ snapshot, call }: { snapshot: Snapshot; call: Session["call"] }) {
  const state = viewState(snapshot);
  const [tab, setTab] = useState<Tab>("timing");
  // the native side computes only the tab shown
  useEffect(() => {
    if (native) void call("set_tab", { tab });
  }, [tab]);
  const toolbar = (
    <div className="range-toolbar">
      <span className="range-label">
        {state && <Badge variant={state.tone}>{state.label}</Badge>}
        {((snapshot.plot.to_us - snapshot.plot.from_us) / 1e6).toFixed(3)} seconds
      </span>
      {snapshot.range ? (
        <Button
          variant="ghost"
          size="xs"
          disabled={!native}
          onClick={() => void call("set_range", { range: null })}
        >
          <Play /> Resume live
        </Button>
      ) : (
        <Button
          variant="ghost"
          size="xs"
          disabled={!native}
          onClick={() =>
            void call("set_range", {
              range: {
                from_us: snapshot.plot.from_us,
                to_us: snapshot.plot.to_us,
              },
            })
          }
        >
          <Pause /> Pause
        </Button>
      )}
    </div>
  );
  return (snapshot.live && (snapshot.plot.reports > 0 || snapshot.range)) || !native ? (
    <AnalysisViews
      plot={snapshot.plot}
      scope={snapshot.range ? "Paused range" : "Live, the last 8 seconds"}
      tab={tab}
      onTab={setTab}
      toolbar={toolbar}
      onBrush={(r) => {
        if (native)
          void call("set_range", {
            range: captureRange(snapshot.plot.from_us, r),
          });
      }}
    />
  ) : (
    <section className="panel live-empty">
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Usb />
          </EmptyMedia>
          <EmptyTitle>No live data</EmptyTitle>
          <EmptyDescription>
            once monitoring runs, the selected mouse's reports show up here
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    </section>
  );
}
