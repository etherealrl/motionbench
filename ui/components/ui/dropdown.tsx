import { useRef, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./select";

/** a select whose empty value shows `placeholder`, which is never an option, so nothing can
 * pick "no choice" and send it on */
export function Dropdown({
  value,
  onValueChange,
  disabled,
  children,
  className,
  onOpen,
  placeholder,
  ...trigger
}: {
  value: string | number;
  onValueChange: (value: string) => void;
  onOpen?: () => void;
  placeholder?: ReactNode;
  disabled?: boolean;
  children: ReactNode;
  "aria-label"?: string;
  id?: string;
  size?: "sm" | "default";
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  return (
    <Select
      value={String(value)}
      disabled={disabled}
      open={open}
      // a disabled fieldset around it disables the trigger, which Radix doesn't see
      onOpenChange={(next) => {
        const opening = next && !triggerRef.current?.matches(":disabled");
        if (opening) onOpen?.();
        setOpen(opening);
      }}
      onValueChange={onValueChange}
    >
      <SelectTrigger ref={triggerRef} {...trigger} className={cn("w-full min-w-0", className)}>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent position="popper" align="start" className="dropdown-content">
        {children}
      </SelectContent>
    </Select>
  );
}

export function DropdownOption({
  value,
  children,
  disabled,
}: {
  value?: string | number;
  children: ReactNode;
  disabled?: boolean;
}) {
  const text = typeof children === "string" || typeof children === "number" ? children : "";
  const optionValue = String(value ?? text);
  return (
    <SelectItem value={optionValue} data-value={optionValue} disabled={disabled}>
      {children}
    </SelectItem>
  );
}
