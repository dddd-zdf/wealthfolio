import { parseCsv } from "./activity-imports.mjs";

const FILE_KEYS = ["accounts", "activities", "holdings", "portfolioHistory"];
const MAX_FILE_BYTES = 10 * 1024 * 1024;
const WRITE_CHUNK_SIZE = 50;

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function errorResponse(status, message) {
  return json({ message }, status);
}

function asText(value) {
  return value == null ? "" : String(value).trim();
}

function parseBoolean(value, fallback = false) {
  const text = asText(value).toLowerCase();
  if (["true", "1", "yes"].includes(text)) return true;
  if (["false", "0", "no"].includes(text)) return false;
  return fallback;
}

function parseJsonCell(value, label, fallback) {
  const text = asText(value);
  if (!text) return fallback;
  try {
    return JSON.parse(text);
  } catch {
    throw new TypeError(`The ${label} column contains invalid JSON.`);
  }
}

function dateOnly(value, label) {
  const match = asText(value).match(/^(\d{4}-\d{2}-\d{2})/);
  if (!match) throw new TypeError(`A valid ${label} date is required.`);
  const date = new Date(`${match[1]}T00:00:00Z`);
  if (!Number.isFinite(date.getTime()) || date.toISOString().slice(0, 10) !== match[1]) {
    throw new TypeError(`A valid ${label} date is required.`);
  }
  return match[1];
}

function hashHex(bytes) {
  return crypto.subtle.digest("SHA-256", bytes).then((digest) =>
    [...new Uint8Array(digest)].map((value) => value.toString(16).padStart(2, "0")).join(""),
  );
}

async function readExportFile(form, key) {
  const file = form.get(key);
  if (!file || typeof file.arrayBuffer !== "function") throw new TypeError(`Select the ${key} export file.`);
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (!bytes.byteLength || bytes.byteLength > MAX_FILE_BYTES) throw new TypeError(`The ${key} export must be between 1 byte and 10 MB.`);
  const source = new TextDecoder("utf-8", { fatal: true }).decode(bytes).replace(/^\uFEFF/, "");
  const parsed = parseCsv(source, { delimiter: ",", hasHeaderRow: true, quoteChar: "\"", escapeChar: "\\" });
  if (parsed.errors.length) throw new TypeError(`The ${key} export has invalid CSV quoting or column structure.`);
  const rows = parsed.rows.map((cells) => Object.fromEntries(parsed.headers.map((header, index) => [header, cells[index] ?? ""])));
  if (!rows.length) throw new TypeError(`The ${key} export has no data rows.`);
  return { bytes, rows };
}

function uniqueIds(rows, label) {
  const ids = rows.map((row) => asText(row.id));
  if (ids.some((id) => !id || id.length > 160)) throw new TypeError(`The ${label} export has a missing or invalid row ID.`);
  if (new Set(ids).size !== ids.length) throw new TypeError(`The ${label} export contains duplicate row IDs.`);
  return ids;
}

function sanitizeAccount(row) {
  const accountType = asText(row.accountType).toUpperCase();
  const trackingMode = asText(row.trackingMode).toUpperCase() || "NOT_SET";
  const currency = asText(row.currency).toUpperCase();
  if (!/^[\w-]{1,80}$/.test(asText(row.id))) throw new TypeError("An account has an invalid ID.");
  if (!asText(row.name) || !["SECURITIES", "CASH", "CREDIT_CARD", "CRYPTOCURRENCY"].includes(accountType)) {
    throw new TypeError("An account has an unsupported name or account type.");
  }
  if (!/^[A-Z]{3}$/.test(currency) || !["TRANSACTIONS", "HOLDINGS", "NOT_SET"].includes(trackingMode)) {
    throw new TypeError("An account has an unsupported currency or tracking mode.");
  }
  const meta = parseJsonCell(row.meta, "account metadata", {});
  if (!meta || typeof meta !== "object" || Array.isArray(meta)) throw new TypeError("Account metadata must be a JSON object.");
  return {
    id: asText(row.id),
    name: asText(row.name),
    accountType,
    currency,
    group: asText(row.group) || null,
    trackingMode,
    meta,
    isDefault: parseBoolean(row.isDefault),
    isActive: parseBoolean(row.isActive, true),
    isArchived: parseBoolean(row.isArchived),
    createdAt: asText(row.createdAt) || new Date().toISOString(),
    updatedAt: asText(row.updatedAt) || new Date().toISOString(),
  };
}

