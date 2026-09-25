import { createAccount } from "./accounts.mjs";
import { getActivityCategories, getActivityTaxonomies, ensureDefaultTaxonomies } from "./taxonomies.mjs";

const MAX_ASSIGNMENTS = 1_000;
const MAX_CONTEXT_IDS = 100;
const MATCH_TYPES = new Set(["contains", "starts_with", "exact", "regex"]);
const INVESTMENT_TYPES = new Set(["BUY", "SELL", "COVER", "SHORT", "RECEIVE", "RECEIVED", "DISPOSE", "DELIVER", "TRANSFER_IN", "TRANSFER_OUT"]);

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

async function readJson(request) {
  try {
    return await request.json();
  } catch {
    return null;
  }
}

async function queryRows(env, sql, ...values) {
  const statement = env?.DB?.prepare?.(sql);
  if (!statement?.bind) throw new Error("Database unavailable");
  const result = await statement.bind(...values).all();
  return result?.results ?? result?.rows ?? [];
}

async function getOwnedIds(env, table, ownerId, ids) {
  const found = new Set();
  for (let offset = 0; offset < ids.length; offset += 99) {
    const chunk = ids.slice(offset, offset + 99);
    if (!chunk.length) continue;
    const rows = await queryRows(
      env,
      `SELECT id FROM ${table} WHERE owner_id = ? AND id IN (${chunk.map(() => "?").join(",")})`,
      ownerId,
      ...chunk,
    );
    for (const row of rows) if (row.id) found.add(String(row.id));
  }
  return found;
}

async function resolveCategories(ownerId, assignments, env) {
  if (!assignments.length) return new Map();
  await ensureDefaultTaxonomies(ownerId, env);
  const categories = new Map();
  for (let offset = 0; offset < assignments.length; offset += 45) {
    const chunk = assignments.slice(offset, offset + 45);
    const taxonomyIds = [...new Set(chunk.map((item) => item.taxonomyId))];
    const keys = [...new Set(chunk.map((item) => item.categoryKey))];
    const rows = await queryRows(
      env,
      `SELECT c.taxonomy_id, c.category_key, c.id FROM categories c JOIN taxonomies t ON t.owner_id = c.owner_id AND t.id = c.taxonomy_id WHERE c.owner_id = ? AND c.is_active = 1 AND t.scope = 'activity' AND c.taxonomy_id IN (${taxonomyIds.map(() => "?").join(",")}) AND c.category_key IN (${keys.map(() => "?").join(",")})`,
      ownerId,
      ...taxonomyIds,
      ...keys,
    );
    for (const row of rows) categories.set(`${row.taxonomy_id}\u001f${row.category_key}`, String(row.id));
  }
  return categories;
}

function validateAssignmentInputs(value) {
  if (!Array.isArray(value) || value.length > MAX_ASSIGNMENTS) throw new TypeError(`assignments must contain at most ${MAX_ASSIGNMENTS} items.`);
  const assignments = value.map((entry) => {
    if (!isRecord(entry)) throw new TypeError("Each assignment must be an object.");
    const activityId = text(entry.activityId);
    const taxonomyId = text(entry.taxonomyId);
    const categoryKey = text(entry.categoryKey);
    if (!activityId || !taxonomyId || !categoryKey) throw new TypeError("Each assignment requires activityId, taxonomyId, and categoryKey.");
    if (activityId.length > 128 || taxonomyId.length > 128 || categoryKey.length > 128) throw new TypeError("Assignment values are too long.");
    return { activityId, taxonomyId, categoryKey };
  });
  const unique = new Set(assignments.map((item) => `${item.activityId}\u001f${item.taxonomyId}`));
  if (unique.size !== assignments.length) throw new TypeError("An activity can have only one assignment per taxonomy in a call.");
  return assignments;
}

