// A web currency change is acknowledged only after the server rebuilds history.
// Cancel old in-flight responses and clear financial amounts before accepting
// the new settings label.
import type { QueryClient } from "@tanstack/react-query";
import { QueryKeys } from "./query-keys";
import type { Settings } from "./types";

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
  QueryKeys.CONTRIBUTION_LIMIT_PROGRESS,
  QueryKeys.ALLOCATION_TARGET_DRIFT,
  QueryKeys.SPENDING_TRANSACTIONS,
  QueryKeys.SPENDING_REPORT,
  QueryKeys.SPENDING_BUDGET,
  QueryKeys.SPENDING_INSIGHT,
  QueryKeys.SPENDING_EVENTS,
  QueryKeys.SNAPSHOT_HOLDINGS,
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

/** Protect a tab that missed SSE and discovers the change on settings refetch. */
export async function fetchWebCurrencySettings(
  queryClient: QueryClient,
  fetchSettings: () => Promise<Settings>,
): Promise<Settings> {
  const next = await fetchSettings();
  const previous = queryClient.getQueryData<Settings>([QueryKeys.SETTINGS]);
  if (previous && previous.baseCurrency !== next.baseCurrency) {
    await resetWebCurrencyQueries(queryClient);
  }
  return next;
}
