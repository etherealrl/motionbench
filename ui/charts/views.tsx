import { useCallback, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Items } from "@/components/inline";
import type { Plot, Tab } from "@/types";
import { MovementTab } from "./movement";
import { RelayTab } from "./relay";
import { Readouts } from "./readouts";
import { Timeline } from "./timeline";
import { TimingTab } from "./timing";

/** the readouts and the interval timeline, which picks the range, over tabs that show the same
 * range through different lenses. a baseline shares distributions, never a time axis */
export function AnalysisViews({
  plot,
  baseline,
  baselineLabel = "baseline",
  scope,
  tab,
  onTab,
  onBrush,
  toolbar,
  comparison,
  comparisonActions,
}: {
  plot: Plot;
  baseline?: Plot;
  /** what the baseline is: another recording, or the box's own input */
  baselineLabel?: string;
  /** what the views cover: live, a saved recording, a selected range */
  scope: string;
  tab: Tab;
  onTab: (tab: Tab) => void;
  onBrush: (range: [number, number]) => void;
  /** controls for the range, under the readouts it scopes */
  toolbar?: ReactNode;
  /** the Comparison tab's content, in Compare only */
  comparison?: ReactNode;
  /** beside the legend while the Comparison tab is open */
  comparisonActions?: ReactNode;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const { readouts, timing, relay } = useSteadyViews(plot, baseline, baselineLabel, onBrush);
  const shown = shownTab(tab, plot);
  const held = useHeldHeight(
    shown,
    shown === "comparison" || !!plot.tabs[shown as keyof Plot["tabs"]],
  );
  return (
    <>
      {readouts}
      {toolbar}
      <Timeline
        plot={plot}
        label={baseline && "candidate"}
        hover={hover}
        onHover={setHover}
        onBrush={onBrush}
      />
      {baseline && (
        <Timeline
          plot={baseline}
          label={baselineLabel}
          hover={hover}
          onHover={setHover}
          onBrush={onBrush}
        />
      )}
      <div className="analysis-tabs" ref={held.ref} style={{ "--tabs-hold": `${held.height}px` }}>
        <Tabs
          value={shown}
          onValueChange={(value) => {
            held.hold();
            onTab(value as Tab);
          }}
        >
          <TabsHead
            plot={plot}
            baselineLabel={baseline && baselineLabel}
            scope={scope}
            comparison={!!comparison}
            actions={<span hidden={shown !== "comparison"}>{comparisonActions}</span>}
          />
          <TabsContent value="timing">{timing}</TabsContent>
          <TabsContent value="movement">
            <MovementTab
              plot={plot}
              baseline={baseline}
              baselineLabel={baselineLabel}
              hover={hover}
              onHover={setHover}
            />
          </TabsContent>
          <TabsContent value="relay">{relay}</TabsContent>
          {comparison && <TabsContent value="comparison">{comparison}</TabsContent>}
        </Tabs>
      </div>
    </>
  );
}

/** only a saved recording with the box's own input has a relay, anywhere else the tab isn't there */
function shownTab(tab: Tab, plot: Plot): Tab {
  return tab === "relay" && !plot.tabs.relay ? "timing" : tab;
}

/** the views that don't follow the hover. it moves with every pointer event, so these keep their
 * elements and their canvases aren't drawn again each time */
function useSteadyViews(
  plot: Plot,
  baseline: Plot | undefined,
  baselineLabel: string,
  onBrush: (range: [number, number]) => void,
) {
  const brush = useRef(onBrush);
  brush.current = onBrush;
  const select = useCallback((range: [number, number]) => brush.current(range), []);
  const readouts = useMemo(() => <Readouts plot={plot} baseline={baseline} />, [plot, baseline]);
  const timing = useMemo(
    () => (
      <TimingTab plot={plot} baseline={baseline} baselineLabel={baselineLabel} onSelect={select} />
    ),
    [plot, baseline, baselineLabel, select],
  );
  const relay = useMemo(
    () => <RelayTab plot={plot} baseline={baseline} baselineLabel={baselineLabel} />,
    [plot, baseline, baselineLabel],
  );
  return { readouts, timing, relay };
}

/** a tab shown live has no data until native's next frame; the area keeps its old height until
 * `tab` is `ready`, so the page never shrinks under the scroll position */
function useHeldHeight(tab: Tab, ready: boolean) {
  const ref = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(0);
  useLayoutEffect(() => {
    if (ready) setHeight(0);
  }, [tab, ready]);
  return {
    ref,
    height,
    hold: () => setHeight(ref.current?.offsetHeight ?? 0),
  };
}

/** the tabs, what the views cover and the legend, with the Comparison tab's actions */
function TabsHead({
  plot,
  baselineLabel,
  scope,
  comparison,
  actions,
}: {
  plot: Plot;
  /** set when there's a baseline */
  baselineLabel?: string;
  scope: string;
  comparison: boolean;
  actions: ReactNode;
}) {
  return (
    <div className="analysis-tabs-head">
      <TabsList>
        <TabsTrigger value="timing">Timing</TabsTrigger>
        <TabsTrigger value="movement">Movement</TabsTrigger>
        {plot.tabs.relay && <TabsTrigger value="relay">Relay</TabsTrigger>}
        {comparison && <TabsTrigger value="comparison">Comparison</TabsTrigger>}
      </TabsList>
      <span className="analysis-scope">
        <Items>
          <span>{plot.readouts.moving.toLocaleString()} moving intervals</span>
          <span>
            {(plot.readouts.gaps - plot.readouts.moving).toLocaleString()} other gaps left out
          </span>
        </Items>
        {baselineLabel && (
          <span className="legend">
            <span className="candidate-mark" /> Candidate
            <span className="baseline-mark" />{" "}
            {baselineLabel.charAt(0).toUpperCase() + baselineLabel.slice(1)}
          </span>
        )}
        {scope}
        {actions}
      </span>
    </div>
  );
}
