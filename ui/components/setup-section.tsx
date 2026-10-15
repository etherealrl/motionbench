import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

/** one optional part of a recording: a header row (title, status, one action) over its settings,
 *  with what it reports beside them when there is room */
export function SetupSection({
  title,
  status,
  hint,
  action,
  aside,
  open,
  onOpenChange,
  children,
}: {
  title: string;
  status?: ReactNode;
  hint?: string;
  action?: ReactNode;
  aside?: ReactNode;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
}) {
  return (
    <Collapsible open={open} onOpenChange={onOpenChange}>
      <section aria-label={title}>
        <div className="setup-section-header">
          <CollapsibleTrigger asChild>
            <button type="button" className="setup-section-trigger">
              <ChevronRight className="disclosure-chevron" size={16} aria-hidden />
              <h3>{title}</h3>
            </button>
          </CollapsibleTrigger>
          {status}
          {hint && <span className="setup-section-hint">{hint}</span>}
          {/* the action sits outside the toggle, so starting or connecting never folds the section */}
          <div className="setup-section-action">{action}</div>
        </div>
        <CollapsibleContent>
          <div className="setup-section-body">
            <div className="setup-section-main">{children}</div>
            {aside && <div className="setup-section-aside">{aside}</div>}
          </div>
        </CollapsibleContent>
      </section>
    </Collapsible>
  );
}