function sanitizeActivity(row, accountIds) {
  const id = asText(row.id);
  const accountId = asText(row.accountId);
  const activityDate = dateOnly(row.date, "activity");
  const activityType = asText(row.activityType).toUpperCase();
  if (!/^[\w-]{1,160}$/.test(id) || !accountIds.has(accountId)) {
    throw new TypeError("An activity has an invalid ID or points to an account outside the export.");
  }
  if (!activityType) throw new TypeError("An activity is missing its type.");
  const symbol = asText(row.assetSymbol || row.symbol).toUpperCase() || null;
  const payload = { ...row, date: activityDate, symbol };
  payload.metadata = parseJsonCell(row.metadata, "activity metadata", null);
  for (const key of ["needsReview", "isUserModified"]) {
    if (key in payload) payload[key] = parseBoolean(payload[key]);
  }
  return {
    id,
    accountId,
    activityDate,
    assetId: symbol,
    symbol,
    activityType,
    quantity: asText(row.quantity) || null,
    unitPrice: asText(row.unitPrice) || null,
    currency: asText(row.currency || row.accountCurrency).toUpperCase() || null,
    fee: asText(row.fee) || null,
    tax: asText(row.tax) || null,
    amount: asText(row.amount) || null,
    idempotencyKey: asText(row.idempotencyKey) || null,
    payload,
  };
}

function scrubAccountExport(row) {
  const clean = { ...row };
  delete clean.accountNumber;
  delete clean.providerAccountId;
  delete clean.platformId;
  delete clean.provider;
  return clean;
}

function buildSummary(data) {
  const activityDates = data.activities.map((row) => row.activityDate).sort();
  const historyDates = data.history.map((row) => row.valuationDate).sort();
  const activityTypes = {};
  for (const row of data.activities) activityTypes[row.activityType] = (activityTypes[row.activityType] ?? 0) + 1;
  const accountTypes = {};
  for (const row of data.accounts) accountTypes[row.accountType] = (accountTypes[row.accountType] ?? 0) + 1;
  return {
    counts: {
      accounts: data.accounts.length,
      activities: data.activities.length,
      holdings: data.holdings.length,
      portfolioHistory: data.history.length,
    },
    accountTypes,
    activityTypes,
    activityDateRange: [activityDates[0], activityDates.at(-1)],
    portfolioHistoryDateRange: [historyDates[0], historyDates.at(-1)],
    portfolioHistoryScope: "all-accounts",
    activityRowsMissingAmount: data.activities.filter((row) => row.amount == null).length,
    holdingsAccountReferences: data.holdings.reduce((sum, row) => sum + row.sourceAccountIds.length, 0),
    importedDataExclusions: ["broker/provider connection identifiers"],
  };
}

