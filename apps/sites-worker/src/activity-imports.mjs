/**
 * Activity CSV import handlers used by the Sites Worker.
 *
 * The desktop import service does considerably more asset enrichment than the
 * web worker can do.  These handlers deliberately keep the same wire shape,
 * perform the validation that is possible at this boundary, and store the
 * complete reviewed row for later processing.
 */

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };
const OWNER_HEADER = "oai-authenticated-user-id";
const MAX_CSV_BYTES = 10 * 1024 * 1024;
// D1 transactions are capped at 100 statements. 500 rows fit in one atomic
// batch of five-row INSERT statements (85 bindings apiece).
const MAX_IMPORT_ROWS = 500;

const CASH_ACTIVITY_TYPES = new Set([
  "ADJUSTMENT",
  "CASH",
  "CASH_DEPOSIT",
  "CASH_WITHDRAWAL",
  "CREDIT",
  "DEBIT",
  "DEPOSIT",
  "DIVIDEND",
  "FEE",
  "FX_CONVERSION",
  "INTEREST",
  "RETURN_OF_CAPITAL",
  "TAX",
  "TRANSFER",
  "WITHDRAWAL",
]);

function json(value, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: JSON_HEADERS,
  });
}

function errorResponse(status, message) {
  // Do not put account identifiers, transaction values, or database errors in
  // a response.  The caller only needs a stable category to display.
  return json({ error: message }, status);
}

function ownerId(request) {
  const id = request.headers.get(OWNER_HEADER)?.trim();
  return id || null;
}

async function readJson(request) {
  try {
    return await request.json();
  } catch {
    return null;
  }
}

function getActivities(body) {
  return body && Array.isArray(body.activities) ? body.activities : null;
}

function text(value) {
  return typeof value === "string" ? value.trim() : value == null ? "" : String(value).trim();
}

function field(row, camel, snake = camel) {
  return row?.[camel] ?? row?.[snake];
}

function addError(row, fieldName, message) {
  const errors = row.errors && typeof row.errors === "object" ? { ...row.errors } : {};
  const existing = Array.isArray(errors[fieldName]) ? [...errors[fieldName]] : [];
  if (!existing.includes(message)) existing.push(message);
  errors[fieldName] = existing;
  row.errors = errors;
  row.isValid = false;
}

function addWarning(row, fieldName, message) {
  const warnings = row.warnings && typeof row.warnings === "object" ? { ...row.warnings } : {};
  const existing = Array.isArray(warnings[fieldName]) ? [...warnings[fieldName]] : [];
  if (!existing.includes(message)) existing.push(message);
  warnings[fieldName] = existing;
  row.warnings = warnings;
}