async function applyAssignments(ownerId, assignments, env, auditAction) {
  if (!assignments.length) return [];
  const keyAssignments = assignments.filter((item) => !item.categoryId);
  const categoryMap = await resolveCategories(ownerId, keyAssignments, env);
  const normalized = assignments.map((item) => {
    const categoryId = item.categoryId ?? categoryMap.get(`${item.taxonomyId}\u001f${item.categoryKey}`);
    if (!categoryId) throw new TypeError("Unknown activity taxonomy or category key.");
    return { ...item, categoryId };
  });
  const validCategoryPairs = new Set();
  for (let offset = 0; offset < normalized.length; offset += 49) {
    const chunk = normalized.slice(offset, offset + 49);
    const taxonomyIds = [...new Set(chunk.map((item) => item.taxonomyId))];
    const categoryIds = [...new Set(chunk.map((item) => item.categoryId))];
    const validCategories = await queryRows(
      env,
      `SELECT c.id, c.taxonomy_id FROM categories c JOIN taxonomies t ON t.owner_id = c.owner_id AND t.id = c.taxonomy_id WHERE c.owner_id = ? AND c.is_active = 1 AND t.scope = 'activity' AND c.taxonomy_id IN (${taxonomyIds.map(() => "?").join(",")}) AND c.id IN (${categoryIds.map(() => "?").join(",")})`,
      ownerId,
      ...taxonomyIds,
      ...categoryIds,
    );
    for (const row of validCategories) validCategoryPairs.add(`${row.taxonomy_id}\u001f${row.id}`);
  }
  if (normalized.some((item) => !validCategoryPairs.has(`${item.taxonomyId}\u001f${item.categoryId}`))) throw new TypeError("Unknown activity taxonomy or category.");
  const activities = await getOwnedIds(env, "activity_records", ownerId, [...new Set(normalized.map((item) => item.activityId))]);
  if (normalized.some((item) => !activities.has(item.activityId))) throw new TypeError("An activity is not available for this private Site.");

  const statements = [];
  for (let offset = 0; offset < normalized.length; offset += 20) {
    const chunk = normalized.slice(offset, offset + 20);
    const values = chunk.map(() => "(?, ?, ?, ?, ?, CURRENT_TIMESTAMP)").join(",");
    statements.push(
      env.DB.prepare(
        `INSERT INTO activity_category_assignments (id, owner_id, activity_id, taxonomy_id, category_id, created_at) VALUES ${values} ON CONFLICT(owner_id, activity_id, taxonomy_id) DO UPDATE SET category_id = excluded.category_id, created_at = CURRENT_TIMESTAMP`,
      ).bind(...chunk.flatMap((item) => [crypto.randomUUID(), ownerId, item.activityId, item.taxonomyId, item.categoryId])),
    );
  }
  if (auditAction) {
    statements.push(
      env.DB.prepare("INSERT INTO mcp_audit_logs (id, owner_id, action, result_count) VALUES (?, ?, ?, ?)")
        .bind(crypto.randomUUID(), ownerId, auditAction, normalized.length),
    );
  }
  await env.DB.batch(statements);
  return normalized.map(({ activityId, taxonomyId, categoryId }) => ({ activityId, taxonomyId, categoryId }));
}

function payload(row) {
  try {
    const value = JSON.parse(row.payload_json ?? "{}");
    return isRecord(value) ? value : {};
  } catch {
    return {};
  }
}

function activityNotes(row) {
  const source = payload(row);
  return text(source.notes ?? source.comment ?? source.memo ?? row.notes) || null;
}

function activityDate(row) {
  return text(row.activity_date ?? payload(row).date);
}

