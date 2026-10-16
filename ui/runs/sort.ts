import type { MediusState, Run } from "@/types";

export type SortBy = "name" | "date" | "box";
export type Sort = { by: SortBy; down: boolean };

type RecordedBox = {
  family?: string;
  name?: string;
  firmware?: string;
  /** which box it was, the same each time it connects */
  key?: string;
  medius?: Partial<MediusState> | null;
};

export function recordedBox(run: Run) {
  return run.metadata.box as RecordedBox | undefined;
}

export function boxLabel(box: RecordedBox | undefined) {
  return box?.name || box?.family || "";
}

// a recording with no date or box goes last whichever way it's sorted
export function sorted(runs: Run[], { by, down }: Sort) {
  const key = (run: Run): string | number | null =>
    by === "date"
      ? run.started_us
      : by === "box"
        ? boxLabel(recordedBox(run)) || null
        : run.name.toLowerCase();
  return [...runs].sort((a, b) => {
    const [x, y] = [key(a), key(b)];
    if (x === null || y === null) return x === y ? 0 : x === null ? 1 : -1;
    const order = x < y ? -1 : x > y ? 1 : 0;
    return down ? -order : order;
  });
}
