export function frameMailbox<T>(
  consume: (value: T) => void,
  request = requestAnimationFrame,
  cancel = cancelAnimationFrame,
) {
  let frame: number | null = null;
  let latest: T;
  let disposed = false;
  return {
    push(value: T) {
      if (disposed) return;
      latest = value;
      if (frame !== null) return;
      frame = request(() => {
        frame = null;
        if (!disposed) consume(latest);
      });
    },
    dispose() {
      disposed = true;
      if (frame !== null) cancel(frame);
      frame = null;
    },
  };
}
