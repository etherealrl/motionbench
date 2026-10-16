import { useState, type ReactNode } from "react";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { SettingRow } from "@/components/setting-row";
import type { VisionConfig } from "@/types";

/** a number setting: key, label, unit, decimals shown, what it means */
export type NumberSetting = [keyof VisionConfig, string, string | null, number, string?];
export type Update = (key: keyof VisionConfig, value: string | number | boolean) => void;

// a text field, so a decimal reads with a point like the readouts whatever the system's locale;
// what is typed stays as typed until the field loses focus
export function NumberRow({
  setting: [key, label, unit, decimals, help],
  config,
  update,
  description = help,
  after,
}: {
  setting: NumberSetting;
  config: VisionConfig;
  update: Update;
  /** what it means when that depends on more than the setting */
  description?: ReactNode;
  /** a control that goes with the number, beside it */
  after?: ReactNode;
}) {
  const [typed, setTyped] = useState<string | null>(null);
  const field = (
    <InputGroup>
      <InputGroupInput
        id={`target-${key}`}
        inputMode="decimal"
        value={typed ?? String(Number(Number(config[key]).toFixed(decimals)))}
        onChange={(e) => setTyped(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
        }}
        // saved when the field is left, a half written value would be refused
        onBlur={() => {
          const value = Number(typed?.replace(",", "."));
          if (typed?.trim() && Number.isFinite(value)) update(key, value);
          setTyped(null);
        }}
      />
      {unit && <InputGroupAddon align="inline-end">{unit}</InputGroupAddon>}
    </InputGroup>
  );
  return (
    <SettingRow id={`target-${key}`} label={label} description={description}>
      {after ? (
        <div className="input-with-action">
          {field}
          {after}
        </div>
      ) : (
        field
      )}
    </SettingRow>
  );
}
