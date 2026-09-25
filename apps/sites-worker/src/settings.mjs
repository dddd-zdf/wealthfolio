const DEFAULT_SETTINGS = Object.freeze({
  theme: "light",
  font: "font-mono",
  language: "en",
  formattingRegion: "system",
  baseCurrency: "USD",
  timezone: "",
  onboardingCompleted: true,
  autoUpdateCheckEnabled: false,
  menuBarVisible: true,
  syncEnabled: false,
  restoreReconnectRequired: false,
  defaultReturnMetric: "twr",
  insightsOverviewLayout: null,
});

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

async function getSettings(ownerId, env) {
  const row = await env.DB.prepare("SELECT settings_json FROM user_settings WHERE owner_id = ? LIMIT 1").bind(ownerId).first();
  if (!row?.settings_json) return { ...DEFAULT_SETTINGS };
  try {
    const parsed = JSON.parse(row.settings_json);
    const publicSettings = { ...parsed };
    delete publicSettings.sitesImportMappings;
    return { ...DEFAULT_SETTINGS, ...publicSettings, syncEnabled: false };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export async function handleSettingsRoute(request, route, ownerId, env) {
  if (route === "/settings" && request.method === "GET") {
    return json(await getSettings(ownerId, env));
  }
  if (route === "/settings" && request.method === "PUT") {
    let update;
    try {
      update = await request.json();
    } catch {
      return json({ message: "Invalid settings update." }, 400);
    }
    if (!update || typeof update !== "object" || Array.isArray(update)) {
      return json({ message: "Invalid settings update." }, 400);
    }
    const allowed = new Set(Object.keys(DEFAULT_SETTINGS).filter((key) => key !== "syncEnabled" && key !== "restoreReconnectRequired"));
    const safeUpdate = Object.fromEntries(Object.entries(update).filter(([key]) => allowed.has(key)));
    const current = await env.DB.prepare("SELECT settings_json FROM user_settings WHERE owner_id = ? LIMIT 1").bind(ownerId).first();
    let privateSettings = {};
    try {
      privateSettings = JSON.parse(current?.settings_json ?? "{}");
    } catch {
      privateSettings = {};
    }
    const settings = {
      ...(await getSettings(ownerId, env)),
      ...safeUpdate,
      ...(privateSettings.sitesImportMappings ? { sitesImportMappings: privateSettings.sitesImportMappings } : {}),
      syncEnabled: false,
      restoreReconnectRequired: false,
    };
    await env.DB.prepare(
      "INSERT INTO user_settings (owner_id, settings_json, updated_at) VALUES (?, ?, CURRENT_TIMESTAMP) ON CONFLICT(owner_id) DO UPDATE SET settings_json = excluded.settings_json, updated_at = CURRENT_TIMESTAMP",
    )
      .bind(ownerId, JSON.stringify(settings))
      .run();
    return json(settings);
  }
  if (route === "/settings/auto-update-enabled" && request.method === "GET") {
    return json(false);
  }
  return null;
}
