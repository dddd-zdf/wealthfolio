import { HoldingType } from "@/lib/constants";
import type { Holding } from "@/lib/types";

// Aggregated holdings list their accounts; account-scoped ones leave it empty.
const sourceAccounts = (holding: Holding) =>
  holding.sourceAccountIds?.length ? holding.sourceAccountIds : [holding.accountId];

// Wealthsimple private-market funds (WSE200, WSE401P, ...). Their prices are
// saved weekly by the Wealthsimple sync, so their accounts get no pencil.
const isSyncedFund = (holding: Holding) => /^WSE\d+P?$/.test(holding.instrument?.symbol ?? "");

/**
 * Manually priced holdings of an account whose value is entered as one total
 * (e.g. a pension plan or private-market fund). `holdings` is the all-accounts
 * list, so holdings shared with another account can be detected. Returns null
 * when the account has none, or when a manual price would also move another
 * account or can't be expressed in the account's currency, or when the account
 * holds a fund the Wealthsimple sync prices.
 */
export function managedHoldings(
  holdings: Holding[],
  accountId: string,
  accountCurrency: string,
): Holding[] | null {
  const manual = holdings.filter(
    (holding) =>
      holding.holdingType !== HoldingType.CASH &&
      holding.quantity > 0 &&
      holding.instrument?.quoteMode === "MANUAL" &&
      sourceAccounts(holding).includes(accountId),
  );
  if (manual.length === 0 || manual.some(isSyncedFund)) return null;
  const isolated = manual.every(
    (holding) => sourceAccounts(holding).length === 1 && holding.localCurrency === accountCurrency,
  );
  return isolated ? manual : null;
}

export interface ManagedPrice {
  assetId: string;
  price: number;
  currency: string;
}

/**
 * Prices that bring the account from `currentTotal` to `newTotal`. Cash and
 * market-priced holdings keep their value; manual holdings all move by the
 * same factor. Null when the result would not be a positive price.
 */
export function managedPrices(
  holdings: Holding[],
  currentTotal: number,
  newTotal: number,
): ManagedPrice[] | null {
  const manualValue = holdings.reduce((sum, holding) => sum + holding.marketValue.local, 0);
  const fixedValue = currentTotal - manualValue;
  if (manualValue <= 0 || newTotal - fixedValue <= 0) return null;
  const factor = (newTotal - fixedValue) / manualValue;
  return holdings.map((holding) => ({
    assetId: holding.instrument?.id ?? "",
    price: (holding.marketValue.local / holding.quantity) * factor,
    currency: holding.localCurrency,
  }));
}
