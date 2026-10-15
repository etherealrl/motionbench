import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  ArrowDownAZ,
  ArrowUpAZ,
  CalendarArrowDown,
  CalendarArrowUp,
  ChevronDown,
  X,
} from "lucide-react";
import type { Run } from "@/types";
import { Items } from "@/components/inline";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { sorted, type Sort } from "@/runs/sort";

// the orders a list of recordings can take, newest first unless picked otherwise
const orders = {
  newest: {
    sort: { by: "date", down: true },
    label: "Newest first",
    Icon: CalendarArrowDown,
  },
  oldest: {
    sort: { by: "date", down: false },
    label: "Oldest first",
    Icon: CalendarArrowUp,
  },
  az: {
    sort: { by: "name", down: false },
    label: "Name A to Z",
    Icon: ArrowDownAZ,
  },
  za: {
    sort: { by: "name", down: true },
    label: "Name Z to A",
    Icon: ArrowUpAZ,
  },
} satisfies Record<string, { sort: Sort; label: string; Icon: typeof X }>;
type Order = keyof typeof orders;

export function RecordingPicker({
  name,
  runs,
  selected,
  disabled = false,
  onChange,
}: {
  name: string;
  runs: Run[];
  selected: string[];
  disabled?: boolean;
  onChange: (ids: string[]) => void;
}) {
  const [search, setSearch] = useState("");
  const [order, setOrder] = useState<Order>("newest");
  const visible = sorted(runs, orders[order].sort).filter((run) =>
    run.name.toLowerCase().includes(search.trim().toLowerCase()),
  );
  const chosen = runs.filter((run) => selected.includes(run.id));
  return (
    <div className="population-card">
      <div className="population-heading">
        <strong>
          <i className={name === "Baseline" ? "baseline-dot" : "candidate-dot"} />
          {name}
        </strong>
      </div>
      <Popover>
        <PopoverTrigger asChild>
          <Button
            variant="outline"
            className="population-trigger"
            disabled={disabled}
            aria-label={"Choose " + name.toLowerCase() + " recordings"}
          >
            <span>
              {chosen.length === 1
                ? chosen[0].name
                : chosen.length
                  ? chosen.length + " recordings"
                  : "Choose recordings"}
            </span>
            <ChevronDown />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="recording-popover">
          <div className="recording-search">
            <Input
              type="search"
              aria-label={"Search " + name.toLowerCase() + " recordings"}
              placeholder="Search recordings…"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
            <span className="recording-sort">
              <Dropdown
                aria-label={"Sort " + name.toLowerCase() + " recordings"}
                value={order}
                onValueChange={(value) => setOrder(value as Order)}
              >
                {Object.entries(orders).map(([key, { label, Icon }]) => (
                  <DropdownOption key={key} value={key}>
                    <Icon />
                    <span className="recording-sort-label">{label}</span>
                  </DropdownOption>
                ))}
              </Dropdown>
            </span>
          </div>
          <div className="recording-picker" role="group" aria-label={name + " recordings"}>
            {visible.map((run) => (
              <label key={run.id} className="recording-choice">
                <Checkbox
                  checked={selected.includes(run.id)}
                  disabled={disabled}
                  onCheckedChange={(checked) =>
                    onChange(
                      checked === true
                        ? [...selected, run.id]
                        : selected.filter((id) => id !== run.id),
                    )
                  }
                />
                <span title={run.path}>
                  {run.name}
                  <small>
                    {run.paired ? (
                      "Physical input available"
                    ) : (
                      <Items>
                        {run.panels.map((panel) => (
                          <span key={panel}>{panel}</span>
                        ))}
                      </Items>
                    )}
                  </small>
                </span>
              </label>
            ))}
            {!visible.length && (
              <span className="picker-empty">
                {runs.length ? "No matching recordings" : "No recordings yet"}
              </span>
            )}
          </div>
        </PopoverContent>
      </Popover>
      {chosen.length > 1 && (
        <div className="selected-recordings">
          {chosen.map((run) => (
            <span key={run.id} title={run.path}>
              {run.name}
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={"Remove " + run.name + " from " + name.toLowerCase()}
                disabled={disabled}
                onClick={() => onChange(selected.filter((id) => id !== run.id))}
              >
                <X />
              </Button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