function randomId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  // This fallback is only for older Node test runners.  It is not used as an
  // idempotency value and therefore does not affect duplicate detection.
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}-${Math.random().toString(36).slice(2)}`;
}

function normalizeDate(value) {
  const date = text(value);
  if (!date) return "";
  const match = date.match(/^(\d{4}-\d{2}-\d{2})(?:[T ]|$)/);
  return match ? match[1] : date;
}

function normalizeDecimal(value) {
  if (value == null || text(value) === "") return "";
  const n = Number(value);
  if (!Number.isFinite(n)) return text(value);
  return String(n).replace(/\.0+$/, "").replace(/(\.\d*?)0+$/, "$1");
}

function needsSymbol(row) {
  const type = text(field(row, "activityType", "activity_type")).toUpperCase();
  return !CASH_ACTIVITY_TYPES.has(type);
}

function validateRow(input, index) {
  const row = input && typeof input === "object" && !Array.isArray(input) ? { ...input } : {};
  const explicitlyInvalid = row.isValid === false;
  row.lineNumber = Number.isInteger(row.lineNumber) ? row.lineNumber : index + 1;
  row.isDraft = Boolean(row.isDraft);

  const account = text(field(row, "accountId", "account_id"));
  const date = normalizeDate(field(row, "date", "activityDate"));
  const type = text(field(row, "activityType", "activity_type"));
  const currency = text(row.currency);
  const symbol = text(row.symbol);
  const assetId = text(field(row, "assetId", "asset_id"));

  if (!account) addError(row, "accountId", "Account is required before importing activities.");
  if (!date || !/^\d{4}-\d{2}-\d{2}$/.test(date)) addError(row, "activityDate", "A valid activity date is required.");
  if (!type) addError(row, "activityType", "Activity type is required.");
  if (!currency) addError(row, "currency", "Currency is required.");
  if (needsSymbol(row) && !symbol && !assetId) addError(row, "symbol", "Symbol or asset_id is required for this activity.");

  row.accountId = account || row.accountId;
  row.date = date || row.date;
  row.activityType = type || row.activityType;
  row.symbol = symbol;
  row.assetId = assetId || row.assetId;
  row.isValid = !explicitlyInvalid && (!row.errors || Object.keys(row.errors).length === 0);
  if (!row.id) row.id = randomId();
  return row;
}

async function sha256Hex(value) {
  const digest = await globalThis.crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** Match the semantic fields used by core's import idempotency key. */
async function idempotencyKey(row) {
  const explicit = text(field(row, "idempotencyKey", "idempotency_key"));
  if (explicit) return explicit;
  const account = text(field(row, "accountId", "account_id"));
  const type = text(field(row, "activityType", "activity_type"));
  const date = normalizeDate(field(row, "date", "activityDate"));
  const symbol = text(row.symbol);
  const asset = text(field(row, "assetId", "asset_id")) ||
    (symbol ? `${symbol}${text(row.exchangeMic) ? `@${text(row.exchangeMic)}` : ""}` : "");
  const quantity = normalizeDecimal(row.quantity);
  const unitPrice = normalizeDecimal(field(row, "unitPrice", "unit_price"));
  const amount = normalizeDecimal(row.amount);
  const currency = text(row.currency) || "USD";
  const providerReference = text(field(row, "providerReferenceId", "provider_reference_id"));
  const comment = text(row.comment).split(/\s+/).filter(Boolean).join(" ");
  let source = `${account}|${type}|${date}|${asset}|${quantity}|${unitPrice}|${amount}|${currency}|${providerReference}|${comment}`;
  const fee = normalizeDecimal(row.fee);
  if (fee && Number(fee) !== 0) source += `\x1ffee\x1f${fee}`;
  return sha256Hex(source);
}

async function d1Rows(statement) {
  const result = await statement.all();
  return result?.results ?? result?.rows ?? [];
}

async function duplicateLookup(env, owner, keys) {
  const duplicates = new Map();
  const uniqueKeys = [...new Set(keys.filter(Boolean))];
  for (let offset = 0; offset < uniqueKeys.length; offset += 99) {
    const chunk = uniqueKeys.slice(offset, offset + 99);
    const statement = env?.DB?.prepare?.(
      `SELECT id, idempotency_key FROM activity_records WHERE owner_id = ? AND idempotency_key IN (${chunk.map(() => "?").join(",")})`,
    );
    if (!statement?.bind) throw new Error("Database unavailable");
    const rows = await d1Rows(statement.bind(owner, ...chunk));
    for (const row of rows) {
      if (row?.id && row?.idempotency_key) duplicates.set(String(row.idempotency_key), String(row.id));
    }
  }
  return duplicates;
}

async function accountIdsForOwner(env, owner, accountIds) {
  const ids = [...new Set(accountIds.filter(Boolean))];
  const owned = new Set();
  for (let offset = 0; offset < ids.length; offset += 99) {
    const chunk = ids.slice(offset, offset + 99);
    const statement = env?.DB?.prepare?.(
      `SELECT id FROM accounts WHERE owner_id = ? AND is_archived = 0 AND id IN (${chunk.map(() => "?").join(",")})`,
    );
    if (!statement?.bind) throw new Error("Database unavailable");
    for (const row of await d1Rows(statement.bind(owner, ...chunk))) {
      if (row?.id) owned.add(String(row.id));
    }
  }
  return owned;
}

function duplicateObject(map) {
  return Object.fromEntries(map.entries());
}

function parseCsvRecords(source, delimiter, quote) {
  const rows = [];
  let row = [];
  let cell = "";
  let quoted = false;
  let justClosedQuote = false;
  let malformed = false;

  for (let i = 0; i < source.length; i += 1) {
    const char = source[i];
    if (quoted) {
      if (char === quote) {
        if (source[i + 1] === quote) {
          cell += quote;
          i += 1;
        } else {
          quoted = false;
          justClosedQuote = true;
        }
      } else {
        cell += char;
      }
      continue;
    }
    if (char === quote && cell === "") {
      quoted = true;
    } else if (char === delimiter) {
      row.push(cell);
      cell = "";
      justClosedQuote = false;
    } else if (char === "\n" || char === "\r") {
      if (char === "\r" && source[i + 1] === "\n") i += 1;
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
      justClosedQuote = false;
    } else {
      if (justClosedQuote && !/\s/.test(char)) malformed = true;
      cell += char;
      justClosedQuote = false;
    }
  }
  if (quoted) malformed = true;
  if (cell !== "" || row.length) {
    row.push(cell);
    rows.push(row);
  }
  return { rows, malformed };
}

function delimiterFor(source, config) {
  const configured = config?.delimiter;
  if (configured && configured !== "auto") return configured === "\\t" ? "\t" : String(configured)[0] || ",";
  const sample = source.split(/\r?\n/).find((line) => line.trim()) || "";
  const candidates = [",", ";", "\t", "|"];
  return candidates.reduce((best, candidate) => {
    const score = sample.split(candidate).length - 1;
    return score > best.score ? { candidate, score } : best;
  }, { candidate: ",", score: 0 }).candidate;
}

function parseCsv(source, rawConfig = {}) {
  const config = rawConfig && typeof rawConfig === "object" ? rawConfig : {};
  const delimiter = delimiterFor(source, config);
  const quote = text(config.quoteChar || "\"").charAt(0) || "\"";
  const parsed = parseCsvRecords(source, delimiter, quote);
  const errors = parsed.malformed ? [{ rowIndex: null, columnIndex: null, message: "CSV contains an unmatched quote.", errorType: "parse" }] : [];
  const skipTop = Number.isInteger(config.skipTopRows) ? Math.max(0, config.skipTopRows) : 0;
  const skipBottom = Number.isInteger(config.skipBottomRows) ? Math.max(0, config.skipBottomRows) : 0;
  const skipEmpty = config.skipEmptyRows !== false;
  const working = parsed.rows.slice(skipTop, skipBottom ? -skipBottom : undefined);
  const filtered = skipEmpty ? working.filter((item) => item.some((value) => value.trim() !== "")) : working;
  if (!filtered.length) throw new Error("CSV contains no rows");
  const hasHeader = config.hasHeaderRow !== false;
  const headerIndex = Math.min(Number.isInteger(config.headerRowIndex) ? config.headerRowIndex : 0, filtered.length - 1);
  const headers = hasHeader ? filtered[headerIndex].map((value) => value.trim()) :
    Array.from({ length: Math.max(...filtered.map((item) => item.length)) }, (_, index) => `Column${index + 1}`);
  const dataRows = hasHeader ? filtered.filter((_, index) => index !== headerIndex) : filtered;
  const rows = dataRows.map((item, index) => {
    if (item.length > headers.length) {
      errors.push({ rowIndex: index, columnIndex: null, message: "Extra columns were ignored.", errorType: "structure" });
      return item.slice(0, headers.length);
    }
    return [...item, ...Array(Math.max(0, headers.length - item.length)).fill("")];
  });
  return {
    headers,
    rows,
    detectedConfig: {
      ...config,
      delimiter,
      hasHeaderRow: hasHeader,
      headerRowIndex: headerIndex,
      skipTopRows: skipTop,
      skipBottomRows: skipBottom,
      skipEmptyRows: skipEmpty,
      quoteChar: quote,
    },
    errors,
    rowCount: rows.length,
  };
}

/** POST /activities/import/parse */
export async function handleParseCsv(request) {
  if (!ownerId(request)) return errorResponse(401, "Authenticated user is required.");
  try {
    const form = await request.formData();
    const file = form.get("file");
    if (!file || typeof file.arrayBuffer !== "function") return errorResponse(400, "A CSV file is required.");
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (bytes.byteLength > MAX_CSV_BYTES) return errorResponse(413, "CSV file is too large.");
    let config = {};
    const configValue = form.get("config");
    if (configValue != null) {
      config = typeof configValue === "string" ? JSON.parse(configValue) : JSON.parse(await configValue.text());
    }
    const source = new TextDecoder("utf-8", { fatal: false }).decode(bytes).replace(/^\uFEFF/, "");
    return json(parseCsv(source, config));
  } catch {
    return errorResponse(400, "Unable to parse CSV import request.");
  }
}

/** POST /activities/import/check */
export async function handleCheckActivitiesImport(request, env) {
  const owner = ownerId(request);
  if (!owner) return errorResponse(401, "Authenticated user is required.");
  const body = await readJson(request);
  const input = getActivities(body);
  if (!input || input.length > MAX_IMPORT_ROWS) return errorResponse(400, "Invalid activity import request.");
  try {
    const checked = [];
    const firstByKey = new Map();
    const keys = [];
    for (let index = 0; index < input.length; index += 1) {
      const row = validateRow(input[index], index);
      if (row.isValid) {
        const key = await idempotencyKey(row);
        row.__idempotencyKey = key;
        keys.push(key);
        if (firstByKey.has(key)) {
          row.duplicateOfLineNumber = firstByKey.get(key);
          addWarning(row, "_duplicate", `Duplicate of line ${row.duplicateOfLineNumber} in this import batch`);
        } else {
          firstByKey.set(key, row.lineNumber);
        }
      }
      checked.push(row);
    }
    const accounts = await accountIdsForOwner(env, owner, checked.map((row) => text(row.accountId)));
    for (const row of checked) {
      if (row.isValid && !accounts.has(text(row.accountId))) addError(row, "accountId", "Account is not available for this private Site.");
    }
    const existing = await duplicateLookup(env, owner, keys);
    for (const row of checked) {
      if (row.__idempotencyKey && existing.has(row.__idempotencyKey)) {
        row.duplicateOfId = existing.get(row.__idempotencyKey);
        addWarning(row, "_duplicate", "Duplicate activity already exists");
      }
      delete row.__idempotencyKey;
    }
    return json(checked);
  } catch {
    return errorResponse(500, "Unable to check activity import.");
  }
}

/** POST /activities/import */
export async function handleImportActivities(request, env) {
  const owner = ownerId(request);
  if (!owner) return errorResponse(401, "Authenticated user is required.");
  const body = await readJson(request);
  const input = getActivities(body);
  if (!input || input.length > MAX_IMPORT_ROWS) return errorResponse(400, "Invalid activity import request.");
  const rows = input.map((row, index) => validateRow(row, index));
  if (rows.some((row) => !row.isValid)) return errorResponse(400, "Import contains invalid activities.");

  try {
    const accounts = await accountIdsForOwner(env, owner, rows.map((row) => text(row.accountId)));
    if (rows.some((row) => !accounts.has(text(row.accountId)))) {
      return errorResponse(400, "Import contains an account that is not available for this private Site.");
    }
    const keyByIndex = [];
    for (const row of rows) keyByIndex.push(await idempotencyKey(row));
    const existing = await duplicateLookup(env, owner, keyByIndex.filter((key, index) => !Boolean(rows[index].forceImport)));
    const firstByKey = new Map();
    const importRunId = randomId();
    const createdAt = new Date().toISOString();
    const resultRows = rows.map((row) => ({ ...row }));
    const toInsert = [];
    let duplicates = 0;

    for (let index = 0; index < rows.length; index += 1) {
      const row = resultRows[index];
      const key = keyByIndex[index];
      const force = Boolean(row.forceImport);
      if (!force && existing.has(key)) {
        row.duplicateOfId = existing.get(key);
        addWarning(row, "_duplicate", "Duplicate activity already exists");
        duplicates += 1;
        continue;
      }
      if (!force && firstByKey.has(key)) {
        row.duplicateOfLineNumber = firstByKey.get(key);
        addWarning(row, "_duplicate", `Duplicate of line ${row.duplicateOfLineNumber} in this import batch`);
        duplicates += 1;
        continue;
      }
      if (!force) firstByKey.set(key, row.lineNumber);
      toInsert.push({ row, key: force ? null : key });
    }

    const records = toInsert.map(({ row, key }) => {
      const accountId = text(field(row, "accountId", "account_id"));
      const date = normalizeDate(field(row, "date", "activityDate"));
      const activityType = text(field(row, "activityType", "activity_type"));
      const exact = (value) => value == null || text(value) === "" ? null : String(value);
      return [
        text(row.id) || randomId(),
        owner,
        accountId,
        date,
        text(field(row, "assetId", "asset_id")) || null,
        text(row.symbol) || null,
        activityType,
        exact(row.quantity),
        exact(field(row, "unitPrice", "unit_price")),
        text(row.currency) || null,
        exact(row.fee),
        exact(row.tax),
        exact(row.amount),
        key,
        JSON.stringify(row),
        importRunId,
        createdAt,
      ];
    });
    const statements = [];
    for (let offset = 0; offset < records.length; offset += 5) {
      const chunk = records.slice(offset, offset + 5);
      const values = chunk.map(() => "(" + Array.from({ length: 17 }, () => "?").join(",") + ")").join(",");
      const statement = env?.DB?.prepare?.(
        `INSERT INTO activity_records (id, owner_id, account_id, activity_date, asset_id, symbol, activity_type, quantity, unit_price, currency, fee, tax, amount, idempotency_key, payload_json, import_run_id, created_at) VALUES ${values}`,
      );
      if (!statement?.bind) throw new Error("Database unavailable");
      statements.push(statement.bind(...chunk.flat()));
    }
    if (statements.length) await env.DB.batch(statements);

    return json({
      activities: resultRows,
      importRunId,
      summary: {
        total: rows.length,
        imported: toInsert.length,
        skipped: duplicates,
        duplicates,
        assetsCreated: 0,
        success: true,
        errorMessage: null,
      },
    });
  } catch {
    return errorResponse(500, "Unable to import activities.");
  }
}

/** POST /activities/import/check-duplicates */
export async function handleCheckExistingDuplicates(request, env) {
  const owner = ownerId(request);
  if (!owner) return errorResponse(401, "Authenticated user is required.");
  const body = await readJson(request);
  if (!body || !Array.isArray(body.idempotencyKeys) || body.idempotencyKeys.length > MAX_IMPORT_ROWS) {
    return errorResponse(400, "Invalid duplicate lookup request.");
  }
  try {
    const duplicates = await duplicateLookup(env, owner, body.idempotencyKeys.map(text));
    return json({ duplicates: duplicateObject(duplicates) });
  } catch {
    return errorResponse(500, "Unable to check existing duplicates.");
  }
}
