import { useEffect, useState } from "react";
import { FolderOpen, RotateCcw } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { setRecordingsFolder, useSettings } from "@/lib/settings";

type Folders = { running: string; fallback: string };

/** where recordings go. a new folder is used from the next start, since the library and the recorder
 * already have this one open, so the row offers the restart and asks whether the recordings go too */
export function RecordingsFolder({ onError }: { onError: (message: string) => void }) {
  const [folders, setFolders] = useState<Folders | null>(null);
  // the folder waiting on the move question, null being the default one
  const [asking, setAsking] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    invoke<Folders>("recordings_folders").then(setFolders, (e) => onError(String(e)));
  }, [onError]);
  const storage = useSettings().storage;
  if (!folders) return null;
  const next = storage.folder ?? folders.fallback;
  const choose = async (folder: string | null, move: boolean) => {
    setAsking(undefined);
    try {
      await setRecordingsFolder(folder, move);
    } catch (e) {
      onError(String(e));
    }
  };
  // going back to the folder in use leaves nothing to move
  const pick = (folder: string | null) =>
    (folder ?? folders.fallback) === folders.running
      ? void choose(folder, false)
      : setAsking(folder);
  const browse = async () => {
    const folder = await open({ directory: true, defaultPath: next });
    if (typeof folder === "string") pick(folder);
  };
  return (
    <div className="settings-row settings-row-top">
      <span className="settings-icon">
        <FolderOpen aria-hidden className="size-4" />
      </span>
      <div className="settings-folder" role="group" aria-label="Recordings folder">
        <span className="settings-path" title={next}>
          {next}
        </span>
        <div className="settings-folder-actions">
          <Button size="sm" variant="outline" onClick={() => void browse()}>
            Change
          </Button>
          {storage.folder && (
            <Button
              size="sm"
              variant="ghost"
              title={`back to ${folders.fallback}`}
              onClick={() => pick(null)}
            >
              <RotateCcw />
              Default
            </Button>
          )}
        </div>
        {next !== folders.running && (
          <div className="settings-restart">
            <small>
              {storage.move_from
                ? "new recordings go here after a restart, and the ones you already made move with them"
                : "new recordings go here after a restart, the ones you already made stay where they are"}
            </small>
            <Button
              size="sm"
              onClick={() => void invoke("restart_app").catch((e) => onError(String(e)))}
            >
              Restart
            </Button>
          </div>
        )}
      </div>
      <AlertDialog
        open={asking !== undefined}
        onOpenChange={(open) => {
          if (!open) setAsking(undefined);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Move your recordings too?</AlertDialogTitle>
            <AlertDialogDescription>
              they move to {asking ?? folders.fallback} when the app restarts, so they don't end up
              split between two folders. if you leave them in {folders.running}, Runs won't show
              them anymore
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <Button variant="outline" onClick={() => void choose(asking ?? null, false)}>
              Leave them
            </Button>
            <Button onClick={() => void choose(asking ?? null, true)}>Move them</Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
