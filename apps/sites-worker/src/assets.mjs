function json(value, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function text(value) {
  return typeof value === "string" ? value.trim() : "";
}

async function stableAssetId(input) {
  const symbol = text(input?.instrumentSymbol ?? input?.displayCode ?? input?.symbol).toUpperCase();
  const exchange = text(input?.instrumentExchangeMic ?? input?.exchangeMic).toUpperCase();
  const currency = text(input?.quoteCcy ?? input?.currency).toUpperCase();
  const value = `${symbol}\u001f${exchange}\u001f${currency}`;
  const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  const digest = [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  return `site-${digest.slice(0, 32)}`;
}

function assetDraft(candidate) {
  const symbol = text(candidate.symbol).toUpperCase();
  const currency = text(candidate.quoteCcy ?? candidate.currency).toUpperCase() || "USD";
  const exchangeMic = text(candidate.exchangeMic);
  const quoteMode = text(candidate.quoteMode).toUpperCase() || "MARKET";
  return {
    kind: "INVESTMENT",
    name: symbol,
    displayCode: symbol,
    isActive: true,
    quoteMode,
    quoteCcy: currency,
    instrumentType: text(candidate.instrumentType).toUpperCase() || "EQUITY",
    instrumentSymbol: symbol,
    ...(exchangeMic ? { instrumentExchangeMic: exchangeMic } : {}),
    ...(text(candidate.providerId) ? { providerId: text(candidate.providerId) } : {}),
    ...(text(candidate.providerSymbol) ? { providerSymbol: text(candidate.providerSymbol) } : {}),
  };
}

async function ownedAccount(env, ownerId, accountId) {
  const statement = env?.DB?.prepare?.("SELECT id FROM accounts WHERE owner_id = ? AND is_archived = 0 AND id = ? LIMIT 1");
  if (!statement?.bind) throw new Error("Database unavailable");
  return statement.bind(ownerId, accountId).first();
}

async function findExistingActivityAsset(env, ownerId, candidate) {
  const statement = env?.DB?.prepare?.(
    "SELECT asset_id, symbol, currency, payload_json FROM activity_records WHERE owner_id = ? AND UPPER(symbol) = ? ORDER BY activity_date DESC LIMIT 200",
  );
  if (!statement?.bind) throw new Error("Database unavailable");
  const result = await statement.bind(ownerId, text(candidate.symbol).toUpperCase()).all();
  const exchange = text(candidate.exchangeMic).toUpperCase();
  const currency = text(candidate.quoteCcy ?? candidate.currency).toUpperCase();
  return (result?.results ?? result?.rows ?? []).find((row) => {
    let payload = {};
    try { payload = JSON.parse(row.payload_json ?? "{}"); } catch { payload = {}; }
    const asset = payload.asset && typeof payload.asset === "object" ? payload.asset : {};
    const existingExchange = text(payload.exchangeMic ?? payload.instrumentExchangeMic ?? asset.instrumentExchangeMic ?? asset.exchangeMic).toUpperCase();
    const existingCurrency = text(payload.quoteCcy ?? asset.quoteCcy ?? row.currency).toUpperCase();
    return existingExchange === exchange && (!currency || existingCurrency === currency);
  }) ?? null;
}

async function previewImportAssets(request, ownerId, env) {
  let body;
  try {
    body = await request.json();
  } catch {
    return json({ message: "Invalid asset preview request." }, 400);
  }
  if (!Array.isArray(body?.candidates) || body.candidates.length > 500) {
    return json({ message: "Asset preview accepts at most 500 candidates." }, 400);
  }
  const result = [];
  for (const candidate of body.candidates) {
    const key = text(candidate?.key);
    const symbol = text(candidate?.symbol);
    const accountId = text(candidate?.accountId);
    if (!key || !symbol || !accountId) {
      result.push({
        key,
        status: "NEEDS_FIXING",
        resolutionSource: "sites-private-import",
        reviewSymbol: symbol || undefined,
        errors: { symbol: ["A symbol and account are required to resolve an asset."] },
      });
      continue;
    }
    if (!(await ownedAccount(env, ownerId, accountId))) {
      result.push({
        key,
        status: "NEEDS_FIXING",
        resolutionSource: "sites-private-import",
        reviewSymbol: symbol,
        errors: { accountId: ["Account is not available for this private Site."] },
      });
      continue;
    }
    const existing = await findExistingActivityAsset(env, ownerId, candidate);
    if (existing) {
      result.push({
        key,
        status: "EXISTING_ASSET",
        resolutionSource: "sites-owner-activity-history",
        reviewSymbol: existing.symbol || symbol,
        assetId: existing.asset_id || await stableAssetId(candidate),
      });
      continue;
    }
    result.push({
      key,
      status: "AUTO_RESOLVED_NEW_ASSET",
      resolutionSource: "sites-private-import",
      reviewSymbol: symbol,
      draft: assetDraft(candidate),
    });
  }
  return json(result);
}

/** Handle Sites asset routes needed by transaction CSV import. */
export async function handleAssetRoute(request, route, ownerId, env) {
  if (route === "/activities/import/assets/preview") {
    if (request.method !== "POST") return json({ message: "Method not allowed." }, 405);
    return previewImportAssets(request, ownerId, env);
  }
  if (route !== "/assets") return null;
  if (request.method !== "POST") return json({ message: "Method not allowed." }, 405);
  let input;
  try {
    input = await request.json();
  } catch {
    return json({ message: "Invalid asset." }, 400);
  }
  const symbol = text(input?.instrumentSymbol ?? input?.displayCode);
  if (!symbol) return json({ message: "An asset symbol is required." }, 400);
  const id = await stableAssetId(input);
  const now = new Date().toISOString();
  // Asset metadata is carried with imported activity rows. The Sites D1 port
  // does not have the desktop assets/profile table, so the ID is stable from
  // the instrument identity and no separate shadow record is created here.
  return json({ ...input, id, createdAt: now, updatedAt: now }, 201);
}