async function parseExportBundle(form) {
  const fileData = {};
  for (const key of FILE_KEYS) fileData[key] = await readExportFile(form, key);
  const bytes = new Uint8Array(fileData.accounts.bytes.length + fileData.activities.bytes.length + fileData.holdings.bytes.length + fileData.portfolioHistory.bytes.length);
  let offset = 0;
  for (const key of FILE_KEYS) {
    bytes.set(fileData[key].bytes, offset);
    offset += fileData[key].bytes.length;
  }
  const sourceHash = await hashHex(bytes);
  const accountIds = new Set(uniqueIds(fileData.accounts.rows, "account"));
  const activitiesIds = uniqueIds(fileData.activities.rows, "activity");
  const holdingIds = uniqueIds(fileData.holdings.rows, "holding");
  const historyIds = uniqueIds(fileData.portfolioHistory.rows, "portfolio history");
  const accounts = fileData.accounts.rows.map(sanitizeAccount);
  const activities = fileData.activities.rows.map((row) => sanitizeActivity(row, accountIds));
  const idempotencyKeys = activities.map((row) => row.idempotencyKey).filter(Boolean);
  if (new Set(idempotencyKeys).size !== idempotencyKeys.length) throw new TypeError("The activity export has duplicate idempotency keys.");
  const holdings = fileData.holdings.rows.map((row, index) => {
    const sourceAccountIds = parseJsonCell(row.sourceAccountIds, "holding source accounts", []);
    if (!Array.isArray(sourceAccountIds) || sourceAccountIds.some((id) => !accountIds.has(asText(id)))) {
      throw new TypeError("A holding points to an account outside the account export.");
    }
    const asOfDate = dateOnly(row.asOfDate, "holding");
    const symbol = asText(row.symbol).toUpperCase();
    if (!symbol) throw new TypeError("A holding is missing its symbol.");
    return {
      id: holdingIds[index],
      symbol,
      accountId: "all",
      sourceAccountIds: sourceAccountIds.map(asText),
      asOfDate,
      price: asText(row.price) || null,
      currency: asText(row.instrumentCurrency || row.localCurrency || row.baseCurrency).toUpperCase() || null,
      payload: { ...row, symbol, accountId: "all", sourceAccountIds: sourceAccountIds.map(asText), asOfDate },
    };
  });
  const history = fileData.portfolioHistory.rows.map((row, index) => {
    const valuationDate = dateOnly(row.valuationDate, "portfolio history");
    const payload = { ...row, accountId: "all", valuationDate };
    return { id: historyIds[index], accountId: "all", valuationDate, payload };
  });
  if (new Set(history.map((row) => row.valuationDate)).size !== history.length) {
    throw new TypeError("The portfolio history export must contain one all-account row per date.");
  }
  if (accounts.filter((row) => row.isDefault).length > 1) throw new TypeError("The account export has more than one default account.");

  return {
    sourceHash,
    accounts,
    activities,
    holdings,
    history,
    rawAccounts: fileData.accounts.rows.map(scrubAccountExport),
    rawActivities: fileData.activities.rows,
    rawHoldings: fileData.holdings.rows,
    rawHistory: fileData.portfolioHistory.rows,
    summary: buildSummary({ accounts, activities, holdings, history }),
  };
}

async function existingIds(env, table, ownerId, ids) {
  const found = new Set();
  for (let offset = 0; offset < ids.length; offset += 90) {
    const chunk = ids.slice(offset, offset + 90);
    const statement = env.DB.prepare(`SELECT id FROM ${table} WHERE owner_id = ? AND id IN (${chunk.map(() => "?").join(",")})`);
    const result = await statement.bind(ownerId, ...chunk).all();
    for (const row of result.results ?? result.rows ?? []) found.add(String(row.id));
  }
  return found;
}

