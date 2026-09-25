const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };
const MAX_PAGE_SIZE = 1_000;
const SORT_COLUMNS = new Map([
  ["date", "a.activity_date"],
  ["activityDate", "a.activity_date"],
  ["amount", "a.amount"],
  ["quantity", "a.quantity"],
  ["unitPrice", "a.unit_price"],
  ["activityType", "a.activity_type"],
  ["assetSymbol", "a.symbol"],
  ["symbol", "a.symbol"],
  ["accountName", "accounts.name"],
  ["createdAt", "a.created_at"],
]);

function json(value, status = 200) {
  return new Response(JSON.stringify(value), { status, headers: JSON_HEADERS });
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}

function stringArray(value) {
  if (value === undefined || value === null) return [];
  const values = Array.isArray(value) ? value : [value];
  return [...new Set(values.map(text).filter(Boolean))];
}

function parsePayload(row) {
  try {
    const value = JSON.parse(row.payload_json ?? "{}");
    return value && typeof value === "object" && !Array.isArray(value) ? value : {};
  } catch {
    return {};
  }
}

function dateValue(value) {
  const date = text(value);
  if (!date) return null;
  return /^\d{4}-\d{2}-\d{2}$/.test(date) ? `${date}T00:00:00.000Z` : date;
}

function mapActivity(row) {
  const payload = parsePayload(row);
  const asset = payload.asset && typeof payload.asset === "object" ? payload.asset : {};
  const date = dateValue(row.activity_date ?? payload.activityDate ?? payload.date);
  const createdAt = row.created_at ?? payload.createdAt ?? date ?? new Date(0).toISOString();
  const status = text(payload.status).toUpperCase();
  const needsReview = payload.needsReview === true || payload.needs_review === true || status === "DRAFT" || status === "PENDING";
  return {
    id: String(row.id),
    activityType: String(row.activity_type ?? payload.activityType ?? "UNKNOWN"),
    subtype: payload.subtype ?? null,
    status: ["POSTED", "PENDING", "DRAFT", "VOID"].includes(status) ? status : needsReview ? "PENDING" : "POSTED",
    date,
    quantity: row.quantity ?? payload.quantity ?? null,
    unitPrice: row.unit_price ?? payload.unitPrice ?? null,
    amount: row.amount ?? payload.amount ?? null,
    fee: row.fee ?? payload.fee ?? null,
    tax: row.tax ?? payload.tax ?? null,
    currency: row.currency ?? payload.currency ?? "USD",
    needsReview,
    comment: payload.comment ?? payload.notes ?? payload.memo ?? null,
    fxRate: payload.fxRate ?? null,
    createdAt,
    assetId: String(row.asset_id ?? payload.assetId ?? asset.id ?? ""),
    updatedAt: payload.updatedAt ?? createdAt,
    accountId: String(row.account_id),
    accountName: String(row.account_name ?? ""),
    accountCurrency: String(row.account_currency ?? "USD"),
    assetSymbol: String(row.symbol ?? payload.symbol ?? asset.symbol ?? ""),
    assetName: asset.name ?? payload.assetName ?? undefined,
    assetQuoteMode: asset.quoteMode ?? undefined,
    exchangeMic: asset.exchangeMic ?? payload.exchangeMic ?? undefined,
    instrumentType: asset.instrumentType ?? payload.instrumentType ?? undefined,
    assetContractMultiplier: asset.contractMultiplier ?? payload.contractMultiplier ?? null,
    sourceSystem: payload.sourceSystem ?? undefined,
    sourceRecordId: payload.sourceRecordId ?? undefined,
    sourceGroupId: payload.sourceGroupId ?? undefined,
    idempotencyKey: row.idempotency_key ?? payload.idempotencyKey ?? undefined,
    importRunId: row.import_run_id ?? payload.importRunId ?? undefined,
    isUserModified: payload.isUserModified ?? undefined,
    metadata: payload.metadata && typeof payload.metadata === "object" ? payload.metadata : undefined,
    transferOutId: payload.transferOutId ?? undefined,
    transferInId: payload.transferInId ?? undefined,
    counterpartActivityId: payload.counterpartActivityId ?? undefined,
    counterpartAccountId: payload.counterpartAccountId ?? undefined,
    counterpartAmount: payload.counterpartAmount ?? null,
    counterpartCurrency: payload.counterpartCurrency ?? null,
    counterpartFxRate: payload.counterpartFxRate ?? null,
  };
}

