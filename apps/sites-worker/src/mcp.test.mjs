import test from "node:test";
import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";

import { handleMcpRoute } from "./mcp.mjs";

const workerRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(workerRoot, "../..");
const owner = "owner-test";
const otherOwner = "other-owner";

function makeD1() {
  const database = new DatabaseSync(":memory:");
  const d1 = {
    prepare(sql) {
      return {
        sql,
        params: [],
        bind(...params) {
          return { sql, params };
        },
        all() {
          throw new Error("Call bind before all.");
        },
        first() {
          throw new Error("Call bind before first.");
        },
        run() {
          throw new Error("Call bind before run.");
        },
      };
    },
    async batch(statements) {
      const originalExec = database.prepare.bind(database);
      try {
        database.exec("BEGIN IMMEDIATE");
        for (const statement of statements) originalExec(statement.sql).run(...statement.params);
        database.exec("COMMIT");
        return statements.map(() => ({ success: true }));
      } catch (error) {
        database.exec("ROLLBACK");
        throw error;
      }
    },
    close() { database.close(); },
  };
  const prepare = d1.prepare.bind(d1);
  d1.prepare = (sql) => {
    const create = prepare(sql);
    return {
      bind(...params) {
        const bound = create.bind(...params);
        return {
          async all() { return { results: database.prepare(sql).all(...params) }; },
          async first() { return database.prepare(sql).get(...params) ?? null; },
          async run() { return { success: true, ...database.prepare(sql).run(...params) }; },
          sql,
          params,
        };
      },
    };
  };
  return { d1, database };
}

async function migratedDb() {
  const { d1, database } = makeD1();
  const migrationDir = path.join(repositoryRoot, "drizzle");
  const files = (await readdir(migrationDir)).filter((name) => name.endsWith(".sql")).sort();
  for (const file of files) {
    const migration = await readFile(path.join(migrationDir, file), "utf8");
    database.exec(migration.replace(/^-->.*$/gm, ""));
  }
  return { d1, database };
}

