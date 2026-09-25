import test from "node:test";
import assert from "node:assert/strict";
import { handlePortfolioRoute } from "./portfolio.mjs";

const owner = "owner-one";

function request(path, body, init = {}) {
  return new Request(`https://example.test${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    ...init,
  });
}

function fakeDb(activityRecords, assetQuotes, accounts = [], settings = { baseCurrency: "USD" }) {
  const calls = [];
  return {
    calls,
    DB: {
      prepare(sql) {
        return {
          bind(ownerId) {
            calls.push({ sql, ownerId });
            const isActivities = sql.includes("FROM activity_records");
            const isQuotes = sql.includes("FROM asset_quotes");
            const isAccounts = sql.includes("FROM accounts");
            return {
              async all() {
                const source = isActivities ? activityRecords : isQuotes ? assetQuotes : isAccounts ? accounts : [];
                // Return all fixture rows so portfolio.mjs's defensive owner
                // check is exercised in addition to the SQL owner predicate.
                return { results: source };
              },
              async first() {
                return sql.includes("FROM user_settings") ? { settings_json: JSON.stringify(settings) } : null;
              },
            };
          },
        };
      },
    },
  };
}

function activity(overrides = {}) {
  return {
    owner_id: owner,
    account_id: "account-1",
    activity_date: "2026-09-01",
    asset_id: "asset-aapl",
    symbol: "AAPL",
    activity_type: "BUY",
    quantity: "10",
    unit_price: "100.00",
    currency: "USD",
    fee: "0",
    tax: "0",
    amount: "1000",
    ...overrides,
  };
}

test("holdings query calculates buy/sell quantity, basis and latest quote value", async () => {
  const db = fakeDb(
    [
      activity(),
      activity({ activity_date: "2026-09-10", activity_type: "SELL", quantity: "4", unit_price: "120", amount: "480" }),
    ],
    [
      { owner_id: owner, asset_id: "asset-aapl", symbol: "AAPL", quote_date: "2026-09-09", price: "125", currency: "USD" },
      { owner_id: owner, asset_id: "asset-aapl", symbol: "AAPL", quote_date: "2026-09-11", price: "130", currency: "USD" },
    ],
  );
  const response = await handlePortfolioRoute(request("/holdings/query", { filter: { type: "account", accountId: "account-1" } }),
    "/holdings/query", owner, db);
  assert.equal(response.status, 200);
  const [holding] = await response.json();
  assert.equal(holding.quantity, 6);
  assert.equal(holding.costBasis.base, 600);
  assert.equal(holding.price, 130);
  assert.equal(holding.marketValue.base, 780);
  assert.equal(holding.unrealizedGain.base, 180);
  assert.equal(holding.unrealizedGainPct, 0.3);
  assert.equal(db.calls.length, 2);
  assert.ok(db.calls.every((call) => call.sql.includes("owner_id = ?") && call.ownerId === owner));
});

test("portfolio summary aggregates accounts and remains owner scoped", async () => {
  const db = fakeDb(
    [
      activity(),
      activity({ account_id: "account-2", quantity: "2", amount: "200" }),
      activity({ owner_id: "other-owner", account_id: "other", quantity: "100" }),
    ],
    [{ owner_id: owner, asset_id: "asset-aapl", symbol: "AAPL", quote_date: "2026-09-12", price: "110", currency: "USD" }],
  );
  const response = await handlePortfolioRoute(request("/sites/portfolio/summary"), "/sites/portfolio/summary", owner, db);
  assert.equal(response.status, 200);
  const summary = await response.json();
  assert.equal(summary.positions.length, 1);
  assert.equal(summary.positions[0].quantity, 12);
  assert.equal(summary.totalValue, 1320);
  assert.equal(summary.costBasis, 1200);
  assert.equal(summary.unrealizedGain, 120);
  assert.equal(summary.summary.accountCount, 2);
});

test("performance summary follows the existing PerformanceResult wire shape", async () => {
  const db = fakeDb([activity()], [{ owner_id: owner, asset_id: "asset-aapl", quote_date: "2026-09-12", price: "125", currency: "USD" }]);
  const response = await handlePortfolioRoute(
    request("/performance/summary", { itemType: "account", itemId: "account-1", startDate: "2026-09-01", endDate: "2026-09-12" }),
    "/performance/summary",
    owner,
    db,
  );
  assert.equal(response.status, 200);
  const result = await response.json();
  assert.equal(result.scope.id, "account-1");
  assert.equal(result.summary.amount, 250);
  assert.equal(result.summary.percent, 0.25);
  assert.equal(result.returns.valueReturn, 0.25);
  assert.equal(result.period.endDate, "2026-09-12");
});

test("unsupported paths are left for the dispatcher", async () => {
  assert.equal(await handlePortfolioRoute(request("/portfolio"), "/portfolio", owner, fakeDb([], [])), null);
  assert.equal(await handlePortfolioRoute(request("/sites/mcp/portfolio"), "/sites/mcp/portfolio", owner, fakeDb([], [])), null);
});

test("holdings list query is an exact supported alias", async () => {
  const db = fakeDb([activity()], [{ owner_id: owner, asset_id: "asset-aapl", quote_date: "2026-09-12", price: "100", currency: "USD" }]);
  const response = await handlePortfolioRoute(request("/holdings/list/query", { filter: { type: "all" } }), "/holdings/list/query", owner, db);
  assert.equal(response.status, 200);
  assert.equal((await response.json())[0].instrument.symbol, "AAPL");
});

test("asset holdings are returned from owner-scoped activity calculations", async () => {
  const db = fakeDb(
    [activity(), activity({ owner_id: "other-owner", account_id: "other-account", asset_id: "asset-secret", symbol: "SECRET" })],
    [{ owner_id: owner, asset_id: "asset-aapl", symbol: "AAPL", quote_date: "2026-09-12", price: "110", currency: "USD" }],
  );
  const response = await handlePortfolioRoute(
    request("/holdings/by-asset?assetId=asset-aapl"),
    "/holdings/by-asset",
    owner,
    db,
  );
  assert.equal(response.status, 200);
  const [holding] = await response.json();
  assert.equal(holding.instrument.id, "asset-aapl");
  assert.equal(holding.quantity, 10);
  assert.equal(holding.costBasis.base, 1000);
  assert.equal(holding.marketValue.base, 1100);
});

test("batch performance summaries use the existing account-scope keys", async () => {
  const db = fakeDb(
    [activity()],
    [{ owner_id: owner, asset_id: "asset-aapl", quote_date: "2026-09-12", price: "125", currency: "USD" }],
  );
  const response = await handlePortfolioRoute(
    request("/performance/summaries", {
      scopes: [{ accountIds: ["account-1"] }],
      startDate: "2026-09-01",
      endDate: "2026-09-12",
      profile: "dashboard",
    }),
    "/performance/summaries",
    owner,
    db,
  );
  assert.equal(response.status, 200);
  const summaries = await response.json();
  assert.equal(summaries["accounts:account-1"].scope.id, "accounts:account-1");
  assert.equal(summaries["accounts:account-1"].summary.amount, 250);
  assert.equal(summaries["accounts:account-1"].dataQuality.status, "ok");
});

test("performance summaries do not report a zero return when the quote is missing", async () => {
  const db = fakeDb([activity()], []);
  const response = await handlePortfolioRoute(
    request("/performance/summaries", { scopes: [{ accountIds: ["account-1"] }] }),
    "/performance/summaries",
    owner,
    db,
  );
  const result = (await response.json())["accounts:account-1"];
  assert.equal(result.dataQuality.status, "partial");
  assert.equal(result.summary.amount, null);
  assert.equal(result.summary.amountStatus, "unavailable");
  assert.equal(result.returns.valueReturn, null);
  assert.deepEqual(result.series, []);
});

test("current valuation returns owner-scoped D1 totals and account values", async () => {
  const db = fakeDb(
    [activity()],
    [{ owner_id: owner, asset_id: "asset-aapl", symbol: "AAPL", quote_date: "2026-09-12", price: "130", currency: "USD" }],
    [{ owner_id: owner, id: "account-1", currency: "USD" }],
    { baseCurrency: "USD" },
  );
  const response = await handlePortfolioRoute(
    request("/valuations/current/query", { filter: { type: "all" }, includeAccounts: true }),
    "/valuations/current/query",
    owner,
    db,
  );
  assert.equal(response.status, 200);
  const result = await response.json();
  assert.equal(result.summary.totalValueBase, 1300);
  assert.equal(result.summary.investmentMarketValueBase, 1300);
  assert.equal(result.summary.accountCount, 1);
  assert.equal(result.accounts[0].accountId, "account-1");
  assert.equal(result.accounts[0].totalValueBase, 1300);
  assert.deepEqual(result.summary.warnings, []);
});

test("portfolio refresh acknowledges on-demand D1 calculation without a cache write", async () => {
  const response = await handlePortfolioRoute(request("/portfolio/update", {}), "/portfolio/update", owner, {});
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { success: true, source: "d1-read-through" });
});
