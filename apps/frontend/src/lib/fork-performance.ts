import { shouldDisplayAnnualizedPerformanceReturn } from "@/lib/performance";
import type { Holding, PerformanceResult } from "@/lib/types";

const numberOrNull = (value: number | null | undefined): number | null =>
  value == null || !Number.isFinite(Number(value)) ? null : Number(value);

export interface MoneyWeightedReturnDisplay {
  value: number;
  annualized: boolean;
}

/**
 * Money-weighted return for a headline: the period figure for ranges under a
 * year, the annualized figure for longer ones (a multi-year total is hard to read).
 */
export function performanceMoneyWeightedReturn(
  result: PerformanceResult | null | undefined,
): MoneyWeightedReturnDisplay | null {
  if (!result) return null;
  if (shouldDisplayAnnualizedPerformanceReturn(result)) {
    const annualized = numberOrNull(result.returns?.annualizedIrr);
    return annualized == null ? null : { value: annualized, annualized: true };
  }
  const period = numberOrNull(result.returns?.irr);
  return period == null ? null : { value: period, annualized: false };
}

export interface DayChangeSummary {
  amount: number;
  percent: number | null;
}

/**
 * Sums each holding's own last-session move (latest close vs the close before
 * it, dated by its exchange), so weekends and holidays show the last trading
 * day and each market counts its own session. The percent is against the
 * scope's value before that move. Null when no holding has a day change.
 */
export function summarizeDayChange(
  holdings: Pick<Holding, "dayChange">[],
  totalValueBase: number,
): DayChangeSummary | null {
  let amount = 0;
  let hasDayChange = false;
  for (const holding of holdings) {
    const change = holding.dayChange?.base;
    if (change == null) continue;
    amount += change;
    hasDayChange = true;
  }
  if (!hasDayChange) return null;
  const previousValue = totalValueBase - amount;
  return { amount, percent: previousValue > 0 ? amount / previousValue : null };
}
