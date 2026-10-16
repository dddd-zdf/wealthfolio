import { renderHook, waitFor } from "@/test/render";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getHoldingsList } from "@/adapters";
import { useAccountPeriodGains } from "./fork-accounts-summary";

vi.mock("@/adapters", () => ({
  getHoldingsList: vi.fn(),
  performanceSummaryScopeKey: (accountIds: string[]) => [...accountIds].sort().join(","),
}));

const mockHoldings = vi.mocked(getHoldingsList);

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

type GainsInput = Parameters<typeof useAccountPeriodGains>[0];

const scopes = [{ accountIds: ["a"] }, { accountIds: ["b"] }] as GainsInput["performanceScopes"];

describe("useAccountPeriodGains", () => {
  beforeEach(() => {
    mockHoldings.mockReset();
  });

  it("on 1D sums each account's holdings' last-session move", async () => {
    mockHoldings.mockImplementation(async (filter) => {
      const ids = (filter as { accountIds: string[] }).accountIds;
      return (
        ids[0] === "a"
          ? [{ dayChange: { local: 30, base: 30 } }, { dayChange: { local: -10, base: -10 } }]
          : [{ dayChange: null }]
      ) as never;
    });
    const { result } = renderHook(
      () =>
        useAccountPeriodGains({
          isDayChange: true,
          performanceScopes: scopes,
          performanceSummaries: undefined,
          isLoadingPerformanceSummaries: false,
          currentAccountValuations: [
            { accountId: "a", totalValueBase: 1020 },
            { accountId: "b", totalValueBase: 500 },
          ] as GainsInput["currentAccountValuations"],
        }),
      { wrapper },
    );

    await waitFor(() => expect(result.current.isLoadingPerformanceQueries).toBe(false));
    expect(result.current.periodGains.a.amount).toBe(20);
    expect(result.current.periodGains.a.percent).toBeCloseTo(20 / 1000);
    // No holding with a day change: no gain to show.
    expect(result.current.periodGains.b).toEqual({ amount: null, percent: null });
  });

  it("outside 1D uses the performance summaries and fetches no holdings", () => {
    const { result } = renderHook(
      () =>
        useAccountPeriodGains({
          isDayChange: false,
          performanceScopes: scopes,
          performanceSummaries: {},
          isLoadingPerformanceSummaries: true,
          currentAccountValuations: undefined,
        }),
      { wrapper },
    );

    expect(result.current.isLoadingPerformanceQueries).toBe(true);
    expect(result.current.periodGains).toEqual({});
    expect(mockHoldings).not.toHaveBeenCalled();
  });
});
