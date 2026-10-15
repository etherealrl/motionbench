import { CircleAlert } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import type { Run } from "@/types";

export function DeleteRunDialog({
  run,
  pending,
  error,
  onClose,
  onDelete,
}: {
  run: Run | null;
  pending: boolean;
  error: string | null;
  onClose: () => void;
  onDelete: () => void;
}) {
  return (
    <AlertDialog
      open={!!run}
      onOpenChange={(open) => {
        if (!open && !pending) onClose();
      }}
    >
      <AlertDialogContent
        onEscapeKeyDown={(event) => {
          if (pending) event.preventDefault();
        }}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>Remove from library?</AlertDialogTitle>
          <AlertDialogDescription>
            {run?.name} leaves the library and its comparisons. the capture files and saved results
            stay on disk though, so importing the recording again brings it back
          </AlertDialogDescription>
        </AlertDialogHeader>
        {error && (
          <Alert variant="destructive">
            <CircleAlert />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={pending}>Cancel</AlertDialogCancel>
          {/* a plain button, the dialog has to stay open when removing fails */}
          <Button variant="destructive" disabled={pending} onClick={onDelete}>
            {pending ? "Removing…" : "Remove"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
