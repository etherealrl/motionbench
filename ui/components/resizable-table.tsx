import { useLayoutEffect, useRef, useState } from "react";
import { Table } from "@/components/ui/table";
import { cn } from "@/lib/utils";

const smallest = 4;

/** a table whose columns you can drag wider or narrower by their edges. the widths are shares of
 * the table, so it still fits when the window changes size, and they're remembered under `id`.
 * `widen` makes a column start wider than its content asks, by how many times, keyed by its index.
 * a double click on an edge goes back to the starting widths */
export function ResizableTable({
  id,
  widen = {},
  className,
  children,
  ...props
}: {
  id: string;
  widen?: Record<number, number>;
} & React.ComponentProps<typeof Table>) {
  const frame = useRef<HTMLDivElement>(null);
  const key = `table-columns:${id}`;
  const [widths, setWidths] = useState<number[] | null>(() => saved(key));
  // where each edge between two columns sits, in pixels from the table's left
  const [edges, setEdges] = useState<number[]>([]);

  // the starting widths are what the content asks for, so they're measured before the first fixed layout.
  // a header with a row of groups above it has its columns in the last row
  useLayoutEffect(() => {
    const heads = [...(frame.current?.querySelectorAll("thead tr:last-child th") ?? [])];
    // remembered widths only fit the columns they were made for
    if (!heads.length || widths?.length === heads.length) return;
    const asked = heads.map((th, i) => th.getBoundingClientRect().width * (widen[i] ?? 1));
    const total = asked.reduce((a, b) => a + b, 0);
    setWidths(asked.map((w) => (w / total) * 100));
  }, [widths, widen]);

  useLayoutEffect(() => {
    const table = frame.current?.querySelector("table");
    if (!table || !widths) return;
    const place = () => {
      const heads = [...table.querySelectorAll("thead tr:last-child th")];
      const box = frame.current!.getBoundingClientRect();
      // the page measures in scaled pixels while css places in the UI scale's own, so this undoes it
      const scale = frame.current!.offsetWidth / box.width;
      setEdges(
        heads.slice(0, -1).map((th) => (th.getBoundingClientRect().right - box.left) * scale),
      );
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(table);
    return () => observer.disconnect();
  }, [widths]);

  const drag = (edge: number, down: React.PointerEvent<HTMLDivElement>) => {
    const table = frame.current?.querySelector("table");
    if (!table || !widths) return;
    const start = widths;
    const width = table.getBoundingClientRect().width;
    const handle = down.currentTarget;
    handle.setPointerCapture(down.pointerId);
    let latest = start;
    const move = (e: PointerEvent) => {
      latest = trade(start, edge, ((e.clientX - down.clientX) / width) * 100);
      setWidths(latest);
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      handle.removeEventListener("pointercancel", up);
      localStorage.setItem(key, JSON.stringify(latest));
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
  };

  // the arrow keys move a focused edge a little at a time
  const nudge = (edge: number, e: React.KeyboardEvent<HTMLDivElement>) => {
    const by = { ArrowLeft: -2, ArrowRight: 2 }[e.key];
    if (!widths || !by) return;
    e.preventDefault();
    const next = trade(widths, edge, by);
    setWidths(next);
    localStorage.setItem(key, JSON.stringify(next));
  };

  const reset = () => {
    localStorage.removeItem(key);
    setWidths(null);
  };

  return (
    <div ref={frame} className="resizable-table">
      <Table className={cn(widths && "table-fixed", className)} {...props}>
        {widths && (
          <colgroup>
            {widths.map((w, i) => (
              <col key={i} style={{ "--column": `${w}%` }} />
            ))}
          </colgroup>
        )}
        {children}
      </Table>
      {edges.map((at, i) => (
        <div
          key={i}
          className="column-edge"
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize the columns"
          tabIndex={0}
          style={{ "--at": `${at}px` }}
          onPointerDown={(e) => drag(i, e)}
          onKeyDown={(e) => nudge(i, e)}
          onDoubleClick={reset}
        />
      ))}
    </div>
  );
}

/** moves one edge by `by` percent of the table. it trades width between its two columns, so the
 * table keeps its size, and neither gets narrower than the smallest */
function trade(widths: number[], edge: number, by: number) {
  const pair = widths[edge] + widths[edge + 1];
  const left = Math.min(pair - smallest, Math.max(smallest, widths[edge] + by));
  return widths.map((w, i) => (i === edge ? left : i === edge + 1 ? pair - left : w));
}

function saved(key: string): number[] | null {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? "null");
    return Array.isArray(value) && value.every((w) => typeof w === "number") ? value : null;
  } catch {
    return null;
  }
}
