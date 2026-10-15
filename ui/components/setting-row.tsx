import type { ReactNode } from "react";
import { Field, FieldContent, FieldDescription, FieldLabel } from "@/components/ui/field";

/** a setting: its name and what it does on the left, the control on the right */
export function SettingRow({
  id,
  label,
  description,
  children,
}: {
  id?: string;
  label: string;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Field orientation="horizontal">
      <FieldContent>
        <FieldLabel htmlFor={id}>{label}</FieldLabel>
        {description && <FieldDescription>{description}</FieldDescription>}
      </FieldContent>
      <div className="setting-control">{children}</div>
    </Field>
  );
}

/** a titled group of settings inside a setup section */
export function SettingSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="setting-section" aria-label={title}>
      <h4>{title}</h4>
      {children}
    </section>
  );
}