function amountNumber(row) {
  const value = row.amount ?? payload(row).amount;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function normalizedFilters(input) {
  if (!isRecord(input)) throw new TypeError("Input must be an object.");
  const activityIds = input.activityIds === undefined ? [] : input.activityIds;
  const accountIds = input.accountIds === undefined ? [] : input.accountIds;
  if (!Array.isArray(activityIds) || !Array.isArray(accountIds)) throw new TypeError("activityIds and accountIds must be arrays.");
  if (activityIds.length > MAX_CONTEXT_IDS || accountIds.length > MAX_CONTEXT_IDS) throw new TypeError(`At most ${MAX_CONTEXT_IDS} IDs may be supplied.`);
  const ids = [...activityIds, ...accountIds];
  if (ids.some((id) => typeof id !== "string" || !id.trim() || id.length > 128)) throw new TypeError("IDs must be non-empty strings of at most 128 characters.");
  const status = input.status === undefined ? "uncategorized" : input.status;
  if (!new Set(["uncategorized", "all", "needs_review"]).has(status)) throw new TypeError("Unsupported categorization status.");
  const startDate = input.startDate === undefined ? "" : input.startDate;
  const endDate = input.endDate === undefined ? "" : input.endDate;
  for (const [name, date] of [["startDate", startDate], ["endDate", endDate]]) {
    if (date && (typeof date !== "string" || !/^\d{4}-\d{2}-\d{2}/.test(date))) throw new TypeError(`${name} must be an ISO date.`);
  }
  const limit = input.limit === undefined ? 100 : input.limit;
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) throw new TypeError("limit must be between 1 and 100.");
  return { activityIds, accountIds, status, startDate, endDate, limit };
}

async function categorizationState(ownerId, filters, env) {
  await ensureDefaultTaxonomies(ownerId, env);
  const [activities, assignments, taxonomyRows, categories] = await Promise.all([
    queryRows(env, "SELECT id, owner_id, account_id, activity_date, activity_type, amount, currency, payload_json FROM activity_records WHERE owner_id = ? ORDER BY activity_date DESC LIMIT 2000", ownerId),
    queryRows(env, "SELECT activity_id, taxonomy_id, category_id FROM activity_category_assignments WHERE owner_id = ?", ownerId),
    getActivityTaxonomies(ownerId, env),
    getActivityCategories(ownerId, env),
  ]);
  const assignmentByActivity = new Map();
  for (const item of assignments) {
    const values = assignmentByActivity.get(String(item.activity_id)) ?? [];
    values.push(item);
    assignmentByActivity.set(String(item.activity_id), values);
  }
  const categoriesById = new Map(categories.map((category) => [category.id, category]));
  const taxonomySummaries = taxonomyRows.map((taxonomy) => ({
    taxonomyId: taxonomy.id,
    taxonomyName: taxonomy.name,
    categories: categories.filter((category) => category.taxonomyId === taxonomy.id).map((category) => {
      let path = category.name;
      const parent = category.parentId ? categoriesById.get(category.parentId) : undefined;
      if (parent) path = `${parent.name} / ${path}`;
      return { categoryId: category.id, key: category.key, name: category.name, path, color: category.color };
    }),
  }));
  let rows = activities.filter((row) => !INVESTMENT_TYPES.has(text(row.activity_type).toUpperCase()) && row.amount !== null && row.amount !== undefined);
  if (filters.activityIds.length) {
    const idSet = new Set(filters.activityIds.map(String));
    rows = rows.filter((row) => idSet.has(String(row.id)));
  }
  if (filters.accountIds.length) {
    const accountSet = new Set(filters.accountIds.map(String));
    rows = rows.filter((row) => accountSet.has(String(row.account_id)));
  }
  if (filters.startDate) rows = rows.filter((row) => activityDate(row) >= filters.startDate);
  if (filters.endDate) rows = rows.filter((row) => activityDate(row) <= filters.endDate);
  if (filters.status === "uncategorized") rows = rows.filter((row) => !(assignmentByActivity.get(String(row.id)) ?? []).length);
  if (filters.status === "needs_review") rows = rows.filter((row) => payload(row).needsReview === true || payload(row).needs_review === true);
  const total = rows.length;
  rows = rows.slice(0, filters.limit);
  const unproposed = rows.map((row) => ({
    activityId: String(row.id),
    activityDate: activityDate(row),
    amount: amountNumber(row),
    currency: text(row.currency) || "USD",
    notes: activityNotes(row),
    reason: payload(row).needsReview || payload(row).needs_review ? "Needs review" : "No activity category assigned",
  }));
  const examples = [];
  for (const row of activities) {
    if (examples.length >= 20) break;
    const notes = activityNotes(row);
    for (const assignment of assignmentByActivity.get(String(row.id)) ?? []) {
      const category = categoriesById.get(String(assignment.category_id));
      if (notes && category) examples.push({ categoryId: category.id, categoryPath: category.parentId ? `${categoriesById.get(category.parentId)?.name ?? ""} / ${category.name}` : category.name, notes });
      if (examples.length >= 20) break;
    }
  }
  return { total, taxonomySummaries, examples, unproposed };
}

