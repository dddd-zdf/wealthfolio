import { HoldingType } from "@/lib/constants";
import type { Holding } from "@/lib/types";
import { describe, expect, it } from "vitest";
import { managedHoldings, managedPrices } from "./managed-account";

const holding = (
  id: string,
  value: number,
  overrides: Partial<Holding> & { quoteMode?: "MANUAL" | "MARKET" } = {},
): Holding => {
  const { quoteMode = "MANUAL", ...rest } = overrides;
  return {
    id,
    holdingType: HoldingType.SECURITY,
    accountId: "PORTFOLIO",
    sourceAccountIds: ["rpp"],
    instrument: { id, symbol: id, currency: "CAD", quoteMode },
    quantity: 10,
    localCurrency: "CAD",
    baseCurrency: "CAD",
    marketValue: { local: value, base: value },
    weight: 1,
    asOfDate: "2026-10-07",
    ...rest,
  } as Holding;
};

describe("managedHoldings", () => {
  it("returns the account's manually priced holdings", () => {
    const fund = holding("ML8321", 100);
    const etf = holding("XIC", 50, { quoteMode: "MARKET" });
    const other = holding("WSE401", 80, { sourceAccountIds: ["tfsa"] });
    expect(managedHoldings([fund, etf, other], "rpp", "CAD")).toEqual([fund]);
  });

  it("is null without manual holdings, or when one is shared or in another currency", () => {
    expect(managedHoldings([holding("XIC", 50, { quoteMode: "MARKET" })], "rpp", "CAD")).toBeNull();
    const shared = holding("ML8321", 100, { sourceAccountIds: ["rpp", "rrsp"] });
    expect(managedHoldings([shared], "rpp", "CAD")).toBeNull();
    expect(
      managedHoldings([holding("ML8321", 100, { localCurrency: "USD" })], "rpp", "CAD"),
    ).toBeNull();
  });

  it("accepts account-scoped holdings", () => {
    const fund = holding("ML8321", 100, { accountId: "rpp", sourceAccountIds: [] });
    expect(managedHoldings([fund], "rpp", "CAD")).toEqual([fund]);
  });
});

describe("managedPrices", () => {
  it("keeps cash fixed and prices a single fund to hit the total", () => {
    // 100 in the fund + 20 cash; new total 140 means the fund is worth 120.
    expect(managedPrices([holding("ML8321", 100)], 120, 140)).toEqual([
      { assetId: "ML8321", price: 12, currency: "CAD" },
    ]);
  });

  it("scales several funds by the same factor", () => {
    const prices = managedPrices([holding("A", 60), holding("B", 40)], 100, 150);
    expect(prices?.map((p) => p.price)).toEqual([9, 6]);
  });

  it("rejects totals at or below the fixed part", () => {
    expect(managedPrices([holding("ML8321", 100)], 120, 20)).toBeNull();
  });
});
