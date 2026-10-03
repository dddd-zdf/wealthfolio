import type { Holding } from "@/lib/types";

export type HoldingPerformanceMetric =
  | "unrealizedGain"
  | "realizedGain"
  | "totalGain"
  | "totalReturn";

export type HoldingPerformanceMode = "daily" | "unrealized" | "pnl" | "return";

type HoldingPerformanceValues = Pick<
  Holding,
  "costBasis" | "unrealizedGain" | "realizedGain" | "totalGain" | "totalReturn" | "returnBasis"
>;

type HoldingPerformanceModeValues = HoldingPerformanceValues & Pick<Holding, "dayChangePct">;

function percentFromBasis(amount: number, basis: number): number | null {
  const exposure = Math.abs(basis);
  if (exposure > 0) return amount / exposure;
  return amount === 0 ? 0 : null;
}

/** Returns the FX-inclusive performance percentage for a base-currency amount. */
export function getBaseHoldingPerformancePercent(
  holding: HoldingPerformanceValues,
  metric: HoldingPerformanceMetric,
): number | null {
  if (metric === "unrealizedGain") {
    if (holding.unrealizedGain == null || holding.costBasis == null) return null;
    return percentFromBasis(holding.unrealizedGain.base, holding.costBasis.base);
  }

  if (metric === "realizedGain") {
    if (holding.realizedGain == null || holding.returnBasis == null) return null;
    const disposedBasis = holding.returnBasis.base - (holding.costBasis?.base ?? 0);
    return percentFromBasis(holding.realizedGain.base, disposedBasis);
  }

  const amount = metric === "totalReturn" ? holding.totalReturn : holding.totalGain;
  if (amount == null || holding.returnBasis == null) return null;
  return percentFromBasis(amount.base, holding.returnBasis.base);
}

/** Selects the percentage matching base-currency values for a performance mode. */
export function getBaseHoldingPerformancePercentForMode(
  holding: HoldingPerformanceModeValues,
  mode: HoldingPerformanceMode,
): number | null {
  if (mode === "unrealized") {
    return getBaseHoldingPerformancePercent(holding, "unrealizedGain");
  }
  if (mode === "daily") return holding.dayChangePct ?? null;
  if (mode === "return") {
    return (
      getBaseHoldingPerformancePercent(holding, "totalReturn") ??
      getBaseHoldingPerformancePercent(holding, "totalGain")
    );
  }
  return (
    getBaseHoldingPerformancePercent(holding, "totalGain") ??
    getBaseHoldingPerformancePercent(holding, "unrealizedGain")
  );
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
