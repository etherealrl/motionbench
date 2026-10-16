import { useState } from "react";
import { ChevronDown, ChevronUp, CircleCheck, CircleDashed, Info, X } from "lucide-react";
import { FloatingPanel, type Corner } from "@/components/floating-panel";
import { Button } from "@/components/ui/button";
import { Dropdown, DropdownOption } from "@/components/ui/dropdown";
import { updateSettings, useSettings } from "@/lib/settings";
import { cases, isCase } from "@/lib/sides";
import type { Case, Purpose, Side, Snapshot } from "@/types";
import { shownGame } from "./setup/projection";
import { checklist, type Check } from "./checklist";

// a side picked for one comparison means nothing in another, so it's cleared on a change
const choose = (purpose: Purpose) =>
  updateSettings((shown) => ({
    display: { ...shown.display, purpose },
    ...(shown.display.purpose !== purpose && { recording: { side: null } }),
  }));

/** asked once, so Capture can show what the comparison needs */
/** a recording takes its tag when it ends, so the comparison can't change until then */
export function PurposePrompt({ recording }: { recording: boolean }) {
  const purpose = useSettings().display.purpose;
  if (purpose !== null) return null;
  return (
    <section className="panel purpose-prompt" aria-labelledby="purpose-title">
      <div className="purpose-head">
        <h3 id="purpose-title">What are you comparing?</h3>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label="Close"
          disabled={recording}
          onClick={() => choose("looking")}
        >
          <X />
        </Button>
      </div>
      <p className="field-description">
        pick one and Capture shows what to check before each recording and tags every recording with
        its side. you can change this later in the settings
      </p>
      <div className="purpose-choices">
        {(Object.keys(cases) as Case[]).map((kind) => (
          <Button key={kind} variant="outline" disabled={recording} onClick={() => choose(kind)}>
            {cases[kind].name}
          </Button>
        ))}
        <Button variant="ghost" disabled={recording} onClick={() => choose("looking")}>
          Just looking around
        </Button>
      </div>
    </section>
  );
}

/** what you're comparing, the same choices as the first start's question */
export function PurposePick({ disabled }: { disabled: boolean }) {
  const purpose = useSettings().display.purpose;
  return (
    <Dropdown
      aria-label="What you're comparing"
      disabled={disabled}
      value={purpose ?? ""}
      placeholder="What are you comparing?"
      onValueChange={(value) => choose(value as Purpose)}
    >
      {(Object.keys(cases) as Case[]).map((kind) => (
        <DropdownOption key={kind} value={kind}>
          {cases[kind].name}
        </DropdownOption>
      ))}
      <DropdownOption value="looking">Just looking around</DropdownOption>
    </Dropdown>
  );
}

/** the side the next recordings get tagged with */
export function SidePick({ kind, disabled }: { kind: Case; disabled: boolean }) {
  const side = useSettings().recording.side;
  const names = cases[kind].sides;
  return (
    <Dropdown
      aria-label="Side"
      className="w-auto min-w-40"
      value={side ?? "none"}
      disabled={disabled}
      onValueChange={(value) =>
        updateSettings({ recording: { side: value === "none" ? null : (value as Side) } })
      }
    >
      <DropdownOption value="none">No side</DropdownOption>
      <DropdownOption value="baseline">{names.baseline}</DropdownOption>
      <DropdownOption value="candidate">{names.candidate}</DropdownOption>
    </Dropdown>
  );
}

const icons = {
  done: <CircleCheck aria-hidden className="text-success" />,
  todo: <CircleDashed aria-hidden className="text-warning" />,
  note: <Info aria-hidden className="text-muted-foreground" />,
};

function CheckItem({ check, folded }: { check: Check; folded: boolean }) {
  return (
    <li className="assist-check" data-state={check.state}>
      {icons[check.state]}
      <span className="text-sm font-medium">{check.label}</span>
      {!folded && check.hint && <span className="field-description">{check.hint}</span>}
    </li>
  );
}

/** a value kept in the window's storage, so it's the same after a restart */
function useStored<T>(key: string, initial: T) {
  const [value, setValue] = useState<T>(() => {
    try {
      return (JSON.parse(localStorage.getItem(key) ?? "null") as T | null) ?? initial;
    } catch {
      return initial;
    }
  });
  const store = (next: T) => {
    localStorage.setItem(key, JSON.stringify(next));
    setValue(next);
  };
  return [value, store] as const;
}

/** what the comparison picked needs, ticked from what the app already knows */
export function ComparisonChecklist({ snapshot }: { snapshot: Snapshot }) {
  const settings = useSettings();
  const [corner, setCorner] = useStored<Corner>("checklist-corner", { right: true, bottom: true });
  const [folded, setFolded] = useStored("checklist-folded", true);
  const kind = settings.display.purpose;
  if (!isCase(kind)) return null;
  const checks = checklist(kind, {
    runs: snapshot.runs,
    config: settings.vision,
    status: snapshot.vision,
    game: shownGame(settings.vision, settings.targets.game)?.name,
    box: snapshot.box,
    side: settings.recording.side,
  });
  return (
    <FloatingPanel label={cases[kind].name} corner={corner} onCornerChange={setCorner}>
      <div className="assist-checklist" data-folded={folded}>
        <div className="assist-head">
          <h3>{cases[kind].name}</h3>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={folded ? "Show the hints" : "Hide the hints"}
            aria-expanded={!folded}
            onClick={() => setFolded(!folded)}
          >
            {folded ? <ChevronUp /> : <ChevronDown />}
          </Button>
        </div>
        <ul>
          {checks.map((check) => (
            <CheckItem key={check.label} check={check} folded={folded} />
          ))}
        </ul>
      </div>
    </FloatingPanel>
  );
}
