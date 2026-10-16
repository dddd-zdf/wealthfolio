import type { DateRange } from "@/lib/types";
import Balance from "@/pages/dashboard/balance";
import { PrivacyAmount } from "@wealthfolio/ui";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { calendarDaysBetweenInclusive, localDateParts } from "../lib/timezone";

interface SpendingTotalProps {
  isLoading: boolean;
  totalSpending: number;
  currency: string;
  dateRange: DateRange | undefined;
}

/** Fork: the period's spending total, with the monthly average beside it. */
export function SpendingTotal({
  isLoading,
  totalSpending,
  currency,
  dateRange,
}: SpendingTotalProps) {
  const { t } = useTranslation();
  // Average per month for ranges longer than a month, normalized by calendar
  // length so partial months (e.g. YTD) don't skew it. Month-or-shorter
  // selections are already monthly, so they skip it.
  const monthlyAvgSpending = useMemo(() => {
    if (!dateRange?.from || !dateRange?.to) return null;
    const days = calendarDaysBetweenInclusive(
      localDateParts(dateRange.from),
      localDateParts(dateRange.to),
    );
    if (days <= 31) return null;
    return totalSpending / (days / (365.25 / 12));
  }, [dateRange, totalSpending]);

  return (
    <div className="flex flex-wrap items-baseline gap-x-3">
      <Balance
        isLoading={isLoading}
        targetValue={totalSpending}
        currency={currency}
        displayCurrency={true}
      />
      {!isLoading && monthlyAvgSpending !== null && (
        <span className="text-muted-foreground text-sm tabular-nums">
          <PrivacyAmount value={monthlyAvgSpending} currency={currency} />{" "}
          {t("spending:tabContent.monthlyAvg")}
        </span>
      )}
    </div>
  );
}