async function listCategorizationContext(request, ownerId, env) {
  const body = await readJson(request);
  try {
    const filters = normalizedFilters(body ?? {});
    const state = await categorizationState(ownerId, filters, env);
    const needsAiJudgement = state.unproposed.length;
    return json({
      taxonomies: state.taxonomySummaries,
      examples: state.examples,
      unproposed: state.unproposed,
      summary: { total: state.total, deterministicallyProposed: 0, needsAiJudgement },
      nextStep: state.total === 0 ? "No matching uncategorized transactions were found." : "Propose categories for each unproposed row, then request confirmation before committing assignments.",
    });
  } catch (error) {
    if (error instanceof TypeError) return json({ message: error.message }, 400);
    throw error;
  }
}

async function proposeTransactionCategories(request, ownerId, env) {
  const body = await readJson(request);
  try {
    const filters = normalizedFilters(body ?? {});
    const aiProposals = body.aiProposals ?? [];
    if (!Array.isArray(aiProposals) || aiProposals.length > filters.limit) throw new TypeError("aiProposals must be an array within the requested limit.");
    const state = await categorizationState(ownerId, filters, env);
    const availableIds = new Set(state.unproposed.map((row) => row.activityId));
    const categoryMap = await resolveCategories(ownerId, aiProposals.map((item) => ({ taxonomyId: text(item?.taxonomyId), categoryKey: text(item?.categoryKey) })), env);
    const proposals = [];
    const usedIds = new Set();
    for (const proposal of aiProposals) {
      if (!isRecord(proposal)) throw new TypeError("Each aiProposal must be an object.");
      const activityId = text(proposal.activityId);
      const taxonomyId = text(proposal.taxonomyId);
      const categoryKey = text(proposal.categoryKey);
      if (!activityId || !taxonomyId || !categoryKey || !availableIds.has(activityId)) throw new TypeError("Each proposal must target an uncategorized activity from the selected context.");
      if (usedIds.has(activityId)) throw new TypeError("An activity may have one proposal per call.");
      usedIds.add(activityId);
      const categoryId = categoryMap.get(`${taxonomyId}\u001f${categoryKey}`);
      if (!categoryId) throw new TypeError("Unknown activity taxonomy or category key.");
      const confidence = proposal.confidence === undefined ? 0.7 : proposal.confidence;
      if (typeof confidence !== "number" || !Number.isFinite(confidence) || confidence < 0 || confidence > 1) throw new TypeError("confidence must be between 0 and 1.");
      const source = state.unproposed.find((item) => item.activityId === activityId);
      const taxonomy = state.taxonomySummaries.find((item) => item.taxonomyId === taxonomyId);
      const category = taxonomy?.categories.find((item) => item.categoryId === categoryId);
      proposals.push({
        activityId,
        activityDate: source.activityDate,
        amount: source.amount,
        currency: source.currency,
        notes: source.notes,
        taxonomyId,
        categoryId,
        categoryPath: category?.path ?? categoryKey,
        confidence,
        source: "ai",
        explanation: typeof proposal.reason === "string" ? proposal.reason.slice(0, 1000) : "Suggested from transaction details.",
      });
    }
    const avgConfidence = proposals.length ? proposals.reduce((sum, item) => sum + item.confidence, 0) / proposals.length : 0;
    const unproposed = state.unproposed.filter((item) => !usedIds.has(item.activityId));
    return json({
      proposals,
      unproposed,
      summary: { total: state.total, proposed: proposals.length, unproposed: Math.max(0, state.total - proposals.length), avgConfidence },
      taxonomies: state.taxonomySummaries,
      examples: state.examples,
      draft_status: "draft",
    });
  } catch (error) {
    if (error instanceof TypeError) return json({ message: error.message }, 400);
    throw error;
  }
}

