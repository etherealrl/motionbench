import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { buttonName } from "@/lib/mouse-buttons";

export function MouseBinding({
  label,
  mask,
  learning,
  disabled,
  locked = false,
  clearDisabled,
  onPick,
  onClear,
}: {
  label: string;
  mask?: number | null;
  learning: boolean;
  disabled: boolean;
  /** in use right now: the binding reads as plain text, like the other values in use */
  locked?: boolean;
  clearDisabled?: boolean;
  onPick: () => void;
  onClear?: () => void;
}) {
  const value = mask ? buttonName(mask) : "Unset";
  if (locked) return <span className="mouse-binding mouse-binding-locked">{value}</span>;
  return (
    <div className="mouse-binding">
      <Button
        variant="outline"
        size="sm"
        disabled={disabled}
        onClick={onPick}
        onKeyDown={(event) => {
          if (learning && event.key === "Escape") {
            event.preventDefault();
            onPick();
          }
        }}
        aria-label={learning ? `Cancel ${label.toLowerCase()} binding` : `${label}: ${value}`}
        className="mouse-binding-picker"
        data-learning={learning || undefined}
      >
        {learning ? "Press a button…" : value}
      </Button>
      {onClear && !!mask && !learning && (
        <Button
          variant="ghost"
          size="icon-sm"
          disabled={clearDisabled ?? disabled}
          onClick={onClear}
          aria-label={`Clear ${label.toLowerCase()}`}
          title={`Clear ${label.toLowerCase()}`}
        >
          <X aria-hidden="true" />
        </Button>
      )}
    </div>
  );
}
