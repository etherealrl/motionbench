import { ArrowDownToLine } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Unavailable } from "@/charts/chart";
import { native } from "@/lib/native";
import type { Comparison } from "./use-comparison";
import { NumbersTable, type Columns } from "./numbers-table";
import { Results } from "./results";

/** saves the shown result with what it came from, nothing while there's none */
export function ExportResult({ comparison }: { comparison: Comparison }) {
  if (!comparison.result) return null;
  return (
    <Button
      variant="outline"
      size="sm"
      disabled={!native}
      onClick={() => void comparison.exportResult()}
    >
      <ArrowDownToLine /> Export analysis
    </Button>
  );
}

/** the Comparison tab: what the judge could separate, and every source recording's numbers.
 * `exporting` puts its export here, for when there's no tab row to hold it */
export function ComparisonTab({
  comparison,
  columns,
  exporting = false,
}: {
  comparison: Comparison;
  columns: Columns | null;
  exporting?: boolean;
}) {
  const { result } = comparison;
  return (
    <div className="comparison-tab">
      {result ? (
        <>
          {exporting && (
            <div className="result-actions">
              <ExportResult comparison={comparison} />
            </div>
          )}
          <Results result={result} />
        </>
      ) : (
        <Unavailable>no judge result for this selection yet</Unavailable>
      )}
      {columns && <NumbersTable columns={columns} />}
    </div>
  );
}
