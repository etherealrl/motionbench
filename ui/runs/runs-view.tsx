import { useRef, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  Clock,
  Ellipsis,
  FileUp,
  Library,
  Scissors,
  Trash2,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { TableHeader, TableBody, TableRow, TableHead, TableCell } from "@/components/ui/table";
import { ResizableTable } from "@/components/resizable-table";
import { Items } from "@/components/inline";
import { deviceIds } from "@/components/device-label";
import { native } from "@/lib/native";
import { panelLabel } from "@/lib/panels";
import type { Session } from "@/app/use-session";
import { outcome } from "@/lib/outcome";
import type { MediusState, Run } from "@/types";
import { DeleteRunDialog } from "./delete-run";
import { RecordingClocks } from "./recording-clocks";
import { CropRun, cropLength } from "./crop-run";
import { hadTargets } from "./replay";
import type { ShownRun } from "./run-dialog";
import { boxLabel, recordedBox, sorted, type Sort, type SortBy } from "./sort";

export function RunsView({
  session,
  onInspect,
  onDelete,
  onImport,
}: {
  session: Session;
  onInspect: (run: Run) => void;
  onDelete: (run: Run) => Promise<void>;
  onImport: () => void;
}) {
  const { snapshot, libraryReady } = session;
  const [deleting, setDeleting] = useState<Run | null>(null);
  const [clocks, setClocks] = useState<ShownRun | null>(null);
  const [crop, setCrop] = useState<ShownRun | null>(null);
  const [deletePending, setDeletePending] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [sort, setSort] = useState<Sort>({ by: "date", down: true });
  const runs = sorted(snapshot.runs, sort);
  const head = (label: string, by: SortBy) => (
    <SortHead label={label} by={by} sort={sort} onSort={setSort} />
  );
  const deleteRun = async () => {
    if (!deleting) return;
    setDeletePending(true);
    setDeleteError(null);
    try {
      await onDelete(deleting);
      setDeleting(null);
    } catch (e) {
      setDeleteError(String(e));
    } finally {
      setDeletePending(false);
    }
  };
  return (
    <>
      <div className="section-intro">
        <h2>Recordings</h2>
      </div>
      {snapshot.runs.length ? (
        <section className="panel run-table">
          <ResizableTable id="recordings" widen={{ 1: 1.75 }} fit={5} aria-label="Recordings">
            <TableHeader>
              <TableRow>
                {head("Recording", "name")}
                {head("Date", "date")}
                {head("Box", "box")}
                <TableHead>Source</TableHead>
                <TableHead>Analysis</TableHead>
                <TableHead>
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {runs.map((r) => (
                <RunRow
                  key={r.id}
                  run={r}
                  session={session}
                  onInspect={onInspect}
                  onDelete={() => {
                    setDeleteError(null);
                    setDeleting(r);
                  }}
                  onClocks={(opener) => setClocks({ run: r, opener, open: true })}
                  onCrop={(opener) => setCrop({ run: r, opener, open: true })}
                />
              ))}
            </TableBody>
          </ResizableTable>
        </section>
      ) : (
        <section className="panel">
          <Empty>
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Library />
              </EmptyMedia>
              <EmptyTitle>No recordings yet</EmptyTitle>
              <EmptyDescription>
                the recordings you make in Capture or import show up here
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button variant="outline" disabled={!native || !libraryReady} onClick={onImport}>
                <FileUp />
                Import recording
              </Button>
            </EmptyContent>
          </Empty>
        </section>
      )}
      <RecordingClocks
        shown={clocks}
        onClose={() => setClocks((shown) => shown && { ...shown, open: false })}
      />
      <CropRun
        shown={crop}
        onClose={() => setCrop((shown) => shown && { ...shown, open: false })}
      />
      <DeleteRunDialog
        run={deleting}
        pending={deletePending}
        error={deleteError}
        onClose={() => setDeleting(null)}
        onDelete={() => void deleteRun()}
      />
    </>
  );
}

function SortHead({
  label,
  by,
  sort,
  onSort,
}: {
  label: string;
  by: SortBy;
  sort: Sort;
  onSort: (sort: Sort) => void;
}) {
  const active = sort.by === by;
  const Arrow = sort.down ? ArrowDown : ArrowUp;
  return (
    <TableHead aria-sort={active ? (sort.down ? "descending" : "ascending") : "none"}>
      <Button
        variant="ghost"
        size="sm"
        // the button pads its label, so it moves out by that much to start where the cells do
        className={active ? "-ml-2.5" : "-ml-3"}
        onClick={() => onSort({ by, down: active ? !sort.down : by === "date" })}
      >
        {label}
        {active && <Arrow />}
      </Button>
    </TableHead>
  );
}

function Started({ run }: { run: Run }) {
  const us = run.started_us;
  if (us === null) return <span>-</span>;
  const at = new Date(us / 1000);
  return (
    <>
      <span>{at.toLocaleDateString(undefined, { dateStyle: "medium" })}</span>
      <small>{at.toLocaleTimeString(undefined, { timeStyle: "short" })}</small>
    </>
  );
}

// where the recording came from
function Source({ run }: { run: Run }) {
  return <span>{run.devices[0] ? (deviceIds(run.devices[0]) ?? "-") : "Event dataset"}</span>;
}

function MediusSummary({ medius }: { medius: Partial<MediusState> }) {
  return (
    <>
      <span>{medius.render?.effective ?? "-"}</span>
      <span>emit {medius.emit?.resolved_hz ?? "-"} Hz</span>
      <span>spread {medius.spread?.percent ?? "-"}%</span>
    </>
  );
}

/** the box a recording went through, its firmware, and on a Medius what it was set to */
function RecordedBoxCell({ run }: { run: Run }) {
  const box = recordedBox(run);
  if (!boxLabel(box)) return <span>-</span>;
  return (
    <>
      <span>{boxLabel(box)}</span>
      <small>
        <Items>
          {box?.firmware && <span>{box.firmware}</span>}
          {box?.medius && <MediusSummary medius={box.medius} />}
        </Items>
      </small>
    </>
  );
}

// why a recording made with target capture still has no Trajectory
function noTrials(run: Run) {
  return hadTargets(run) && !run.panels.includes("trajectory14")
    ? run.missing.trajectory14
    : undefined;
}

function Analysis({ run }: { run: Run }) {
  const judge = run.result?.judge;
  const cropped = cropLength(run);
  const why = noTrials(run);
  return (
    <>
      <span>
        <Items>
          {run.panels.length > 0 ? (
            run.panels.map((panel) => <span key={panel}>{panelLabel(panel)}</span>)
          ) : (
            <span>{run.missing.texture19 ? "Evidence only" : "Timing only"}</span>
          )}
          {judge && <span>AUC {judge.auc.toFixed(3)}</span>}
        </Items>
      </span>
      {(run.result || run.paired || cropped) && (
        <small>
          <Items>
            {cropped && <span>{cropped}</span>}
            {run.result ? (
              <>
                <span>{run.result.config.width_ms} ms windows</span>
                <span>{run.result.config.repeats} repeats</span>
              </>
            ) : (
              run.paired && <span>Physical input recorded</span>
            )}
          </Items>
        </small>
      )}
      {why && <small className="run-status-reason">{why}</small>}
    </>
  );
}

/** the name, which becomes its own field when clicked */
function RunName({ run, rename }: { run: Run; rename: (name: string) => void }) {
  const [renaming, setRenaming] = useState(false);
  if (renaming) return <RenameInput run={run} onDone={() => setRenaming(false)} rename={rename} />;
  if (!native)
    return (
      <span className="run-name" title={run.name}>
        {run.name}
      </span>
    );
  return (
    <button
      type="button"
      className="run-name run-name-edit"
      title={run.name}
      aria-label={`Rename ${run.name}`}
      onClick={() => setRenaming(true)}
    >
      {run.name}
    </button>
  );
}

function RenameInput({
  run,
  onDone,
  rename,
}: {
  run: Run;
  onDone: () => void;
  rename: (name: string) => void;
}) {
  return (
    <Input
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      className="-my-1 h-7 max-w-sm"
      aria-label={`Name for ${run.name}`}
      defaultValue={run.name}
      onBlur={(e) => {
        const name = e.target.value.trim();
        if (name && name !== run.name) rename(name);
        onDone();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          e.currentTarget.value = run.name;
          e.currentTarget.blur();
        }
      }}
    />
  );
}

