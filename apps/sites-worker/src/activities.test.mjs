import test from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";

import { handleActivitySearchRoute } from "./activities.mjs";
import { handleAssetRoute } from "./assets.mjs";
import { handleActivityImportMappingRoute } from "./activity-import-mappings.mjs";

const owner = "owner-a";
const headers = { "oai-authenticated-user-id": owner, "content-type": "application/json" };

function request(route, body, method = "POST") {
  return new Request(`https://example.test${route}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

function makeDb() {
  const database = new DatabaseSync(":memory:");
  database.exec(`
    CREATE TABLE accounts (id TEXT, owner_id TEXT, name TEXT, currency TEXT, is_archived INTEGER);
    CREATE TABLE activity_records (
      id TEXT, owner_id TEXT, account_id TEXT, activity_date TEXT, asset_id TEXT, symbol TEXT,
      activity_type TEXT, quantity TEXT, unit_price TEXT, currency TEXT, fee TEXT, tax TEXT,
      amount TEXT, idempotency_key TEXT, payload_json TEXT, import_run_id TEXT, created_at TEXT
    );
    CREATE TABLE user_settings (owner_id TEXT PRIMARY KEY, settings_json TEXT, updated_at TEXT);
    INSERT INTO accounts VALUES ('account-a', 'owner-a', 'Synthetic account', 'USD', 0);
    INSERT INTO accounts VALUES ('account-b', 'owner-b', 'Other owner account', 'USD', 0);
    INSERT INTO activity_records VALUES (
      'activity-old', 'owner-a', 'account-a', '2026-09-01', 'asset-synth', 'SYNTH',
      'BUY', '2', '100', 'USD', '0', '0', '200', 'key-old',
      '{"needsReview":false,"asset":{"instrumentType":"EQUITY"}}', 'run-1', '2026-09-01T00:00:00.000Z'
    );
    INSERT INTO activity_records VALUES (
      'activity-new', 'owner-a', 'account-a', '2026-09-03', 'asset-synth', 'SYNTH',
      'SELL', '1', '110', 'USD', '0', '0', '110', 'key-new',
      '{"needsReview":true,"asset":{"instrumentType":"EQUITY"}}', 'run-1', '2026-09-03T00:00:00.000Z'
    );
    INSERT INTO activity_records VALUES (
      'private-row', 'owner-b', 'account-b', '2026-09-04', 'asset-secret', 'SECRET',
      'BUY', '1', '999', 'USD', '0', '0', '999', 'key-other', '{}', 'run-2', '2026-09-04T00:00:00.000Z'
    );
  `);
  return {
    close: () => database.close(),
    DB: {
      prepare(sql) {
        return {
          bind(...values) {
            return {
              async all() { return { results: database.prepare(sql).all(...values) }; },
              async first() { return database.prepare(sql).get(...values) ?? null; },
              async run() { return { success: true, ...database.prepare(sql).run(...values) }; },
            };
          },
        };
      },
    },
  };
}

test("activity search is owner-scoped, paginated, and maps stored import payloads", async () => {
  const env = makeDb();
  try {
    const response = await handleActivitySearchRoute(
      request("/activities/search", { page: 0, pageSize: 1, sort: { id: "date", desc: true } }),
      "/activities/search",
      owner,
      env,
    );
    assert.equal(response.status, 200);
    const result = await response.json();
    assert.equal(result.meta.totalRowCount, 2);
    assert.equal(result.data.length, 1);
    assert.equal(result.data[0].id, "activity-new");
    assert.equal(result.data[0].accountName, "Synthetic account");
    assert.equal(result.data[0].assetSymbol, "SYNTH");
    assert.equal(result.data[0].needsReview, true);

    const filtered = await handleActivitySearchRoute(
      request("/activities/search", { accountIdFilter: ["account-b"] }),
      "/activities/search",
      owner,
      env,
    );
    assert.equal((await filtered.json()).meta.totalRowCount, 0);
  } finally {
    env.close();
  }
});

test("asset preview resolves prior symbols and prepares new assets without remote lookups", async () => {
  const env = makeDb();
  try {
    const response = await handleAssetRoute(
      request("/activities/import/assets/preview", {
        candidates: [
          { key: "SYNTH::EQUITY", accountId: "account-a", symbol: "SYNTH", currency: "USD" },
          { key: "NEW::EQUITY", accountId: "account-a", symbol: "NEW", currency: "USD" },
          { key: "SECRET", accountId: "account-b", symbol: "SECRET", currency: "USD" },
        ],
      }),
      "/activities/import/assets/preview",
      owner,
      env,
    );
    assert.equal(response.status, 200);
    const [existing, created, rejected] = await response.json();
    assert.equal(existing.status, "EXISTING_ASSET");
    assert.equal(existing.assetId, "asset-synth");
    assert.equal(created.status, "AUTO_RESOLVED_NEW_ASSET");
    assert.equal(created.draft.instrumentSymbol, "NEW");
    assert.equal(rejected.status, "NEEDS_FIXING");

    const createA = await handleAssetRoute(
      request("/assets", { instrumentSymbol: "NEW", quoteCcy: "USD", quoteMode: "MARKET" }),
      "/assets",
      owner,
      env,
    );
    const createB = await handleAssetRoute(
      request("/assets", { instrumentSymbol: "NEW", quoteCcy: "USD", quoteMode: "MARKET" }),
      "/assets",
      owner,
      env,
    );
    assert.equal((await createA.json()).id, (await createB.json()).id);
  } finally {
    env.close();
  }
});

test("import mapping reads and writes only a mapping for an owned account", async () => {
  const env = makeDb();
  try {
    const saved = await handleActivityImportMappingRoute(
      request("/activities/import/mapping", {
        mapping: { accountId: "account-a", importType: "CSV_ACTIVITY", fieldMappings: { DATE: "Trade Date" } },
      }),
      "/activities/import/mapping",
      owner,
      env,
    );
    assert.equal((await saved.json()).fieldMappings.DATE, "Trade Date");
    const read = await handleActivityImportMappingRoute(
      request("/activities/import/mapping?accountId=account-a&contextKind=CSV_ACTIVITY", undefined, "GET"),
      "/activities/import/mapping",
      owner,
      env,
    );
    assert.equal((await read.json()).fieldMappings.DATE, "Trade Date");
    const denied = await handleActivityImportMappingRoute(
      request("/activities/import/mapping", {
        mapping: { accountId: "account-b", importType: "CSV_ACTIVITY" },
      }),
      "/activities/import/mapping",
      owner,
      env,
    );
    assert.equal(denied.status, 404);
  } finally {
    env.close();
  }
});