async function preview(payload, ownerId, env) {
  const [existingAccountIds, existingActivityIds] = await Promise.all([
    existingIds(env, "accounts", ownerId, payload.accounts.map((row) => row.id)),
    existingIds(env, "activity_records", ownerId, payload.activities.map((row) => row.id)),
  ]);
  let previousImport = null;
  try {
    previousImport = await env.DB.prepare(
      "SELECT id, status FROM wealthfolio_import_batches WHERE owner_id = ? AND source_hash = ? LIMIT 1",
    ).bind(ownerId, payload.sourceHash).first();
  } catch {
    // The migration tables do not exist until the user confirms an import.
  }
  const canResume = previousImport?.status === "in_progress";
  const conflicts = {
    accountIds: canResume ? 0 : existingAccountIds.size,
    activityIds: canResume ? 0 : existingActivityIds.size,
  };
  const alreadyImported = previousImport?.status === "complete";
  return {
    ...payload.summary,
    canImport: conflicts.accountIds === 0 && conflicts.activityIds === 0 && !alreadyImported,
    alreadyImported,
    conflicts,
    existingSiteRows: {
      accounts: Number((await env.DB.prepare("SELECT COUNT(*) AS count FROM accounts WHERE owner_id = ?").bind(ownerId).first())?.count ?? 0),
      activities: Number((await env.DB.prepare("SELECT COUNT(*) AS count FROM activity_records WHERE owner_id = ?").bind(ownerId).first())?.count ?? 0),
    },
  };
}

async function ensureMigrationTables(env) {
  await env.DB.prepare(`CREATE TABLE IF NOT EXISTS wealthfolio_import_batches (
    id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, source_hash TEXT NOT NULL, status TEXT NOT NULL,
    accounts_count INTEGER NOT NULL, activities_count INTEGER NOT NULL, holdings_count INTEGER NOT NULL,
    history_count INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(owner_id, source_hash)
  )`).run();
  await env.DB.prepare(`CREATE TABLE IF NOT EXISTS wealthfolio_import_rows (
    owner_id TEXT NOT NULL, dataset TEXT NOT NULL, row_id TEXT NOT NULL, record_date TEXT,
    import_id TEXT NOT NULL, payload_json TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(owner_id, dataset, row_id)
  )`).run();
  await env.DB.prepare("CREATE INDEX IF NOT EXISTS wealthfolio_import_rows_owner_date_idx ON wealthfolio_import_rows(owner_id, dataset, record_date)").run();
}

async function insertBatches(env, statements) {
  for (let offset = 0; offset < statements.length; offset += WRITE_CHUNK_SIZE) {
    await env.DB.batch(statements.slice(offset, offset + WRITE_CHUNK_SIZE));
  }
}

function sourceRowStatements(env, ownerId, importId, dataset, rows, getDate = () => null) {
  return rows.map((row) => env.DB.prepare(
    "INSERT OR IGNORE INTO wealthfolio_import_rows (owner_id, dataset, row_id, record_date, import_id, payload_json) VALUES (?, ?, ?, ?, ?, ?)",
  ).bind(ownerId, dataset, row.id, getDate(row), importId, JSON.stringify(row)));
}

