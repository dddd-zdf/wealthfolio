import type { PerformanceResult } from "@/lib/types";

const numberOrNull = (value: number | null | undefined): number | null =>
  value == null || !Number.isFinite(Number(value)) ? null : Number(value);

const MS_PER_DAY = 24 * 60 * 60 * 1000;
const ANNUALIZED_DISPLAY_MIN_DAYS = 365;

function dateKeyToUtcMs(value: string | null | undefined): number | null {
  if (!value) return null;

  const [dateKey] = value.split("T");
  const [year, month, day] = dateKey.split("-").map(Number);
  if (!Number.isInteger(year) || !Number.isInteger(month) || !Number.isInteger(day)) return null;

  const timestamp = Date.UTC(year, month - 1, day);
  if (!Number.isFinite(timestamp)) return null;

  const date = new Date(timestamp);
  if (
    date.getUTCFullYear() !== year ||
    date.getUTCMonth() !== month - 1 ||
    date.getUTCDate() !== day
  ) {
    return null;
  }

  return timestamp;
}

export function performancePeriodPnl(result: PerformanceResult | null | undefined): number | null {
  if (!result || result.summary?.amountStatus !== "complete") return null;
  return numberOrNull(result.summary.amount);
}

export function performanceSummaryReturn(
  result: PerformanceResult | null | undefined,
): number | null {
  if (!result || result.summary?.percentStatus !== "complete") return null;
  return numberOrNull(result.summary.percent);
}

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

export function simpleReturnFromNetContribution(
  returnAmount: number | null | undefined,
  netContribution: number | null | undefined,
): number | null {
  const amount = numberOrNull(returnAmount);
  const contribution = numberOrNull(netContribution);
  if (amount == null || contribution == null || contribution <= 0) return null;

  return amount / contribution;
}

export function shouldDisplayAnnualizedPerformanceReturn(
  result: PerformanceResult | null | undefined,
): boolean {
  const start = dateKeyToUtcMs(result?.period.startDate);
  const end = dateKeyToUtcMs(result?.period.endDate);
  if (start == null || end == null || end < start) return false;

  return (end - start) / MS_PER_DAY >= ANNUALIZED_DISPLAY_MIN_DAYS;
}
