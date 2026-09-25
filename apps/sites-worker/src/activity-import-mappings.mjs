function json(value, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}

function parseSettings(row) {
  try {
    const value = JSON.parse(row?.settings_json ?? "{}");
    return value && typeof value === "object" && !Array.isArray(value) ? value : {};
  } catch {
    return {};
  }
}

async function accountExists(env, ownerId, accountId) {
  const statement = env?.DB?.prepare?.("SELECT id FROM accounts WHERE owner_id = ? AND is_archived = 0 AND id = ? LIMIT 1");
  if (!statement?.bind) throw new Error("Database unavailable");
  return statement.bind(ownerId, accountId).first();
}

/** Handle GET/POST /activities/import/mapping. */
export async function handleActivityImportMappingRoute(request, route, ownerId, env) {
  if (route !== "/activities/import/mapping") return null;
  const url = new URL(request.url);
  if (request.method === "GET") {
    const accountId = text(url.searchParams.get("accountId"));
    const importType = text(url.searchParams.get("contextKind")) || "CSV_ACTIVITY";
    if (!accountId || !(await accountExists(env, ownerId, accountId))) {
      return json({ message: "Account is not available for this private Site." }, 404);
    }
    const row = await env.DB.prepare("SELECT settings_json FROM user_settings WHERE owner_id = ? LIMIT 1").bind(ownerId).first();
    const settings = parseSettings(row);
    const saved = settings.sitesImportMappings?.[`${accountId}:${importType}`];
    return json(saved ?? {
      accountId,
      importType,
      name: "",
      fieldMappings: {},
      activityMappings: {},
      symbolMappings: {},
      accountMappings: {},
      symbolMappingMeta: {},
    });
  }
  if (request.method === "POST") {
    let mapping;
    try {
      mapping = (await request.json())?.mapping;
    } catch {
      return json({ message: "Invalid import mapping." }, 400);
    }
    const accountId = text(mapping?.accountId);
    const importType = text(mapping?.importType) || "CSV_ACTIVITY";
    if (!accountId || !(await accountExists(env, ownerId, accountId))) {
      return json({ message: "Account is not available for this private Site." }, 404);
    }
    if (!mapping || typeof mapping !== "object" || Array.isArray(mapping)) {
      return json({ message: "Invalid import mapping." }, 400);
    }
    const statement = env.DB.prepare("SELECT settings_json FROM user_settings WHERE owner_id = ? LIMIT 1");
    const settings = parseSettings(await statement.bind(ownerId).first());
    const importMappings = { ...(settings.sitesImportMappings ?? {}) };
    const safeMapping = {
      ...mapping,
      accountId,
      importType,
      fieldMappings: mapping.fieldMappings && typeof mapping.fieldMappings === "object" ? mapping.fieldMappings : {},
      activityMappings: mapping.activityMappings && typeof mapping.activityMappings === "object" ? mapping.activityMappings : {},
      symbolMappings: mapping.symbolMappings && typeof mapping.symbolMappings === "object" ? mapping.symbolMappings : {},
      accountMappings: mapping.accountMappings && typeof mapping.accountMappings === "object" ? mapping.accountMappings : {},
    };
    importMappings[`${accountId}:${importType}`] = safeMapping;
    settings.sitesImportMappings = importMappings;
    await env.DB.prepare(
      "INSERT INTO user_settings (owner_id, settings_json, updated_at) VALUES (?, ?, CURRENT_TIMESTAMP) ON CONFLICT(owner_id) DO UPDATE SET settings_json = excluded.settings_json, updated_at = CURRENT_TIMESTAMP",
    ).bind(ownerId, JSON.stringify(settings)).run();
    return json(safeMapping);
  }
  return json({ message: "Method not allowed." }, 405);
}
