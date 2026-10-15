import type { ReactNode } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { Run } from "@/types";

// the shown run outlives open, so the dialog keeps its content and its opener while it closes
export type ShownRun = {
  run: Run;
  opener: HTMLElement | null;
  open: boolean;
};

/** a dialog about one run, opened from its row's menu. focus goes back to the control that opened
 *  it, the menu it came from is gone by then */
export function RunDialog({
  shown,
  title,
  className,
  onClose,
  children,
}: {
  shown: ShownRun | null;
  title: string;
  className?: string;
  onClose: () => void;
  children: (run: Run) => ReactNode;
}) {
  return (
    <Dialog open={!!shown?.open} onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        className={className}
        onCloseAutoFocus={(event) => {
          if (!shown?.opener) return;
          event.preventDefault();
          shown.opener.focus();
        }}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{shown?.run.name}</DialogDescription>
        </DialogHeader>
        {shown && children(shown.run)}
      </DialogContent>
    </Dialog>
  );
}
