// Fork: dashboard behaviour attached to upstream's DashboardContent through
// hooks, so the upstream component keeps its own data flow and markup.
// - 1D: the headline gain sums each holding's last-session move (stored daily
//   totals stay flat over weekends and holidays).
// - 1D/1W: the chart uses intraday values, with a faint guide where the period started.
// - Other periods: money-weighted return beside the simple return.
import { getIntradayValuations, isWeb } from "@/adapters";
import type { HistoryChartData } from "@/components/history-chart";
import { performanceMoneyWeightedReturn, summarizeDayChange } from "@/lib/fork-performance";
import { performancePeriodPnl, performanceSummaryReturn } from "@/lib/performance";
import { QueryKeys } from "@/lib/query-keys";
import type { AccountScope, Holding } from "@/lib/types";
import { useQuery } from "@tanstack/react-query";
import type { TimePeriod as UITimePeriod } from "@wealthfolio/ui";
import { useMemo } from "react";

// Regular North American session (9:30-16:00 ET); bars exclude extended hours.
const REGULAR_SESSION_MS = 6.5 * 60 * 60 * 1000;

type PerformanceSummary = Parameters<typeof performancePeriodPnl>[0];
type DayChange = ReturnType<typeof summarizeDayChange>;

export function getDashboardChartGain(
  point: HistoryChartData,
  first: HistoryChartData,
  referenceValue: number | undefined,
): { amount: number | null; percent: number | null } {
  if (point.currency !== first.currency) return { amount: null, percent: null };
  const startValue = referenceValue ?? first.totalValue;
  const netDeposits = point.netContribution - first.netContribution;
  const amount = point.totalValue - startValue - netDeposits;
  return { amount, percent: startValue > 0 ? amount / startValue : null };
}

interface DashboardGainInput {
  isDayChange: boolean;
  holdings: Holding[] | undefined;
  totalValue: number;
  performance: PerformanceSummary;
  isPerformanceLoading: boolean;
  isHoldingsLoading: boolean;
  isHoldingsComplete: boolean;
  isCurrentValuationLoading: boolean;
}

export function useDashboardGain({
  isDayChange,
  holdings,
  totalValue,
  performance,
  isPerformanceLoading,
  isHoldingsLoading,
  isHoldingsComplete,
  isCurrentValuationLoading,
}: DashboardGainInput) {
  const dayChange = useMemo(
    () =>
      isDayChange && isHoldingsComplete ? summarizeDayChange(holdings ?? [], totalValue) : null,
    [isDayChange, isHoldingsComplete, holdings, totalValue],
  );
  return {
    dayChange,
    gainLossAmount: isDayChange ? (dayChange?.amount ?? null) : performancePeriodPnl(performance),
    simpleReturn: isDayChange
      ? (dayChange?.percent ?? null)
      : performanceSummaryReturn(performance),
    moneyWeightedReturn: isDayChange ? null : performanceMoneyWeightedReturn(performance),
    isGainLoading: isDayChange
      ? isHoldingsLoading || isCurrentValuationLoading
      : isPerformanceLoading,
  };
}

interface DashboardChartInput {
  selectedInterval: UITimePeriod | null;
  scope: AccountScope;
  scopeKey: string;
  totalValue: number;
  baseCurrency: string;
  dailyChartData: HistoryChartData[];
  dayChange: DayChange | null;
  isAllTime: boolean;
}

export function useDashboardChart({
  selectedInterval,
  scope,
  scopeKey,
  totalValue,
  baseCurrency,
  dailyChartData,
  dayChange,
  isAllTime,
}: DashboardChartInput) {
  // The web 1D chart can show one observed point as a dot.
  const intradayRange =
    selectedInterval === "1D" || selectedInterval === "1W" ? selectedInterval : null;
  const minimumPoints = isWeb && intradayRange === "1D" ? 1 : 2;
  const { data: intradayPoints } = useQuery({
    // Include the display/base currency so a settings transition cannot reuse
    // a curve calculated for the previous currency and merely relabel it.
    queryKey: [
      QueryKeys.HISTORY_VALUATION,
      "intraday",
      scopeKey,
      ...(isWeb ? [baseCurrency] : []),
      intradayRange,
    ],
    queryFn: () => getIntradayValuations(scope, intradayRange!),
    enabled: intradayRange !== null,
    staleTime: 5 * 60 * 1000,
    refetchInterval: 5 * 60 * 1000,
    retry: 1,
  });

  const chartData = useMemo(() => {
    if (intradayRange && intradayPoints && intradayPoints.length >= minimumPoints) {
      // Native curves are reconstructed from daily anchors. Web points are
      // observed values and keep their original amounts.
      const last = intradayPoints[intradayPoints.length - 1];
      const lastDay = last.timestamp.slice(0, 10);
      const offset = !isWeb && totalValue > 0 ? totalValue - last.totalValueBase : 0;
      return intradayPoints.map((point) => ({
        date: point.timestamp,
        totalValue: point.totalValueBase + (point.timestamp.startsWith(lastDay) ? offset : 0),
        netContribution: point.netContributionBase,
        currency: baseCurrency,
      }));
    }
    return dailyChartData;
  }, [intradayRange, intradayPoints, minimumPoints, totalValue, dailyChartData, baseCurrency]);

  // 1D shows a single session: span all of it so a half-finished day fills half
  // the chart instead of stretching across the full width.
  const timeDomain = useMemo((): [number, number] | undefined => {
    if (selectedInterval !== "1D" || !intradayPoints || intradayPoints.length < minimumPoints) {
      return undefined;
    }
    const open = Date.parse(intradayPoints[0].timestamp);
    const lastPoint = Date.parse(intradayPoints[intradayPoints.length - 1].timestamp);
    return [open, Math.max(open + REGULAR_SESSION_MS, lastPoint)];
  }, [selectedInterval, intradayPoints, minimumPoints]);

  // Faint guide at where the period started: yesterday's close on 1D (live
  // total minus today's move), the first point otherwise. ALL starts near zero.
  const referenceValue =
    selectedInterval === "1D"
      ? timeDomain && dayChange
        ? totalValue - dayChange.amount
        : undefined
      : !isAllTime && chartData.length > 1
        ? chartData[0].totalValue
        : undefined;

  return {
    chartData,
    timeDomain,
    referenceValue,
  };
}