async function commitCategoryAssignments(request, ownerId, env) {
  const body = await readJson(request);
  try {
    if (!isRecord(body)) throw new TypeError("Input must be an object.");
    const assignments = validateAssignmentInputs(body.assignments);
    const applied = await applyAssignments(ownerId, assignments, env, "commit_category_assignments");
    return json({ applied });
  } catch (error) {
    if (error instanceof TypeError) return json({ message: error.message }, 400);
    throw error;
  }
}

async function commitCategorizationRule(request, ownerId, env) {
  const body = await readJson(request);
  try {
    if (!isRecord(body)) throw new TypeError("Input must be an object.");
    const pattern = text(body.pattern);
    const taxonomyId = text(body.taxonomyId);
    const categoryKey = text(body.categoryKey);
    const matchType = text(body.matchType || "contains");
    const accountId = text(body.accountId) || null;
    if (!pattern || new TextEncoder().encode(pattern).length > 512) throw new TypeError("pattern must contain 1 to 512 bytes.");
    if (!taxonomyId || !categoryKey) throw new TypeError("taxonomyId and categoryKey are required.");
    if (!MATCH_TYPES.has(matchType)) throw new TypeError("Unsupported categorization rule matchType.");
    if (matchType === "regex") {
      try { new RegExp(pattern); } catch { throw new TypeError("pattern is not a valid regular expression."); }
    }
    const categories = await resolveCategories(ownerId, [{ taxonomyId, categoryKey }], env);
    const categoryId = categories.get(`${taxonomyId}\u001f${categoryKey}`);
    if (!categoryId) throw new TypeError("Unknown activity taxonomy or category key.");
    if (accountId && !(await getOwnedIds(env, "accounts", ownerId, [accountId])).has(accountId)) throw new TypeError("Account is not available for this private Site.");
    const id = crypto.randomUUID();
    const name = text(body.name).slice(0, 120) || `${pattern.slice(0, 96)} → ${categoryKey}`;
    const activityType = text(body.activityType).slice(0, 64) || null;
    await env.DB.batch([
      env.DB.prepare(
        "INSERT INTO categorization_rules (id, owner_id, name, pattern, match_type, taxonomy_id, category_id, activity_type, account_id, is_global, is_enabled) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
      ).bind(id, ownerId, name, pattern, matchType, taxonomyId, categoryId, activityType, accountId, Number(!accountId)),
      env.DB.prepare("INSERT INTO mcp_audit_logs (id, owner_id, action, result_count) VALUES (?, ?, ?, 1)")
        .bind(crypto.randomUUID(), ownerId, "commit_categorization_rule"),
    ]);
    return json({ rule: { id, name, pattern, matchType, taxonomyId, categoryId, activityType, accountId, isGlobal: !accountId, isEnabled: true } }, 201);
  } catch (error) {
    if (error instanceof TypeError) return json({ message: error.message }, 400);
    throw error;
  }
}

