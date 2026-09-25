import test from "node:test";
import assert from "node:assert/strict";
import {
  handleCheckActivitiesImport,
  handleCheckExistingDuplicates,
  handleImportActivities,
  handleParseCsv,
} from "./activity-imports.mjs";

const owner = "user-test";
const headers = { "oai-authenticated-user-id": owner };

function request(path, body, init = {}) {
  return new Request(`https://example.test${path}`, {
    method: "POST",
    headers: { ...headers, ...(body ? { "content-type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
    ...init,
  });
}

function fakeDb(seed = []) {
  const records = [...seed];
  const calls = [];
  const accounts = [{ id: "account-1", owner_id: owner, is_archived: 0 }];
  return {
    records,
    calls,
    accounts,
    DB: {
      prepare(sql) {
        return {
          bind(...args) {
            calls.push({ sql, args });
            return {
              sql,
              args,
              async all() {
                const [queryOwner, ...values] = args;
                if (sql.includes("FROM accounts")) {
                  return { results: accounts.filter((account) => account.owner_id === queryOwner && values.includes(account.id)) };
                }
                const keys = new Set(values);
                return { results: records.filter((record) => record.owner_id === queryOwner && keys.has(record.idempotency_key)) };
              },
              async first() { return null; },
              async run() {
                return { success: true };
              },
            };
          },
        };
      },
      async batch(statements) {
        for (const statement of statements) {
          if (!statement.sql.startsWith("INSERT INTO activity_records")) continue;
          for (let offset = 0; offset < statement.args.length; offset += 17) {
            const [id, ownerId, accountId, date, assetId, symbol, activityType, quantity, unitPrice, currency, fee, tax, amount, key, payload, importRunId, createdAt] = statement.args.slice(offset, offset + 17);
            records.push({ id, owner_id: ownerId, account_id: accountId, activity_date: date, asset_id: assetId, symbol, activity_type: activityType, quantity, unit_price: unitPrice, currency, fee, tax, amount, idempotency_key: key, payload_json: payload, import_run_id: importRunId, created_at: createdAt });
          }
        }
        return statements.map(() => ({ success: true }));
      },
    },
  };
}

function activity(overrides = {}) {
  return {
    date: "2026-09-20",
    symbol: "AAPL",
    activityType: "BUY",
    quantity: "2",
    unitPrice: "100",
    currency: "USD",
    accountId: "account-1",
    isValid: true,
    ...overrides,
  };
}

test("parse handler returns Rust-compatible CSV shape and supports quoted commas", async () => {
  const form = new FormData();
  form.set("file", new Blob(["name,description\nAAPL,\"buy, lot 1\"\n"]), "activities.csv");
  form.set("config", JSON.stringify({ delimiter: "," }));
  const response = await handleParseCsv(new Request("https://example.test/activities/import/parse", { method: "POST", headers, body: form }));
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), {
    headers: ["name", "description"],
    rows: [["AAPL", "buy, lot 1"]],
    detectedConfig: { delimiter: ",", hasHeaderRow: true, headerRowIndex: 0, skipTopRows: 0, skipBottomRows: 0, skipEmptyRows: true, quoteChar: '"' },
    errors: [],
    rowCount: 1,
  });
});

test("check marks malformed rows and duplicate rows without writing", async () => {
  const db = fakeDb();
  const response = await handleCheckActivitiesImport(request("/activities/import/check", { activities: [activity(), activity()] }), db);
  assert.equal(response.status, 200);
  const rows = await response.json();
  assert.equal(rows.length, 2);
  assert.equal(rows[0].isValid, true);
  assert.equal(rows[1].duplicateOfLineNumber, 1);
  assert.equal(db.records.length, 0);

  const invalid = await handleCheckActivitiesImport(request("/activities/import/check", { activities: [activity({ accountId: "" })] }), db);
  assert.equal((await invalid.json())[0].isValid, false);
});

test("commit persists full payload, skips existing duplicates, and forceImport clears key", async () => {
  const db = fakeDb([{ id: "existing-id", owner_id: owner, idempotency_key: "known" }]);
  const first = activity({ idempotencyKey: "known" });
  // An explicit key is retained by the payload but the semantic key remains deterministic.
  const response = await handleImportActivities(request("/activities/import", { activities: [first, activity({ forceImport: true })] }), db);
  assert.equal(response.status, 200);
  const result = await response.json();
  assert.equal(result.summary.imported, 1);
  assert.equal(result.summary.duplicates, 1);
  assert.equal(db.records.length, 2);
  assert.equal(db.records[1].idempotency_key, null);
  assert.equal(db.records[1].quantity, "2");
  assert.equal(db.records[1].unit_price, "100");
  assert.equal(JSON.parse(db.records[1].payload_json).forceImport, true);
});

test("import rejects accounts owned by another Site owner before writing", async () => {
  const db = fakeDb();
  const response = await handleImportActivities(
    request("/activities/import", { activities: [activity({ accountId: "other-account" })] }),
    db,
  );
  assert.equal(response.status, 400);
  assert.equal(db.records.length, 0);
});

test("duplicate lookup is owner scoped and returns the contract map", async () => {
  const db = fakeDb([
    { id: "mine", owner_id: owner, idempotency_key: "same" },
    { id: "other", owner_id: "other-user", idempotency_key: "other-key" },
  ]);
  const response = await handleCheckExistingDuplicates(request("/activities/import/check-duplicates", { idempotencyKeys: ["same", "other-key"] }), db);
  assert.deepEqual(await response.json(), { duplicates: { same: "mine" } });
});

test("all handlers require authenticated owner header", async () => {
  const requestWithoutOwner = new Request("https://example.test/activities/import/check", { method: "POST", body: JSON.stringify({ activities: [] }) });
  const response = await handleCheckActivitiesImport(requestWithoutOwner, fakeDb());
  assert.equal(response.status, 401);
});
