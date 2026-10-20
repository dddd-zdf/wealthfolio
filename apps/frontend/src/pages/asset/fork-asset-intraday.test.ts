import type { Quote } from "@/lib/types";
import { describe, expect, it } from "vitest";
import { buildAssetIntradayChart } from "./fork-asset-intraday";

function quote(timestamp: string, close: number): Quote {
  return {
    id: timestamp,
    createdAt: timestamp,
    dataSource: "YAHOO",
    timestamp,
    assetId: "AAPL",
    open: close,
    high: close,
    low: close,
    volume: 0,
    close,
    adjclose: close,
    currency: "USD",
  };
}

const bars = [
  quote("2026-10-19T13:30:00Z", 101),
  quote("2026-10-19T13:35:00Z", 102),
  quote("2026-10-19T15:00:00Z", 103),
];

describe("buildAssetIntradayChart", () => {
  it("marks the previous stored close and spans the session on 1D", () => {
    const daily = [
      quote("2026-10-15T00:00:00Z", 95),
      quote("2026-10-16T00:00:00Z", 100),
      quote("2026-10-19T00:00:00Z", 103),
    ];
    const chart = buildAssetIntradayChart("1D", bars, daily, "USD");

    expect(chart?.points.map((p) => p.totalValue)).toEqual([101, 102, 103]);
    expect(chart?.referenceValue).toBe(100);
    const open = Date.parse("2026-10-19T13:30:00Z");
    expect(chart?.timeDomain).toEqual([open, open + 6.5 * 60 * 60 * 1000]);
    expect(chart?.headline.ganAmount).toBe(3);
    expect(chart?.headline.percentage).toBeCloseTo(0.03);
    expect(chart?.headline.calculatedAt).toBe("2026-10-19T15:00:00Z");
  });

  it("starts 1W at the first bar on an evenly spaced axis", () => {
    const chart = buildAssetIntradayChart("1W", bars, [], "USD");

    expect(chart?.referenceValue).toBe(101);
    expect(chart?.timeDomain).toBeUndefined();
    expect(chart?.headline.ganAmount).toBe(2);
  });

  it("measures 1D from the first bar without an earlier stored close", () => {
    const chart = buildAssetIntradayChart("1D", bars, [], "USD");

    expect(chart?.referenceValue).toBeUndefined();
    expect(chart?.headline.ganAmount).toBe(2);
  });

  it("falls back to daily quotes without enough bars", () => {
    expect(buildAssetIntradayChart("1D", bars.slice(0, 1), [], "USD")).toBeNull();
  });
});