async function handleSpendingAssignments(request, route, ownerId, env) {
  if (route === "/spending/assignments/bulk" && request.method === "POST") {
    const body = await readJson(request);
    const items = Array.isArray(body) ? body : body?.items;
    try {
      if (!Array.isArray(items) || items.length > MAX_ASSIGNMENTS) throw new TypeError("Invalid category assignments.");
      const assignments = items.map((item) => {
        if (!isRecord(item)) throw new TypeError("Each assignment must be an object.");
        const activityId = text(item.activityId ?? item.activity_id);
        const taxonomyId = text(item.taxonomyId ?? item.taxonomy_id);
        const categoryId = text(item.categoryId ?? item.category_id);
        if (!activityId || !taxonomyId || !categoryId) throw new TypeError("Each assignment requires activityId, taxonomyId, and categoryId.");
        return { activityId, taxonomyId, categoryId };
      });
      const applied = await applyAssignments(ownerId, assignments, env);
      return json({ applied: applied.length });
    } catch (error) {
      if (error instanceof TypeError) return json({ message: error.message }, 400);
      throw error;
    }
  }
  const match = route.match(/^\/spending\/activities\/([^/]+)\/assignments(?:\/([^/]+))?$/);
  if (!match) return null;
  const activityId = decodeURIComponent(match[1]);
  const taxonomyId = match[2] ? decodeURIComponent(match[2]) : "";
  if (request.method === "GET" && !taxonomyId) {
    const assignments = await queryRows(
      env,
      "SELECT a.id, a.activity_id, a.taxonomy_id, a.category_id, c.category_key, c.name FROM activity_category_assignments a JOIN categories c ON c.owner_id = a.owner_id AND c.id = a.category_id WHERE a.owner_id = ? AND a.activity_id = ? ORDER BY a.taxonomy_id",
      ownerId,
      activityId,
    );
    return json(assignments.map((item) => ({ id: item.id, activityId: item.activity_id, taxonomyId: item.taxonomy_id, categoryId: item.category_id, categoryKey: item.category_key, categoryName: item.name })));
  }
  if (request.method === "PUT" && !taxonomyId) {
    const body = await readJson(request);
    try {
      if (!isRecord(body) || !text(body.taxonomyId) || !text(body.categoryId)) throw new TypeError("taxonomyId and categoryId are required.");
      const found = await queryRows(env, "SELECT c.id FROM categories c JOIN taxonomies t ON t.owner_id = c.owner_id AND t.id = c.taxonomy_id WHERE c.owner_id = ? AND c.taxonomy_id = ? AND c.id = ? AND c.is_active = 1 AND t.scope = 'activity'", ownerId, body.taxonomyId, body.categoryId);
      if (!found.length) throw new TypeError("Unknown activity taxonomy or category.");
      const applied = await applyAssignments(ownerId, [{ activityId, taxonomyId: body.taxonomyId, categoryId: body.categoryId }], env);
      return json({ id: `${activityId}:${body.taxonomyId}`, activityId, taxonomyId: body.taxonomyId, categoryId: applied[0].categoryId, weight: 10000, source: "manual" });
    } catch (error) {
      if (error instanceof TypeError) return json({ message: error.message }, 400);
      throw error;
    }
  }
  if (request.method === "DELETE" && taxonomyId) {
    await env.DB.prepare("DELETE FROM activity_category_assignments WHERE owner_id = ? AND activity_id = ? AND taxonomy_id = ?")
      .bind(ownerId, activityId, taxonomyId).run();
    return json({ success: true });
  }
  return null;
}

export async function handleMcpRoute(request, route, ownerId, env) {
  if (request.method === "POST") {
    if (route === "/sites/mcp/list-categorization-context") return listCategorizationContext(request, ownerId, env);
    if (route === "/sites/mcp/propose-transaction-categories") return proposeTransactionCategories(request, ownerId, env);
    if (route === "/sites/mcp/commit-category-assignments") return commitCategoryAssignments(request, ownerId, env);
    if (route === "/sites/mcp/commit-categorization-rule") return commitCategorizationRule(request, ownerId, env);
    if (route === "/sites/mcp/create-account") {
      const body = await readJson(request);
      try {
        const account = await createAccount(body, ownerId, env, { auditAction: "create_account" });
        return json({ account }, 201);
      } catch (error) {
        if (error instanceof TypeError) return json({ message: error.message }, 400);
        throw error;
      }
    }
  }
  return null;
}

export async function handleSpendingCategoryRoute(request, route, ownerId, env) {
  return route.startsWith("/spending/") ? handleSpendingAssignments(request, route, ownerId, env) : null;
}
