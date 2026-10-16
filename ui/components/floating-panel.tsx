import { useLayoutEffect, useRef, type ReactNode } from "react";
import { animate, motion, useMotionValue, type PanInfo } from "motion/react";

export type Corner = { right: boolean; bottom: boolean };

/** pixels between the panel and the window's edge */
const MARGIN = 16;
const SPRING = { type: "spring", stiffness: 320, damping: 24 } as const;

/** a panel floating over the page like a picture in picture. it can be thrown anywhere and springs
 * into the corner the throw was heading for */
export function FloatingPanel({
  corner,
  onCornerChange,
  label,
  children,
}: {
  corner: Corner;
  onCornerChange: (corner: Corner) => void;
  label: string;
  children: ReactNode;
}) {
  const layer = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLElement>(null);
  const x = useMotionValue(0);
  const y = useMotionValue(0);
  const at = useRef(corner);
  // only a committed corner, a render React throws away never moves it
  useLayoutEffect(() => {
    at.current = corner;
  }, [corner]);
  const spot = ({ right, bottom }: Corner) => {
    const { clientWidth, clientHeight } = layer.current!;
    const { offsetWidth, offsetHeight } = panel.current!;
    return {
      x: right ? clientWidth - offsetWidth - MARGIN : MARGIN,
      y: bottom ? clientHeight - offsetHeight - MARGIN : MARGIN,
    };
  };
  // it keeps its corner when it or the window changes size, folding it included
  useLayoutEffect(() => {
    const start = spot(at.current);
    x.set(start.x);
    y.set(start.y);
    const observer = new ResizeObserver(() => {
      const to = spot(at.current);
      animate(x, to.x, SPRING);
      animate(y, to.y, SPRING);
    });
    observer.observe(layer.current!);
    observer.observe(panel.current!);
    return () => observer.disconnect();
  }, []);
  const land = (_: PointerEvent, { velocity }: PanInfo) => {
    const { clientWidth, clientHeight } = layer.current!;
    const { offsetWidth, offsetHeight } = panel.current!;
    // where the throw would carry it, so a flick reaches the far corner
    const next = {
      right: x.get() + velocity.x * 0.2 + offsetWidth / 2 > clientWidth / 2,
      bottom: y.get() + velocity.y * 0.2 + offsetHeight / 2 > clientHeight / 2,
    };
    const to = spot(next);
    animate(x, to.x, { ...SPRING, velocity: velocity.x });
    animate(y, to.y, { ...SPRING, velocity: velocity.y });
    if (next.right !== corner.right || next.bottom !== corner.bottom) onCornerChange(next);
  };
  return (
    <div ref={layer} className="floating-layer">
      <motion.section
        ref={panel}
        aria-label={label}
        className="floating-panel overflow-hidden rounded-lg border bg-popover/90 shadow-lg backdrop-blur-xl"
        // motion only takes its drag position as a style
        // eslint-disable-next-line shadcn/no-inline-styles
        style={{ x, y }}
        drag
        dragMomentum={false}
        onDragEnd={land}
        whileDrag={{ scale: 1.02 }}
      >
        <div className="floating-content">{children}</div>
      </motion.section>
    </div>
  );
}
