import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { QueryKeys } from "./query-keys";
import {
  fetchWebCurrencySettings,
  isWebCurrencyCompletion,
  resetWebCurrencyQueries,
} from "./currency-cache";
import type { Settings } from "./types";

describe("web currency cache transition", () => {
  it("clears old amounts before a tab accepts changed settings after missing SSE", async () => {
    const client = new QueryClient();
    client.setQueryData([QueryKeys.SETTINGS], { baseCurrency: "CAD" });
    client.setQueryData([QueryKeys.PERFORMANCE_HISTORY, "portfolio:all"], { amount: 100 });
    const next = await fetchWebCurrencySettings(
      client,
      async () => ({ baseCurrency: "USD" }) as Settings,
    );
    expect(next.baseCurrency).toBe("USD");
    expect(client.getQueryData([QueryKeys.PERFORMANCE_HISTORY, "portfolio:all"])).toBeUndefined();
    client.clear();
  });

  it("recognizes only an explicit currency completion event", () => {
    expect(isWebCurrencyCompletion({ baseCurrencyChanged: true })).toBe(true);
    for (const payload of [
      null,
      {},
      { baseCurrencyChanged: false },
      { baseCurrencyChanged: "true" },
      { currencyChanged: true },
    ]) {
      expect(isWebCurrencyCompletion(payload)).toBe(false);
    }
  });

  it("cancels and resets every financial cache family", async () => {
    const client = new QueryClient();
    const keys = [
      [QueryKeys.HOLDINGS, "portfolio:all"],
      [QueryKeys.NET_WORTH_HISTORY, "2025-01-01", "2025-01-02"],
      [QueryKeys.SPENDING_TRANSACTIONS],
      [QueryKeys.SNAPSHOT_HOLDINGS, "account:one", "2025-01-06"],
    ];
    keys.forEach((key) => client.setQueryData(key, { amount: 100, currency: "CAD" }));
    client.setQueryData([QueryKeys.SETTINGS], { baseCurrency: "CAD" });
    await resetWebCurrencyQueries(client);
    keys.forEach((key) => expect(client.getQueryData(key)).toBeUndefined());
    client.clear();
  });
});
