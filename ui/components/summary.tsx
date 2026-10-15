import type { ComponentProps } from "react";
import { ChevronRight } from "lucide-react";

/** a disclosure's summary, with a chevron that turns when it opens */
export function Summary({ children, ...props }: ComponentProps<"summary">) {
  return (
    <summary {...props}>
      <ChevronRight className="disclosure-chevron" size={14} aria-hidden />
      {children}
    </summary>
  );
}
