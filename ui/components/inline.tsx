import type { ReactNode } from "react";

/** items on one line, apart by spacing; each child is one item */
export function Items({ children }: { children: ReactNode }) {
  return <span className="items">{children}</span>;
}
