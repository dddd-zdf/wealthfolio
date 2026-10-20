// Fork: 1D/1W asset price chart from live intraday bars, like the dashboard
// (see pages/dashboard/fork-dashboard.ts). Stored quotes are one per day, so
// short ranges would otherwise be a straight line.
// - 1D spans the whole session and marks the previous close.
// - 1W marks where the week started.
// Falls back to the stored daily quotes when no bars are available.
import { getAssetIntradayQuotes } from "@/adapters";
import { QueryKeys } from "@/lib/query-keys";
import type { Quote, TimePeriod } from "@/lib/types";
import { useQuery } from "@tanstack/react-query";
import { format } from "date-fns";
import { useMemo } from "react";

// Regular North American session (9:30-16:00 ET); bars exclude extended hours.
const REGULAR_SESSION_MS = 6.5 * 60 * 60 * 1000;

export interface AssetIntradayPoint {
  timestamp: string;
  totalValue: number;
  currency: string;
}

export interface AssetIntradayChart {
  points: AssetIntradayPoint[];
  /** 1D only: [session open, session end] in epoch ms. */
  timeDomain?: [number, number];
  /** Where the period started: previous close on 1D, first bar on 1W. */
  referenceValue?: number;
}

export function useAssetIntradayChart({
  assetId,
  interval,
  quoteHistory,
  currency,
}: {
  assetId: string;
  interval: TimePeriod;
  quoteHistory: Quote[];
  currency: string;
}): AssetIntradayChart | null {
  const range = interval === "1D" || interval === "1W" ? interval : null;
  const { data: bars } = useQuery({
    queryKey: [QueryKeys.QUOTE_HISTORY, "intraday", assetId, range],
    queryFn: () => getAssetIntradayQuotes(assetId, range!),
    enabled: range !== null && assetId.length > 0,
    staleTime: 5 * 60 * 1000,
    refetchInterval: 5 * 60 * 1000,
    retry: 1,
  });

  return useMemo(
    () => (range && bars ? buildAssetIntradayChart(range, bars, quoteHistory, currency) : null),
    [range, bars, quoteHistory, currency],
  );
}

export function buildAssetIntradayChart(
  range: "1D" | "1W",
  bars: Quote[],
  quoteHistory: Quote[],
  currency: string,
): AssetIntradayChart | null {
  if (bars.length < 2) return null;
  const points = bars.map((bar) => ({
    timestamp: bar.timestamp,
    totalValue: bar.close,
    currency: bar.currency || currency,
  }));

  if (range === "1W") {
    return { points, referenceValue: points[0].totalValue };
  }

  const open = Date.parse(points[0].timestamp);
  const lastPoint = Date.parse(points[points.length - 1].timestamp);
  // Previous close: the last stored daily quote before the session's day.
  const sessionDay = format(new Date(open), "yyyy-MM-dd");
  const previousClose = quoteHistory
    .filter((quote) => quote.timestamp.slice(0, 10) < sessionDay)
    .reduce<
      Quote | undefined
    >((latest, quote) => (!latest || quote.timestamp > latest.timestamp ? quote : latest), undefined)?.close;

  return {
    points,
    timeDomain: [open, Math.max(open + REGULAR_SESSION_MS, lastPoint)],
    referenceValue: previousClose,
  };
}
