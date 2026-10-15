import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open } from "@tauri-apps/plugin-dialog";
import { Tabs, TabsContent } from "@/components/ui/tabs";
import { Toaster } from "@/components/ui/sonner";
import { native } from "@/lib/native";
import { useSession } from "@/app/use-session";
import { AppHeader } from "@/app/header";
import { Notices } from "@/app/notices";
import { useUpdate } from "@/app/use-update";
import { useCapture } from "@/capture/use-capture";
import { CaptureView } from "@/capture/capture-view";
import { useComparison } from "@/compare/use-comparison";
import { CompareView } from "@/compare/compare-view";
import { ExperimentPanel } from "@/experiment/experiment";
import { RunsView } from "@/runs/runs-view";
import type { Run } from "@/types";

export function App() {
  const session = useSession();
  const comparison = useComparison(session);
  const { snapshot, libraryReady, call } = session;
  const [tab, setTab] = useState("capture");
  // an imported recording opens in Compare as the candidate
  const importPath = useCallback(
    async (path: string) => {
      const run = await call<Run>("import_run", { path });
      if (run) {
        session.addRun(run);
        comparison.showImported(run.id);
        setTab("compare");
      }
    },
    [call],
  );
  const capture = useCapture(session);
  useUpdate(session.setError);
  useEffect(() => {
    if (!native || !libraryReady) return;
    const off = getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (event.payload.type === "drop")
        for (const path of event.payload.paths) void importPath(path);
    });
    return () => {
      void off.then((unlisten) => unlisten());
    };
  }, [importPath, libraryReady]);
  const chooseImport = async () => {
    const path = await open({
      multiple: true,
      filters: [{ name: "Motion Bench recordings", extensions: ["pcap"] }],
    });
    for (const item of typeof path === "string" ? [path] : (path ?? [])) await importPath(item);
  };
  return (
    <main className="app-shell">
      <Tabs value={tab} onValueChange={setTab}>
        <AppHeader
          snapshot={snapshot}
          libraryReady={libraryReady}
          onImport={() => void chooseImport()}
          onError={session.setError}
        />
        <Notices health={snapshot.health} libraryReady={libraryReady} />
        <TabsContent value="capture">
          <CaptureView session={session} capture={capture} />
        </TabsContent>
        {snapshot.developer_tools && (
          <TabsContent value="experiment">
            <ExperimentPanel
              snapshot={snapshot}
              native={native}
              busy={capture.busy}
              command={call}
              capture={() => {
                capture.openSetup();
                setTab("capture");
              }}
              record={() =>
                snapshot.recording
                  ? capture.stopRecord()
                  : capture.action("recording_toggle", { recording: true })
              }
            />
          </TabsContent>
        )}
        <TabsContent value="compare">
          <CompareView session={session} comparison={comparison} />
        </TabsContent>
        <TabsContent value="runs">
          <RunsView
            session={session}
            onInspect={(run) => {
              comparison.inspect(run);
              setTab("compare");
            }}
            onDelete={async (run) => {
              await invoke("delete_run", { id: run.id });
              session.dropRun(run.id);
              comparison.forget(run.id);
            }}
            onImport={() => void chooseImport()}
          />
        </TabsContent>
      </Tabs>
      <Toaster position="bottom-right" richColors expand />
    </main>
  );
}
