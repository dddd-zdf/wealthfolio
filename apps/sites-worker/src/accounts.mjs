function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function mapAccount(row) {
  return {
    id: row.id,
    name: row.name,
    accountType: row.account_type,
    currency: row.currency,
    group: row.group_name ?? undefined,
    balance: 0,
    isDefault: Boolean(row.is_default),
    isActive: Boolean(row.is_active),
    isArchived: Boolean(row.is_archived),
    trackingMode: row.tracking_mode ?? "NOT_SET",
    createdAt: row.created_at,
    updatedAt: row.updated_at,
    meta: row.meta ?? undefined,
  };
}

function normalizeAccount(input) {
  const name = typeof input?.name === "string" ? input.name.trim() : "";
  const accountType = typeof input?.accountType === "string" ? input.accountType.trim().toUpperCase() : "";
  const currency = typeof input?.currency === "string" ? input.currency.trim().toUpperCase() : "";
  const trackingMode = typeof input?.trackingMode === "string" ? input.trackingMode.trim().toUpperCase() : "NOT_SET";
  const allowedTypes = new Set(["SECURITIES", "CASH", "CREDIT_CARD", "CRYPTOCURRENCY"]);
  if (name.length < 2 || name.length > 50) throw new TypeError("Account name must contain 2 to 50 characters.");
  if (!allowedTypes.has(accountType)) throw new TypeError("Unknown account type.");
  if (!/^[A-Z]{3}$/.test(currency)) throw new TypeError("Currency must be a three-letter ISO code.");
  if (!["TRANSACTIONS", "HOLDINGS", "NOT_SET"].includes(trackingMode)) throw new TypeError("Unknown account tracking mode.");
  if (accountType === "CREDIT_CARD" && trackingMode === "HOLDINGS") throw new TypeError("Credit card accounts cannot use holdings tracking mode.");
  const group = typeof input.group === "string" ? input.group.trim().slice(0, 120) || null : null;
  const meta = typeof input.meta === "string" ? input.meta : null;
  return {
    name,
    accountType,
    currency,
    trackingMode,
    meta,
    group,
    isDefault: input.isDefault === true,
    isActive: input.isActive !== false,
    isArchived: input.isArchived === true,
  };
}

function accountId(input) {
  return typeof input.id === "string" && /^[\w-]{1,80}$/.test(input.id) ? input.id : crypto.randomUUID();
}

async function getAccount(ownerId, id, env) {
  return env.DB.prepare(
    "SELECT id, name, account_type, currency, group_name, tracking_mode, meta, is_default, is_active, is_archived, created_at, updated_at FROM accounts WHERE owner_id = ? AND id = ? LIMIT 1",
  )
    .bind(ownerId, id)
    .first();
}

export async function createAccount(input, ownerId, env, options = {}) {
  const normalized = normalizeAccount(input);
  const id = accountId(input);
  const statements = [];
  if (normalized.isDefault) {
    statements.push(env.DB.prepare("UPDATE accounts SET is_default = 0, updated_at = CURRENT_TIMESTAMP WHERE owner_id = ?").bind(ownerId));
  }
  statements.push(
    env.DB.prepare(
      "INSERT INTO accounts (id, owner_id, name, account_type, currency, group_name, tracking_mode, meta, is_default, is_active, is_archived) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    ).bind(
      id,
      ownerId,
      normalized.name,
      normalized.accountType,
      normalized.currency,
      normalized.group,
      normalized.trackingMode,
      normalized.meta,
      Number(normalized.isDefault),
      Number(normalized.isActive),
      Number(normalized.isArchived),
    ),
  );
  if (options.auditAction) {
    statements.push(
      env.DB.prepare("INSERT INTO mcp_audit_logs (id, owner_id, action, result_count) VALUES (?, ?, ?, 1)")
        .bind(crypto.randomUUID(), ownerId, options.auditAction),
    );
  }
  await env.DB.batch(statements);
  return mapAccount((await getAccount(ownerId, id, env)) ?? {
    id,
    ...normalized,
    group_name: normalized.group,
    account_type: normalized.accountType,
    tracking_mode: normalized.trackingMode,
    is_default: normalized.isDefault,
    is_active: normalized.isActive,
    is_archived: normalized.isArchived,
  });
}

export async function handleAccountRoute(request, route, ownerId, env) {
  const url = new URL(request.url);
  if (route === "/accounts" && request.method === "GET") {
    const includeArchived = url.searchParams.get("includeArchived") === "true";
    const result = await env.DB.prepare(
      `SELECT id, name, account_type, currency, group_name, tracking_mode, meta, is_default, is_active, is_archived, created_at, updated_at FROM accounts WHERE owner_id = ? ${includeArchived ? "" : "AND is_archived = 0"} ORDER BY is_default DESC, name COLLATE NOCASE`,
    )
      .bind(ownerId)
      .all();
    return json(result.results.map(mapAccount));
  }

  if (route === "/accounts" && request.method === "POST") {
    try {
      return json(await createAccount(await request.json(), ownerId, env), 201);
    } catch (error) {
      if (error instanceof TypeError || error instanceof SyntaxError) return json({ message: error.message }, 400);
      throw error;
    }
  }

  const match = route.match(/^\/accounts\/([^/]+)$/);
  if (!match) return null;
  const id = decodeURIComponent(match[1]);
  if (request.method === "DELETE") {
    await env.DB.prepare("UPDATE accounts SET is_active = 0, is_archived = 1, updated_at = CURRENT_TIMESTAMP WHERE owner_id = ? AND id = ?")
      .bind(ownerId, id)
      .run();
    return json({ success: true });
  }
  if (request.method === "PUT") {
    try {
      const normalized = normalizeAccount(await request.json());
      if (!(await getAccount(ownerId, id, env))) return json({ message: "Account not found." }, 404);
      const statements = [];
      if (normalized.isDefault) {
        statements.push(env.DB.prepare("UPDATE accounts SET is_default = 0, updated_at = CURRENT_TIMESTAMP WHERE owner_id = ?").bind(ownerId));
      }
      statements.push(
        env.DB.prepare(
          "UPDATE accounts SET name = ?, account_type = ?, currency = ?, group_name = ?, tracking_mode = ?, meta = ?, is_default = ?, is_active = ?, is_archived = ?, updated_at = CURRENT_TIMESTAMP WHERE owner_id = ? AND id = ?",
        ).bind(
          normalized.name,
          normalized.accountType,
          normalized.currency,
          normalized.group,
          normalized.trackingMode,
          normalized.meta,
          Number(normalized.isDefault),
          Number(normalized.isActive),
          Number(normalized.isArchived),
          ownerId,
          id,
        ),
      );
      await env.DB.batch(statements);
      const updated = await getAccount(ownerId, id, env);
      return updated ? json(mapAccount(updated)) : json({ message: "Account not found." }, 404);
    } catch (error) {
      if (error instanceof TypeError || error instanceof SyntaxError) return json({ message: error.message }, 400);
      throw error;
    }
  }
  return null;
}

export { normalizeAccount };
