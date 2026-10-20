// Web currency changes finish rebuilding on the server before settings success.
// Cancel old in-flight responses and reset amounts before applying the new label.
import type { QueryClient } from "@tanstack/react-query";
import { QueryKeys } from "./query-keys";

const currencyKeys = new Set<string>([
  QueryKeys.PERFORMANCE_SUMMARY,
  QueryKeys.PERFORMANCE_HISTORY,
  QueryKeys.ACCOUNTS_SIMPLE_PERFORMANCE,
  QueryKeys.HISTORY_VALUATION,
  QueryKeys.latestValuations,
  QueryKeys.CURRENT_VALUATION,
  QueryKeys.HOLDINGS,
  QueryKeys.HOLDING,
  QueryKeys.ASSET_HOLDINGS,
  QueryKeys.ASSET_LOTS,
  QueryKeys.PORTFOLIO_ALLOCATIONS,
  QueryKeys.HOLDINGS_BY_ALLOCATION,
  QueryKeys.ACCOUNTS_SUMMARY,
  QueryKeys.PORTFOLIO_SUMMARY,
  QueryKeys.NET_WORTH,
  QueryKeys.NET_WORTH_HISTORY,
  QueryKeys.INCOME_SUMMARY,
  QueryKeys.GOALS,
  QueryKeys.GOAL,
  QueryKeys.GOAL_PLAN,
  QueryKeys.GOAL_FUNDING,
  QueryKeys.RETIREMENT_OVERVIEW,
  QueryKeys.SAVE_UP_OVERVIEW,
  QueryKeys.SAVE_UP_PREVIEW,
  QueryKeys.CONTRIBUTION_LIMITS,
  QueryKeys.SPENDING_TRANSACTIONS,
  QueryKeys.SPENDING_REPORT,
  QueryKeys.SPENDING_BUDGET,
  QueryKeys.SPENDING_INSIGHT,
]);

export async function resetWebCurrencyQueries(queryClient: QueryClient): Promise<void> {
  const filter = {
    predicate: (query: { queryKey: readonly unknown[] }) =>
      typeof query.queryKey[0] === "string" && currencyKeys.has(query.queryKey[0]),
  };
  await queryClient.cancelQueries(filter);
  await queryClient.resetQueries(filter);
}

export function isWebCurrencyCompletion(payload: unknown): boolean {
  return (
    typeof payload === "object" &&
    payload !== null &&
    (payload as { baseCurrencyChanged?: unknown }).baseCurrencyChanged === true
  );
}
