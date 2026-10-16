// Fork: small components used by the dashboard's fork hooks (see
// fork-dashboard.ts and fork-accounts-summary.ts).
import { ManagedValueEdit } from "@/components/managed-value-edit";
import type { performanceMoneyWeightedReturn } from "@/lib/fork-performance";
import type { Holding } from "@/lib/types";
import { GainPercent, PrivacyAmount } from "@wealthfolio/ui";
import { useTranslation } from "react-i18next";

type MoneyWeightedReturn = ReturnType<typeof performanceMoneyWeightedReturn>;

export function DashboardMoneyWeightedReturn({ value }: { value: MoneyWeightedReturn }) {
  const { t } = useTranslation();
  if (value == null) return null;
  return (
    <>
      <div className="border-secondary my-1 border-r pr-2" />
      <span className="lg:text-md text-muted-foreground text-sm font-light">
        {value.annualized
          ? t("performance:metric.ann_mwr_short")
          : t("performance:metric.mwr_short")}
      </span>
      <GainPercent className="lg:text-md !ml-1.5 text-sm font-light" value={value.value} />
    </>
  );
}

export function AccountTotalValue({
  managed,
  value,
  currency,
}: {
  managed?: Holding[] | null;
  value: number;
  currency: string;
}) {
  return managed ? (
    <ManagedValueEdit holdings={managed} value={value} currency={currency} />
  ) : (
    <PrivacyAmount value={value} currency={currency} />
  );
}
