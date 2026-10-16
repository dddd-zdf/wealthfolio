// Fork: account rows on the dashboard.
// - 1D: each row's gain sums its holdings' last-session move instead of the
//   period performance summary.
// - Managed accounts (manually priced holdings) get an in-place total editor.
import { getHoldingsList, performanceSummaryScopeKey } from "@/adapters";
import { useHoldings } from "@/hooks/use-holdings";
import { summarizeDayChange } from "@/lib/fork-performance";
import { managedHoldings } from "@/lib/managed-account";
import { performancePeriodPnl, performanceSummaryReturn } from "@/lib/performance";
import { QueryKeys } from "@/lib/query-keys";
import type {
  Account,
  AccountScope,
  CurrentAccountValuation,
  Holding,
  PerformanceSummaryMap,
  PerformanceSummaryScope,
} from "@/lib/types";
import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";

export interface PeriodGain {
  amount: number | null;
  percent: number | null;
}

// Module-level so useQueries keeps the combined result stable between renders.
function combineDayChangeHoldings(results: { data?: Holding[]; isLoading: boolean }[]) {
  return {
    data: results.map((result) => result.data),
    isLoading: results.some((result) => result.isLoading),
  };
}

interface AccountPeriodGainsInput {
  isDayChange: boolean;
  performanceScopes: PerformanceSummaryScope[];
  performanceSummaries: PerformanceSummaryMap | undefined;
  isLoadingPerformanceSummaries: boolean;
  currentAccountValuations: CurrentAccountValuation[] | undefined;
}

/** Gains keyed by `performanceSummaryScopeKey`. */
export function useAccountPeriodGains({
  isDayChange,
  performanceScopes,
  performanceSummaries,
  isLoadingPerformanceSummaries,
  currentAccountValuations,
}: AccountPeriodGainsInput) {
  const dayChangeHoldings = useQueries({
    queries: (isDayChange ? performanceScopes : []).map((scope) => {
      const filter: AccountScope = { type: "accounts", accountIds: scope.accountIds };
      return {
        queryKey: [QueryKeys.HOLDINGS, filter, { includeClosed: false }],
        queryFn: () => getHoldingsList(filter),
      };
    }),
    combine: combineDayChangeHoldings,
  });
  const isLoadingPerformanceQueries = isDayChange
    ? dayChangeHoldings.isLoading
    : isLoadingPerformanceSummaries;

  const periodGains = useMemo((): Record<string, PeriodGain> => {
    const gains: Record<string, PeriodGain> = {};
    if (!isDayChange) {
      for (const [key, perf] of Object.entries(performanceSummaries ?? {})) {
        gains[key] = {
          amount: performancePeriodPnl(perf),
          percent: performanceSummaryReturn(perf),
        };
      }
      return gains;
    }
    const totals = new Map(
      (currentAccountValuations ?? []).map((valuation) => [
        valuation.accountId,
        valuation.totalValueBase,
      ]),
    );
    performanceScopes.forEach((scope, index) => {
      const holdings = dayChangeHoldings.data[index];
      if (!holdings) return;
      const totalValueBase = scope.accountIds.reduce((sum, id) => sum + (totals.get(id) ?? 0), 0);
      const dayChange = summarizeDayChange(holdings, totalValueBase);
      gains[performanceSummaryScopeKey(scope.accountIds)] = {
        amount: dayChange?.amount ?? null,
        percent: dayChange?.percent ?? null,
      };
    });
    return gains;
  }, [
    isDayChange,
    performanceSummaries,
    currentAccountValuations,
    performanceScopes,
    dayChangeHoldings.data,
  ]);

  return { periodGains, isLoadingPerformanceQueries };
}

/** Manually priced holdings per managed account id. */
export function useManagedHoldingsByAccount(accounts: Account[]) {
  const { holdings: allHoldings } = useHoldings({ type: "all" });
  return useMemo(
    () =>
      new Map(
        accounts.map((account) => [
          account.id,
          managedHoldings(allHoldings, account.id, account.currency),
        ]),
      ),
    [accounts, allHoldings],
  );
}

/** Manually priced holdings of one managed account (account page), or null. */
export function useManagedAccountHoldings(account: Account | undefined) {
  const { holdings: allHoldings } = useHoldings({ type: "all" });
  return useMemo(
    () => (account ? managedHoldings(allHoldings, account.id, account.currency) : null),
    [allHoldings, account],
  );
}