function request(route, body, asOwner = owner) {
  return new Request(`https://wealthfolio.test/api/v1${route}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "oai-authenticated-user-id": asOwner,
    },
    body: JSON.stringify(body),
  });
}

async function seedActivity(database, { id = "activity-1", activityOwner = owner, accountId = "account-1" } = {}) {
  database.prepare(
    "INSERT INTO activity_records (id, owner_id, account_id, activity_date, activity_type, amount, currency, payload_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)",
  ).run(id, activityOwner, accountId, "2026-09-20", "WITHDRAWAL", "-42.50", "CAD", JSON.stringify({ comment: "Coffee shop" }));
}

async function seedAccount(database, id = "account-1", accountOwner = owner) {
  database.prepare(
    "INSERT INTO accounts (id, owner_id, name, account_type, currency) VALUES (?, ?, ?, ?, ?)",
  ).run(id, accountOwner, "Everyday", "CASH", "CAD");
}

test("context and proposals read only owner-scoped transactions and taxonomy categories", async () => {
  const { d1, database } = await migratedDb();
  try {
    await seedActivity(database);
    await seedActivity(database, { id: "foreign-activity", activityOwner: otherOwner });
    const contextResponse = await handleMcpRoute(request("/sites/mcp/list-categorization-context", {}), "/sites/mcp/list-categorization-context", owner, { DB: d1 });
    assert.equal(contextResponse.status, 200);
    const context = await contextResponse.json();
    assert.equal(context.unproposed.length, 1);
    assert.equal(context.unproposed[0].activityId, "activity-1");
    const spending = context.taxonomies.find((taxonomy) => taxonomy.taxonomyId === "spending_categories");
    assert.ok(spending.categories.some((category) => category.key === "food_coffee"));

    const proposedResponse = await handleMcpRoute(
      request("/sites/mcp/propose-transaction-categories", {
        aiProposals: [{ activityId: "activity-1", taxonomyId: "spending_categories", categoryKey: "food_coffee", confidence: 0.9, reason: "Coffee merchant" }],
      }),
      "/sites/mcp/propose-transaction-categories",
      owner,
      { DB: d1 },
    );
    const proposed = await proposedResponse.json();
    assert.equal(proposed.proposals.length, 1);
    assert.equal(proposed.proposals[0].source, "ai");
    assert.equal(proposed.draft_status, "draft");
    assert.equal(database.prepare("SELECT count(*) AS count FROM activity_category_assignments").get().count, 0);
  } finally {
    d1.close();
  }
});

test("category commit is owner-scoped, atomic, and logs only the batch count", async () => {
  const { d1, database } = await migratedDb();
  try {
    await seedActivity(database);
    await seedActivity(database, { id: "foreign-activity", activityOwner: otherOwner });
    const response = await handleMcpRoute(
      request("/sites/mcp/commit-category-assignments", {
        assignments: [
          { activityId: "activity-1", taxonomyId: "spending_categories", categoryKey: "food_coffee" },
          { activityId: "foreign-activity", taxonomyId: "spending_categories", categoryKey: "groceries" },
        ],
      }),
      "/sites/mcp/commit-category-assignments",
      owner,
      { DB: d1 },
    );
    assert.equal(response.status, 400);
    assert.equal(database.prepare("SELECT count(*) AS count FROM activity_category_assignments").get().count, 0);

    const success = await handleMcpRoute(
      request("/sites/mcp/commit-category-assignments", {
        assignments: [{ activityId: "activity-1", taxonomyId: "spending_categories", categoryKey: "food_coffee" }],
      }),
      "/sites/mcp/commit-category-assignments",
      owner,
      { DB: d1 },
    );
    assert.equal(success.status, 200);
    assert.equal((await success.json()).applied[0].activityId, "activity-1");
    assert.equal(database.prepare("SELECT count(*) AS count FROM activity_category_assignments WHERE owner_id = ?").get(owner).count, 1);
    const audit = database.prepare("SELECT action, result_count FROM mcp_audit_logs WHERE owner_id = ?").get(owner);
    assert.deepEqual({ ...audit }, { action: "commit_category_assignments", result_count: 1 });
    assert.equal(database.prepare("SELECT count(*) AS count FROM mcp_audit_logs WHERE owner_id = ? AND action LIKE '%activity-1%'").get(owner).count, 0);
  } finally {
    d1.close();
  }
});

test("MCP rule and account writes validate owner scope and are audited without user details", async () => {
  const { d1, database } = await migratedDb();
  try {
    await seedAccount(database);
    const ruleResponse = await handleMcpRoute(
      request("/sites/mcp/commit-categorization-rule", {
        pattern: "Cafe North",
        matchType: "contains",
        taxonomyId: "spending_categories",
        categoryKey: "food_coffee",
        accountId: "other-account",
      }),
      "/sites/mcp/commit-categorization-rule",
      owner,
      { DB: d1 },
    );
    assert.equal(ruleResponse.status, 400);
    assert.equal(database.prepare("SELECT count(*) AS count FROM categorization_rules").get().count, 0);

    const accountResponse = await handleMcpRoute(
      request("/sites/mcp/create-account", { name: "Savings", accountType: "CASH", currency: "CAD" }),
      "/sites/mcp/create-account",
      owner,
      { DB: d1 },
    );
    assert.equal(accountResponse.status, 201);
    assert.equal(database.prepare("SELECT count(*) AS count FROM accounts WHERE owner_id = ?").get(owner).count, 2);
    const audit = database.prepare("SELECT action, result_count FROM mcp_audit_logs WHERE owner_id = ? ORDER BY rowid DESC LIMIT 1").get(owner);
    assert.deepEqual({ ...audit }, { action: "create_account", result_count: 1 });
  } finally {
    d1.close();
  }
});
