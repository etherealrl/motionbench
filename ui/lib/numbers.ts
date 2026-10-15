/** the middle value, or the mean of the two middle ones; none from nothing */
export function median(values: number[]) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

/** a condition's values for one figure, the recordings without it left out */
export function present(values: (number | null | undefined)[]) {
  return values.filter((v): v is number => v !== null && v !== undefined);
}

/** the candidate's median minus the baseline's as it reads, with no sign once it rounds to
 * zero; none unless both medians exist */
export function difference(
  baseline: number | null,
  candidate: number | null,
  show: (v: number) => string,
) {
  if (baseline === null || candidate === null) return null;
  const apart = candidate - baseline;
  const text = show(Math.abs(apart));
  if (!/[1-9]/.test(text)) return text;
  return `${apart > 0 ? "+" : "−"}${text}`;
}