async function commit(payload, ownerId, env) {
  await ensureMigrationTables(env);
  let batch = await env.DB.prepare(
    "SELECT id, status FROM wealthfolio_import_batches WHERE owner_id = ? AND source_hash = ? LIMIT 1",
  ).bind(ownerId, payload.sourceHash).first();
  if (batch?.status === "complete") return { alreadyImported: true, importId: batch.id, ...payload.summary };
  const previewResult = await preview(payload, ownerId, env);
  if (!previewResult.canImport && !batch) throw new TypeError("The import conflicts with existing account or activity IDs. No records were written.");
  const importId = batch?.id ?? crypto.randomUUID();
  if (!batch) {
    await env.DB.prepare(
      "INSERT INTO wealthfolio_import_batches (id, owner_id, source_hash, status, accounts_count, activities_count, holdings_count, history_count) VALUES (?, ?, ?, 'in_progress', ?, ?, ?, ?)",
    ).bind(importId, ownerId, payload.sourceHash, payload.accounts.length, payload.activities.length, payload.holdings.length, payload.history.length).run();
  }

  try {
    const accountStatements = payload.accounts.map((account) => env.DB.prepare(
      "INSERT OR IGNORE INTO accounts (id, owner_id, name, account_type, currency, group_name, tracking_mode, meta, is_default, is_active, is_archived, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ).bind(account.id, ownerId, account.name, account.accountType, account.currency, account.group,
      account.trackingMode, JSON.stringify(account.meta), Number(account.isDefault), Number(account.isActive),
      Number(account.isArchived), account.createdAt, account.updatedAt));
    await insertBatches(env, accountStatements);

    const activityStatements = payload.activities.map((activity) => env.DB.prepare(
      "INSERT OR IGNORE INTO activity_records (id, owner_id, account_id, activity_date, asset_id, symbol, activity_type, quantity, unit_price, currency, fee, tax, amount, idempotency_key, payload_json, import_run_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ).bind(activity.id, ownerId, activity.accountId, activity.activityDate, activity.assetId, activity.symbol,
      activity.activityType, activity.quantity, activity.unitPrice, activity.currency, activity.fee, activity.tax,
      activity.amount, activity.idempotencyKey, JSON.stringify(activity.payload), importId,
      asText(activity.payload.createdAt) || new Date().toISOString()));
    await insertBatches(env, activityStatements);

    const sourceStatements = [
      ...sourceRowStatements(env, ownerId, importId, "accounts", payload.rawAccounts),
      ...sourceRowStatements(env, ownerId, importId, "activities", payload.rawActivities, (row) => dateOnly(row.date, "activity")),
      ...sourceRowStatements(env, ownerId, importId, "holdings", payload.rawHoldings, (row) => dateOnly(row.asOfDate, "holding")),
      ...sourceRowStatements(env, ownerId, importId, "portfolio_history", payload.rawHistory, (row) => dateOnly(row.valuationDate, "portfolio history")),
    ];
    await insertBatches(env, sourceStatements);

    const quoteRows = new Map();
    for (const holding of payload.holdings) {
      if (holding.price == null) continue;
      const key = holding.symbol.toUpperCase();
      const current = quoteRows.get(key);
      if (!current || holding.asOfDate >= current.asOfDate) quoteRows.set(key, holding);
    }
    const quoteStatements = [...quoteRows.values()].map((holding) => env.DB.prepare(
      "INSERT OR IGNORE INTO asset_quotes (id, owner_id, asset_id, symbol, quote_date, price, currency) VALUES (?, ?, ?, ?, ?, ?, ?)",
    ).bind(crypto.randomUUID(), ownerId, holding.symbol, holding.symbol, holding.asOfDate, holding.price, holding.currency));
    await insertBatches(env, quoteStatements);

    await env.DB.prepare("UPDATE wealthfolio_import_batches SET status = 'complete' WHERE owner_id = ? AND id = ?").bind(ownerId, importId).run();
    return { alreadyImported: false, importId, ...payload.summary };
  } catch (error) {
    await env.DB.prepare("UPDATE wealthfolio_import_batches SET status = 'in_progress' WHERE owner_id = ? AND id = ?").bind(ownerId, importId).run().catch(() => {});
    throw error;
  }
}

export async function handleWealthfolioMigrationRoute(request, route, ownerId, env) {
  if (!["/sites/migration/preview", "/sites/migration/commit"].includes(route)) return null;
  if (!ownerId) return errorResponse(401, "A private signed-in Sites session is required.");
  if (request.method !== "POST") return errorResponse(405, "Method not allowed.");
  try {
    const form = await request.formData();
    const payload = await parseExportBundle(form);
    if (route.endsWith("/preview")) return json(await preview(payload, ownerId, env));
    const confirmed = form.get("confirmed") === "true";
    const previewHash = asText(form.get("previewHash"));
    if (!confirmed || !previewHash || previewHash !== payload.sourceHash) {
      return errorResponse(400, "Confirm the reviewed preview and submit the same files.");
    }
    return json(await commit(payload, ownerId, env));
  } catch (error) {
    if (error instanceof TypeError || error instanceof SyntaxError) return errorResponse(400, error.message);
    throw error;
  }
}

export { parseExportBundle, preview as previewWealthfolioImport };
