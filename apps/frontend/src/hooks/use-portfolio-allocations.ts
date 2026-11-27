import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { AccountScope, PortfolioAllocations } from "@/lib/types";
import { getPortfolioAllocations } from "@/adapters";
import { QueryKeys } from "@/lib/query-keys";

interface UsePortfolioAllocationsOptions {
  enabled?: boolean;
  keepPreviousData?: boolean;
}

export function usePortfolioAllocations(
  accountFilter: AccountScope,
  options: UsePortfolioAllocationsOptions = {},
) {
  const isEnabled =
    (options.enabled ?? true) &&
    (accountFilter.type !== "account" || accountFilter.accountId.trim().length > 0);

  const {
    data: allocations,
    isLoading,
    isError,
    error,
  } = useQuery<PortfolioAllocations, Error>({
    queryKey: [QueryKeys.PORTFOLIO_ALLOCATIONS, accountFilter],
    queryFn: () => getPortfolioAllocations(accountFilter),
    enabled: isEnabled,
    placeholderData: options.keepPreviousData ? keepPreviousData : undefined,
  });

  return { allocations, isLoading, isError, error };
}