function RunRow({
  run: r,
  session,
  onInspect,
  onDelete,
  onClocks,
  onCrop,
}: {
  run: Run;
  session: Session;
  onInspect: (run: Run) => void;
  onDelete: () => void;
  onClocks: (opener: HTMLElement | null) => void;
  onCrop: (opener: HTMLElement | null) => void;
}) {
  const { snapshot, call } = session;
  const menuButton = useRef<HTMLButtonElement>(null);
  const busy = snapshot.job?.status === "running";
  const state = outcome[r.status.kind];
  return (
    <TableRow>
      <TableCell>
        <div className="run-title">
          <RunName run={r} rename={(name) => void call("rename_run", { id: r.id, name })} />
          <Badge variant={state.tone}>{state.label}</Badge>
        </div>
        {r.status.message && <small className="run-status-reason">{r.status.message}</small>}
        <small title={r.path}>{r.path}</small>
      </TableCell>
      <TableCell>
        <Started run={r} />
      </TableCell>
      <TableCell>
        <RecordedBoxCell run={r} />
      </TableCell>
      <TableCell>
        <Source run={r} />
      </TableCell>
      <TableCell>
        <Analysis run={r} />
      </TableCell>
      <TableCell>
        <div className="run-actions">
          <Button size="sm" variant="outline" disabled={busy} onClick={() => onInspect(r)}>
            Inspect
          </Button>
          {r.path.endsWith(".pcap") && (
            <Button
              size="sm"
              variant="outline"
              disabled={!native || busy}
              onClick={(e) => onCrop(e.currentTarget)}
            >
              <Scissors />
              {hadTargets(r) ? "Crop and replay" : "Crop"}
            </Button>
          )}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                ref={menuButton}
                size="icon-sm"
                variant="ghost"
                aria-label={`More actions for ${r.name}`}
              >
                <Ellipsis />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {r.path.endsWith(".pcap") && (
                <>
                  <DropdownMenuItem onSelect={() => onClocks(menuButton.current)}>
                    <Clock />
                    Recording clocks
                  </DropdownMenuItem>
                  <DropdownMenuSeparator />
                </>
              )}
              <DropdownMenuItem
                variant="destructive"
                disabled={!native || busy}
                onSelect={onDelete}
              >
                <Trash2 />
                Remove from library
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </TableCell>
    </TableRow>
  );
}
