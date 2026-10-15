import type { ReactNode } from "react";
import { targetCircle } from "@/lib/vision";

/** a target on screen, its box in the screen's pixels */
export type StageTarget = { bounds: readonly number[]; confidence?: number };

/** the captured screen at its own aspect, each target as the circle its box holds and the
 * crosshair in the middle. `children` draws over it in the screen's pixels, `empty` covers it
 * when there's no target */
export function TargetStage({
  width,
  height,
  targets,
  crosshair = "cross",
  label,
  empty,
  children,
}: {
  width: number;
  height: number;
  targets: StageTarget[];
  crosshair?: "cross" | "dot";
  label: string;
  empty?: ReactNode;
  children?: ReactNode;
}) {
  const circles = targets
    .map((target) => ({
      circle: targetCircle(target.bounds, width, height),
      confidence: target.confidence,
    }))
    .filter((target) => target.circle !== null);
  return (
    <div className="target-stage" style={{ "--stage-ratio": `${width} / ${height}` }}>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={label}
        preserveAspectRatio="xMidYMid meet"
      >
        {children}
        {circles.map(({ circle, confidence }, i) => (
          <g key={i} data-testid="detected-target">
            {confidence !== undefined && (
              <title>{Math.round(confidence * 100)}% confidence, estimated target circle</title>
            )}
            <circle
              cx={circle!.x}
              cy={circle!.y}
              r={circle!.radius}
              fill="var(--color-primary)"
              fillOpacity={0.14}
              stroke="var(--color-primary)"
              strokeWidth={2}
              vectorEffect="non-scaling-stroke"
            />
            <circle cx={circle!.x} cy={circle!.y} r={3} fill="var(--color-primary)" />
          </g>
        ))}
        {crosshair === "cross" ? (
          <path
            d={`M${width / 2 - 18} ${height / 2}h36M${width / 2} ${height / 2 - 18}v36`}
            stroke="currentColor"
            strokeWidth={1.5}
            vectorEffect="non-scaling-stroke"
          />
        ) : (
          <circle
            cx={width / 2}
            cy={height / 2}
            r={Math.max(3, width / 400)}
            fill="var(--color-foreground)"
            data-testid="crosshair"
          />
        )}
      </svg>
      {!circles.length && empty}
    </div>
  );
}
