import { renderHook, waitFor } from "@/test/render";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getIntradayValuations } from "@/adapters";
import { useDashboardChart } from "./fork-dashboard";

vi.mock("@/adapters", () => ({
  getIntradayValuations: vi.fn(),
  isWeb: true,
}));

const mockIntraday = vi.mocked(getIntradayValuations);

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

const daily = [
  { date: "2026-10-13", totalValue: 1000, netContribution: 900, currency: "CAD" },
  { date: "2026-10-14", totalValue: 1010, netContribution: 900, currency: "CAD" },
  { date: "2026-10-15", totalValue: 1020, netContribution: 900, currency: "CAD" },
];

const bar = (timestamp: string, totalValueBase: number) => ({
  timestamp,
  totalValueBase,
  netContributionBase: 900,
});

type ChartInput = Parameters<typeof useDashboardChart>[0];

function input(overrides: Partial<ChartInput>): ChartInput {
  return {
    selectedInterval: "3M",
    scope: { type: "all" },
    scopeKey: "all",
    totalValue: 1020,
    baseCurrency: "CAD",
    dailyChartData: daily,
    dayChange: null,
    isAllTime: false,
    ...overrides,
  } as ChartInput;
}

describe("useDashboardChart", () => {
  beforeEach(() => {
    mockIntraday.mockReset();
  });

  it("draws 1D from intraday bars ending on the live total, spanning the whole session", async () => {
    mockIntraday.mockResolvedValue([
      bar("2026-10-15T13:30:00Z", 1000),
      bar("2026-10-15T15:30:00Z", 1004),
    ] as never);
    const { result } = renderHook(
      () =>
        useDashboardChart(
          input({
            selectedInterval: "1D",
            totalValue: 1010,
            dayChange: { amount: 12, percent: 0.012 } as never,
          }),
        ),
      { wrapper },
    );

    await waitFor(() => expect(result.current.chartData).toHaveLength(2));
    // The latest session is shifted so it ends on the headline value.
    expect(result.current.chartData.map((p) => p.totalValue)).toEqual([1006, 1010]);
    const open = Date.parse("2026-10-15T13:30:00Z");
    expect(result.current.timeDomain).toEqual([open, open + 6.5 * 60 * 60 * 1000]);
    // Yesterday's close: the live total minus today's move.
    expect(result.current.referenceValue).toBe(998);
    expect(mockIntraday).toHaveBeenCalledWith({ type: "all" }, "1D");
    expect(result.current.isIntradayEstimate).toBe(true);
  });

  it("falls back to daily points when intraday has too few bars", async () => {
    mockIntraday.mockResolvedValue([bar("2026-10-15T13:30:00Z", 1000)] as never);
    const { result } = renderHook(() => useDashboardChart(input({ selectedInterval: "1W" })), {
      wrapper,
    });

    await waitFor(() => expect(mockIntraday).toHaveBeenCalled());
    expect(result.current.chartData).toBe(daily);
    expect(result.current.timeDomain).toBeUndefined();
    expect(result.current.referenceValue).toBe(1000);
  });

  it("marks the period start on longer periods without fetching intraday bars", () => {
    const { result } = renderHook(() => useDashboardChart(input({ selectedInterval: "3M" })), {
      wrapper,
    });

    expect(result.current.chartData).toBe(daily);
    expect(result.current.referenceValue).toBe(1000);
    expect(mockIntraday).not.toHaveBeenCalled();
    expect(result.current.isIntradayEstimate).toBe(false);
  });

  it("draws no period-start line for all time", () => {
    const { result } = renderHook(
      () => useDashboardChart(input({ selectedInterval: "ALL", isAllTime: true })),
      { wrapper },
    );

    expect(result.current.referenceValue).toBeUndefined();
  });
});
