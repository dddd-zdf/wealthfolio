import type { PerformanceResult } from "@/lib/types";
import { describe, expect, it } from "vitest";

import { performanceMoneyWeightedReturn, summarizeDayChange } from "./fork-performance";

const baseResult = (): PerformanceResult => ({
  scope: { id: "account-1", currency: "CAD" },
  period: { startDate: "2026-01-01", endDate: "2026-01-31" },
  mode: "valueReturn",
  returns: { valueReturn: null },
  attribution: {
    contributions: 0,
    distributions: 0,
    income: 0,
    realizedPnl: 0,
    unrealizedPnlChange: 0,
    fxEffect: 0,
    fees: 0,
    taxes: 0,
    residual: 0,
  },
  risk: {},
  dataQuality: {
    status: "partial",
    warnings: [],
    notApplicableReasons: [],
  },
  basisStatus: "complete",
  summary: {
    amount: null,
    percent: null,
    method: "valueReturn",
    basis: "bookBasis",
    quality: "partial",
    amountStatus: "unavailable",
    percentStatus: "unavailable",
    basisStatus: "complete",
    reasons: [],
  },
  series: [],
});

describe("performanceMoneyWeightedReturn", () => {
  it("returns the period IRR for ranges under a year", () => {
    const result = baseResult();
    result.returns = { irr: 0.042, annualizedIrr: 0.09 };

    expect(performanceMoneyWeightedReturn(result)).toEqual({ value: 0.042, annualized: false });
  });

  it("returns the annualized IRR for ranges of a year or more", () => {
    const result = baseResult();
    result.period = { startDate: "2022-01-01", endDate: "2026-01-01" };
    result.returns = { irr: 0.45, annualizedIrr: 0.0858 };

    expect(performanceMoneyWeightedReturn(result)).toEqual({ value: 0.0858, annualized: true });
  });

  it("returns null when IRR is unavailable", () => {
    expect(performanceMoneyWeightedReturn(baseResult())).toBeNull();
    expect(performanceMoneyWeightedReturn(null)).toBeNull();
  });
});

describe("summarizeDayChange", () => {
  it("sums each holding's last-session move against the value before it", () => {
    // Over a weekend each holding still carries its last trading day's move.
    const holdings = [
      { dayChange: { local: 100, base: 100 } },
      { dayChange: { local: -50, base: -40 } },
      { dayChange: null }, // e.g. cash or a manual asset
    ];

    const summary = summarizeDayChange(holdings, 10060);

    expect(summary?.amount).toBe(60);
    expect(summary?.percent).toBeCloseTo(60 / 10000);
  });

  it("is unavailable when no holding has a day change", () => {
    expect(summarizeDayChange([{ dayChange: null }, {}], 5000)).toBeNull();
  });

  it("has no percent when the prior value is not positive", () => {
    expect(summarizeDayChange([{ dayChange: { local: 10, base: 10 } }], 10)).toEqual({
      amount: 10,
      percent: null,
    });
  });
});
