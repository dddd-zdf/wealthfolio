import type { Quote } from "@/lib/types";
import { parseLocalDate } from "@/lib/utils";
import { format } from "date-fns";

export interface QuoteEntry {
  id: string;
  date: Date;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  currency: string;
  isNew?: boolean;
}

/** Preserve source precision in edit state; formatting belongs at render time. */
export function toQuoteEntry(quote: Quote): QuoteEntry {
  return {
    id: quote.id,
    date: parseLocalDate(quote.timestamp),
    open: quote.open,
    high: quote.high,
    low: quote.low,
    close: quote.close,
    volume: Math.round(quote.volume),
    currency: quote.currency,
    isNew: false,
  };
}

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
