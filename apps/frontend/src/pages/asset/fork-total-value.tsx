import type { Quote } from "@/lib/types";
import { format } from "date-fns";
import { useCallback, useState } from "react";
import type { QuoteEntry } from "./quote-history-utils";
import { UpdateTotalValueDialog } from "./update-total-value-dialog";

// Generate a temporary ID for new entries
export const generateTempId = () => `temp-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;

// Convert QuoteEntry back to Quote for saving
export const toQuote = (entry: QuoteEntry, assetId: string): Quote => {
  const datePart = format(entry.date, "yyyy-MM-dd").replace(/-/g, "");
  return {
    id: entry.id.startsWith("temp-") ? `${datePart}_${assetId.toUpperCase()}` : entry.id,
    createdAt: new Date().toISOString(),
    dataSource: "MANUAL",
    timestamp: format(entry.date, "yyyy-MM-dd'T'00:00:00'Z'"),
    assetId: assetId,
    open: entry.open,
    high: entry.high,
    low: entry.low,
    close: entry.close,
    volume: entry.volume,
    adjclose: entry.close,
    currency: entry.currency,
  };
};

interface TotalValueEntryOptions {
  isManualDataSource: boolean;
  /** Units held at the end of a day (yyyy-MM-dd); enables entering a total value. */
  unitsOn?: (day: string) => number;
  initialEntries: QuoteEntry[];
  onSaveQuote: (quote: Quote) => Promise<void>;
  currency: string;
  assetId: string;
}

/**
 * The quote grid's "Update value" action: saves the price that values the
 * held units at the entered total. `onUpdateValue` is undefined when hidden.
 */
export function useTotalValueEntry({
  isManualDataSource,
  unitsOn,
  initialEntries,
  onSaveQuote,
  currency,
  assetId,
}: TotalValueEntryOptions) {
  const [totalValueOpen, setTotalValueOpen] = useState(false);
  const handleTotalValue = useCallback(
    (date: Date, price: number) => {
      const day = format(date, "yyyy-MM-dd");
      const existing = initialEntries.find((e) => format(e.date, "yyyy-MM-dd") === day);
      onSaveQuote(
        toQuote(
          {
            id: existing?.id ?? generateTempId(),
            date,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 0,
            currency,
          },
          assetId,
        ),
      );
    },
    [initialEntries, onSaveQuote, currency, assetId],
  );
  const showUpdateValue = isManualDataSource && unitsOn != null;
  const totalValueDialog = showUpdateValue ? (
    <UpdateTotalValueDialog
      open={totalValueOpen}
      onOpenChange={setTotalValueOpen}
      currency={currency}
      unitsOn={unitsOn}
      onSave={handleTotalValue}
    />
  ) : null;

  return {
    onUpdateValue: showUpdateValue ? () => setTotalValueOpen(true) : undefined,
    totalValueDialog,
  };
}
