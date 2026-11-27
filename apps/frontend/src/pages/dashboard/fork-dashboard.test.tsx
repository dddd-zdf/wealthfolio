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
    isAllTime: false,
    ...overrides,
  } as ChartInput;
}

describe("useDashboardChart", () => {
  beforeEach(() => {
    mockIntraday.mockReset();
  });

  it("draws recorded 1D values as observed, spanning the whole session", async () => {
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
          }),
        ),
      { wrapper },
    );

    await waitFor(() => expect(result.current.chartData).toHaveLength(2));
    expect(result.current.chartData.map((p) => p.totalValue)).toEqual([1000, 1004]);
    const open = Date.parse("2026-10-15T13:30:00Z");
    expect(result.current.timeDomain).toEqual([open, open + 6.5 * 60 * 60 * 1000]);
    expect(mockIntraday).toHaveBeenCalledWith({ type: "all" }, "1D");
  });

  it("shows the first 1D snapshot", async () => {
    const timestamp = "2026-10-15T13:30:30Z";
    mockIntraday.mockResolvedValue([bar(timestamp, 1000)]);
    const { result } = renderHook(
      () => useDashboardChart(input({ selectedInterval: "1D", totalValue: 1010 })),
      { wrapper },
    );

    await waitFor(() => expect(result.current.chartData).toHaveLength(1));
    expect(result.current.chartData[0]).toMatchObject({ date: timestamp, totalValue: 1000 });
    expect(result.current.timeDomain).toEqual([
      Date.parse("2026-10-15T13:30:00Z"),
      Date.parse("2026-10-15T20:00:00Z"),
    ]);
  });

  it.each([
    ["2026-11-19T20:56:03Z", "2026-11-19T21:00:00Z"],
    ["2026-11-19T21:06:05Z", "2026-11-19T21:06:05Z"],
  ])("keeps market open on the axis when the first snapshot is late (%s)", async (last, end) => {
    const first = "2026-11-19T18:36:07Z";
    mockIntraday.mockResolvedValue([bar(first, 1000), bar(last, 1004)]);
    const { result } = renderHook(() => useDashboardChart(input({ selectedInterval: "1D" })), {
      wrapper,
    });

    await waitFor(() =>
      expect(result.current.chartData.map((point) => point.date)).toEqual([first, last]),
    );
    expect(result.current.timeDomain).toEqual([
      Date.parse("2026-11-19T14:30:00Z"),
      Date.parse(end),
    ]);
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
  });

  it("draws no period-start line for all time", () => {
    const { result } = renderHook(
      () => useDashboardChart(input({ selectedInterval: "ALL", isAllTime: true })),
      { wrapper },
    );

    expect(result.current.referenceValue).toBeUndefined();
  });
});
