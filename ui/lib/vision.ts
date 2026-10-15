import type { VisionStatus } from "@/types";

export function targetCircle(bounds: readonly number[], width: number, height: number) {
  if (
    bounds.length !== 4 ||
    !bounds.every(Number.isFinite) ||
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  )
    return null;
  const [left, top, right, bottom] = bounds;
  if (left < 0 || top < 0 || right > width || bottom > height || right <= left || bottom <= top)
    return null;
  return {
    x: (left + right) / 2,
    y: (top + bottom) / 2,
    radius: Math.min(right - left, bottom - top) / 2,
  };
}

// a frame older than this is no longer what the screen shows
export function currentFrame(status: VisionStatus | undefined) {
  return (
    !!status?.running && status.ready && !!status.last && (status.frame_age_ms ?? Infinity) < 500
  );
}
