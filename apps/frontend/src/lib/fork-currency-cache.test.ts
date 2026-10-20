import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { isWebCurrencyCompletion, resetWebCurrencyQueries } from "./fork-currency-cache";
import { QueryKeys } from "./query-keys";

describe("web currency cache transition", () => {
  it("recognizes only an explicit currency completion event", () => {
    expect(isWebCurrencyCompletion({ baseCurrencyChanged: true })).toBe(true);
    for (const payload of [
      null,
      undefined,
      {},
      { background: true },
      { baseCurrencyChanged: "true" },
    ]) {
      expect(isWebCurrencyCompletion(payload)).toBe(false);
    }
  });
  it("drops every scope and intraday range, plus income, spending and planning amounts", async () => {
    const client = new QueryClient();
    const keys = [
      [QueryKeys.HISTORY_VALUATION, "intraday", "portfolio:all", "1D", "CAD"],
      [QueryKeys.HISTORY_VALUATION, "intraday", "portfolio:retirement", "1W", "CAD"],
      [QueryKeys.CURRENT_VALUATION, "account:one"],
      [QueryKeys.PERFORMANCE_HISTORY, "portfolio:retirement"],
      [QueryKeys.INCOME_SUMMARY],
      [QueryKeys.HOLDINGS_BY_ALLOCATION],
      [QueryKeys.SPENDING_TRANSACTIONS],
      [QueryKeys.RETIREMENT_OVERVIEW],
    ];
    keys.forEach((key) => client.setQueryData(key, { amount: 100, currency: "CAD" }));
    client.setQueryData([QueryKeys.SETTINGS], { baseCurrency: "CAD" });
    await resetWebCurrencyQueries(client);
    keys.forEach((key) => expect(client.getQueryData(key)).toBeUndefined());
    expect(client.getQueryData([QueryKeys.SETTINGS])).toEqual({ baseCurrency: "CAD" });
    client.clear();
  });

  it("does not let a late old-currency response repopulate the reset cache", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    let resolve!: (value: { amount: number; currency: string }) => void;
    const key = [QueryKeys.HOLDINGS, "portfolio:all"];
    const pending = client
      .fetchQuery({
        queryKey: key,
        queryFn: () =>
          new Promise<{ amount: number; currency: string }>((done) => {
            resolve = done;
          }),
      })
      .catch(() => undefined);
    await resetWebCurrencyQueries(client);
    resolve({ amount: 100, currency: "CAD" });
    await pending;
    expect(client.getQueryData(key)).toBeUndefined();
    client.clear();
  });
});