function normalizeSearch(body) {
  const input = body && typeof body === "object" && !Array.isArray(body) ? body : {};
  const page = Number.isInteger(input.page) && input.page >= 0 ? input.page : 0;
  const requestedSize = Number.isInteger(input.pageSize) && input.pageSize > 0 ? input.pageSize : 50;
  const sort = input.sort && typeof input.sort === "object" ? input.sort : {};
  return {
    page,
    pageSize: Math.min(requestedSize, MAX_PAGE_SIZE),
    accountIds: stringArray(input.accountIdFilter),
    activityTypes: stringArray(input.activityTypeFilter).map((value) => value.toUpperCase()),
    activityIds: stringArray(input.activityIdFilter),
    instrumentTypes: stringArray(input.instrumentTypeFilter).map((value) => value.toUpperCase()),
    keyword: text(input.assetIdKeyword).toUpperCase(),
    needsReview: typeof input.needsReviewFilter === "boolean" ? input.needsReviewFilter : undefined,
    dateFrom: text(input.dateFrom),
    dateTo: text(input.dateTo),
    sortColumn: SORT_COLUMNS.get(text(sort.id)) ?? "a.activity_date",
    sortDirection: sort.desc === true ? "DESC" : "ASC",
  };
}

/** Handle POST /activities/search for the Sites D1 profile. */
export async function handleActivitySearchRoute(request, route, ownerId, env) {
  if (route !== "/activities/search") return null;
  if (request.method !== "POST") return json({ message: "Method not allowed." }, 405);
  if (!text(ownerId)) return json({ message: "Authenticated user is required." }, 401);
  let body;
  try {
    body = await request.json();
  } catch {
    return json({ message: "Invalid activity search request." }, 400);
  }

  try {
    const filters = normalizeSearch(body);
    const where = ["a.owner_id = ?"];
    const values = [ownerId];
    const addInFilter = (column, items) => {
      if (!items.length) return;
      where.push(`${column} IN (${items.map(() => "?").join(",")})`);
      values.push(...items);
    };
    addInFilter("a.account_id", filters.accountIds);
    addInFilter("UPPER(a.activity_type)", filters.activityTypes);
    addInFilter("a.id", filters.activityIds);
    if (filters.keyword) {
      where.push("(UPPER(COALESCE(a.symbol, '')) LIKE ? OR UPPER(COALESCE(a.asset_id, '')) LIKE ?)");
      const pattern = `%${filters.keyword.replace(/[\\%_]/g, "\\$&")}%`;
      values.push(pattern, pattern);
    }
    if (filters.dateFrom && /^\d{4}-\d{2}-\d{2}$/.test(filters.dateFrom)) {
      where.push("a.activity_date >= ?");
      values.push(filters.dateFrom);
    }
    if (filters.dateTo && /^\d{4}-\d{2}-\d{2}$/.test(filters.dateTo)) {
      where.push("a.activity_date <= ?");
      values.push(filters.dateTo);
    }
    if (filters.needsReview !== undefined) {
      where.push(`COALESCE(json_extract(a.payload_json, '$.needsReview'), json_extract(a.payload_json, '$.needs_review'), CASE WHEN UPPER(COALESCE(json_extract(a.payload_json, '$.status'), '')) IN ('DRAFT', 'PENDING') THEN 1 ELSE 0 END, 0) = ?`);
      values.push(Number(filters.needsReview));
    }
    addInFilter(
      "UPPER(COALESCE(json_extract(a.payload_json, '$.asset.instrumentType'), json_extract(a.payload_json, '$.instrumentType'), ''))",
      filters.instrumentTypes,
    );
    const whereClause = where.join(" AND ");
    const countStatement = env?.DB?.prepare?.(
      `SELECT COUNT(*) AS totalRowCount FROM activity_records a JOIN accounts ON accounts.id = a.account_id AND accounts.owner_id = a.owner_id WHERE ${whereClause}`,
    );
    const statement = env?.DB?.prepare?.(
      `SELECT a.id, a.account_id, a.activity_date, a.asset_id, a.symbol, a.activity_type, a.quantity, a.unit_price, a.currency, a.fee, a.tax, a.amount, a.idempotency_key, a.payload_json, a.import_run_id, a.created_at, accounts.name AS account_name, accounts.currency AS account_currency FROM activity_records a JOIN accounts ON accounts.id = a.account_id AND accounts.owner_id = a.owner_id WHERE ${whereClause} ORDER BY ${filters.sortColumn} COLLATE NOCASE ${filters.sortDirection}, a.id ${filters.sortDirection} LIMIT ? OFFSET ?`,
    );
    if (!countStatement?.bind || !statement?.bind) throw new Error("Database unavailable");
    const countRow = await countStatement.bind(...values).first();
    const totalRowCount = Number(countRow?.totalRowCount ?? 0);
    const offset = filters.page * filters.pageSize;
    const result = await statement.bind(...values, filters.pageSize, offset).all();
    const rows = (result?.results ?? result?.rows ?? []).map(mapActivity);
    return json({ data: rows, meta: { totalRowCount } });
  } catch {
    return json({ message: "Unable to search activities." }, 500);
  }
}
