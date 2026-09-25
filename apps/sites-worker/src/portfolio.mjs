/**
 * Read-only portfolio calculations for the Sites Worker.
 *
 * This module intentionally only uses the two rows supplied by the Sites
 * schema contract.  Account metadata, FX rates, lots, splits, dividends and
 * snapshots are therefore unavailable here; responses mark those limits in
 * `warnings` rather than presenting the result as full desktop parity.
 */

const JSON_HEADERS = { "content-type": "application/json; charset=utf-8" };
const TRADE_BUYS = new Set(["BUY", "COVER", "RECEIVE", "RECEIVED", "TRANSFER_IN"]);
const TRADE_SELLS = new Set(["SELL", "SHORT", "DISPOSE", "DELIVER", "TRANSFER_OUT"]);

function json(value, status = 200) {
  return new Response(JSON.stringify(value), { status, headers: JSON_HEADERS });
}

function errorResponse(status, message) {
  return json({ error: message }, status);
}

function text(value) {
  return typeof value === "string" ? value.trim() : value == null ? "" : String(value).trim();
}

function pow10(exponent) {
  return 10n ** BigInt(Math.max(0, exponent));
}

/** A small arbitrary-scale decimal used so quantities and prices never pass through Number. */
class Decimal {
  constructor(coefficient = 0n, scale = 0) {
    let value = BigInt(coefficient);
    let places = Math.max(0, Number(scale));
    while (places > 0 && value % 10n === 0n) {
      value /= 10n;
      places -= 1;
    }
    this.coefficient = value;
    this.scale = places;
  }

  static parse(input) {
    if (input instanceof Decimal) return input;
    let value = text(input);
    if (!value || !/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(value)) return null;
    const exponentIndex = value.search(/[eE]/);
    let exponent = 0;
    if (exponentIndex >= 0) {
      exponent = Number(value.slice(exponentIndex + 1));
      value = value.slice(0, exponentIndex);
    }
    const negative = value.startsWith("-");
    if (negative || value.startsWith("+")) value = value.slice(1);
    const [whole, fraction = ""] = value.split(".");
    let coefficient = BigInt(`${whole || "0"}${fraction}` || "0");
    let scale = fraction.length - exponent;
    if (scale < 0) {
      coefficient *= pow10(-scale);
      scale = 0;
    }
    return new Decimal(negative ? -coefficient : coefficient, scale);
  }

  add(other) {
    const right = Decimal.parse(other);
    if (!right) return this;
    const scale = Math.max(this.scale, right.scale);
    return new Decimal(
      this.coefficient * pow10(scale - this.scale) + right.coefficient * pow10(scale - right.scale),
      scale,
    );
  }

  sub(other) {
    const right = Decimal.parse(other);
    return right ? this.add(new Decimal(-right.coefficient, right.scale)) : this;
  }

  mul(other) {
    const right = Decimal.parse(other);
    return right ? new Decimal(this.coefficient * right.coefficient, this.scale + right.scale) : new Decimal(0n);
  }

  div(other, precision = 18) {
    const right = Decimal.parse(other);
    if (!right || right.coefficient === 0n) return null;
    const numerator = this.coefficient * pow10(right.scale + precision);
    const denominator = right.coefficient * pow10(this.scale);
    return new Decimal(numerator / denominator, precision);
  }

  abs() {
    return new Decimal(this.coefficient < 0n ? -this.coefficient : this.coefficient, this.scale);
  }

  isZero() {
    return this.coefficient === 0n;
  }

  toString() {
    const negative = this.coefficient < 0n;
    let digits = (negative ? -this.coefficient : this.coefficient).toString();
    if (this.scale === 0) return `${negative ? "-" : ""}${digits}`;
    if (digits.length <= this.scale) digits = digits.padStart(this.scale + 1, "0");
    const split = digits.length - this.scale;
    return `${negative ? "-" : ""}${digits.slice(0, split)}.${digits.slice(split)}`;
  }

  toNumber() {
    const value = Number(this.toString());
    return Number.isFinite(value) ? value : null;
  }
}

const ZERO = new Decimal(0n);

function decimal(value) {
  return Decimal.parse(value) ?? new Decimal(0n);
}

function number(value) {
  return decimal(value).toNumber() ?? 0;
}

function payload(row) {
  if (!row || typeof row.payload_json !== "string") return {};
  try {
    const parsed = JSON.parse(row.payload_json);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function rowField(row, key, ...aliases) {
  const values = [row?.[key], ...aliases.map((alias) => row?.[alias]), payload(row)?.[key], ...aliases.map((alias) => payload(row)?.[alias])];
  return values.find((value) => value !== undefined && value !== null && value !== "");
}

function dateOnly(value) {
  const date = text(value);
  return date.match(/^\d{4}-\d{2}-\d{2}/)?.[0] ?? date;
}

async function rowsFor(env, table, ownerId) {
  const columns = table === "activity_records"
    ? "account_id, activity_date, asset_id, symbol, activity_type, quantity, unit_price, currency, fee, tax, amount, payload_json"
    : "asset_id, symbol, quote_date, price, currency";
  const statement = env?.DB?.prepare?.(`SELECT ${columns} FROM ${table} WHERE owner_id = ?`);
  if (!statement?.bind) throw new Error("Database unavailable");
  const result = await statement.bind(ownerId).all();
  const rows = result?.results ?? result?.rows ?? [];
  // D1 enforces owner_id in the SQL predicate. The extra check keeps a simple
  // fixture/mock from accidentally making cross-owner rows visible in tests.
  return rows.filter((row) => row?.owner_id == null || String(row.owner_id) === String(ownerId));
}

function parseRequestFilter(request, body) {
  const url = new URL(request.url);
  const input = body && typeof body === "object" ? body : {};
  const filter = input.filter && typeof input.filter === "object" ? input.filter : {};
  const accountIds = [];
  for (const value of [input.accountId, filter.accountId]) if (text(value)) accountIds.push(text(value));
  for (const values of [input.accountIds, filter.accountIds]) {
    if (Array.isArray(values)) for (const value of values) if (text(value)) accountIds.push(text(value));
  }
  if (filter.type === "account" && text(filter.accountId)) accountIds.push(text(filter.accountId));
  return {
    accountIds: [...new Set(accountIds)],
    assetId: text(input.assetId || url.searchParams.get("assetId")),
    symbol: text(input.symbol || url.searchParams.get("symbol")),
    startDate: dateOnly(input.startDate || url.searchParams.get("startDate")),
    endDate: dateOnly(input.endDate || input.asOfDate || url.searchParams.get("endDate") || url.searchParams.get("asOfDate")),
    includeClosed: Boolean(input.includeClosed || url.searchParams.get("includeClosed") === "true"),
  };
}

function signedQuantity(row) {
  const raw = decimal(rowField(row, "quantity"));
  if (raw.isZero()) return ZERO;
  const type = text(rowField(row, "activity_type", "activityType")).toUpperCase();
  if (TRADE_SELLS.has(type)) return raw.abs().mul(-1);
  if (TRADE_BUYS.has(type)) return raw.abs();
  return raw;
}

function unitPrice(row, quantity) {
  const explicit = rowField(row, "unit_price", "unitPrice");
  if (explicit !== undefined) return decimal(explicit);
  const amount = decimal(rowField(row, "amount"));
  const absoluteQuantity = quantity.abs();
  return absoluteQuantity.isZero() ? ZERO : amount.abs().div(absoluteQuantity) ?? ZERO;
}

function filterRows(rows, filter) {
  return rows.filter((row) => {
    const accountId = text(row.account_id);
    const assetId = text(row.asset_id);
    const symbol = text(row.symbol).toUpperCase();
    if (filter.accountIds.length && !filter.accountIds.includes(accountId)) return false;
    if (filter.assetId && assetId !== filter.assetId) return false;
    if (filter.symbol && symbol !== filter.symbol.toUpperCase()) return false;
    const date = dateOnly(row.activity_date);
    if (filter.endDate && date > filter.endDate) return false;
    return true;
  });
}

function calculatePositions(rows) {
  const positions = new Map();
  const sorted = [...rows].sort((left, right) => text(left.activity_date).localeCompare(text(right.activity_date)));
  for (const row of sorted) {
    const quantity = signedQuantity(row);
    const assetId = text(rowField(row, "asset_id", "assetId"));
    const symbol = text(rowField(row, "symbol"));
    if (!assetId && !symbol || quantity.isZero()) continue;
    const accountId = text(rowField(row, "account_id", "accountId"));
    const key = `${accountId}\u001f${assetId || `symbol:${symbol.toUpperCase()}`}`;
    const current = positions.get(key) ?? {
      accountId,
      assetId: assetId || symbol,
      symbol,
      currency: text(rowField(row, "currency")) || "USD",
      quantity: ZERO,
      costBasis: ZERO,
      openDate: dateOnly(rowField(row, "activity_date", "activityDate")),
      lastActivityDate: dateOnly(rowField(row, "activity_date", "activityDate")),
    };
    const price = unitPrice(row, quantity);
    const fees = decimal(rowField(row, "fee")).abs().add(decimal(rowField(row, "tax")).abs());
    const type = text(rowField(row, "activity_type", "activityType")).toUpperCase();
    if (TRADE_SELLS.has(type) || quantity.coefficient < 0n) {
      const existingAverage = current.quantity.isZero() ? ZERO : current.costBasis.div(current.quantity.abs()) ?? ZERO;
      // Selling removes the average cost of the disposed units. The sale fee
      // belongs to realized P/L, which this reduced schema cannot calculate;
      // it must not reduce the cost basis of the units that remain.
      current.costBasis = current.costBasis.sub(existingAverage.mul(quantity.abs()));
    } else {
      current.costBasis = current.costBasis.add(quantity.abs().mul(price)).add(fees);
    }
    current.quantity = current.quantity.add(quantity);
    current.symbol = current.symbol || symbol;
    current.currency = current.currency || text(rowField(row, "currency"));
    current.lastActivityDate = dateOnly(rowField(row, "activity_date", "activityDate"));
    positions.set(key, current);
  }
  return [...positions.values()];
}

function latestQuotes(rows, endDate) {
  const quotes = new Map();
  for (const row of rows) {
    const date = dateOnly(row.quote_date);
    if (endDate && date > endDate) continue;
    const keys = [text(row.asset_id), text(row.symbol).toUpperCase()].filter(Boolean);
    for (const key of keys) {
      const previous = quotes.get(key);
      if (!previous || date >= previous.date) quotes.set(key, { date, price: decimal(row.price), currency: text(row.currency) });
    }
  }
  return quotes;
}

function quoteFor(position, quotes) {
  return quotes.get(position.assetId) ?? quotes.get(position.symbol.toUpperCase()) ?? null;
}

function positionView(position, quotes) {
  const quote = quoteFor(position, quotes);
  const value = quote ? position.quantity.mul(quote.price) : null;
  const gain = value ? value.sub(position.costBasis) : null;
  const gainPercent = gain && !position.costBasis.isZero() ? gain.div(position.costBasis.abs()) : gain?.isZero() ? ZERO : null;
  const id = `${position.accountId}:${position.assetId}`;
  const asOfDate = quote?.date || position.lastActivityDate || position.openDate || null;
  const monetary = (amount) => amount == null ? null : { local: amount.toNumber(), base: amount.toNumber() };
  return {
    id,
    accountId: position.accountId,
    holdingType: "security",
    isClosed: position.quantity.isZero(),
    instrument: {
      id: position.assetId,
      symbol: position.symbol || position.assetId,
      name: null,
      currency: quote?.currency || position.currency,
      quoteMode: "MARKET",
      exchangeMic: null,
      instrumentType: null,
      classifications: null,
    },
    assetKind: "INVESTMENT",
    quantity: position.quantity.toNumber(),
    openDate: position.openDate ? `${position.openDate}T00:00:00.000Z` : null,
    lots: [],
    contractMultiplier: 1,
    localCurrency: quote?.currency || position.currency,
    baseCurrency: quote?.currency || position.currency,
    fxRate: 1,
    marketValue: monetary(value || ZERO),
    costBasis: monetary(position.costBasis),
    price: quote?.price.toNumber() ?? null,
    unrealizedGain: monetary(gain),
    unrealizedGainPct: gainPercent?.toNumber() ?? null,
    realizedGain: null,
    realizedGainPct: null,
    totalGain: monetary(gain),
    totalGainPct: gainPercent?.toNumber() ?? null,
    income: null,
    totalReturn: monetary(gain),
    totalReturnPct: gainPercent?.toNumber() ?? null,
    returnBasis: monetary(position.costBasis),
    dayChange: null,
    dayChangePct: null,
    prevCloseValue: null,
    weight: 0,
    asOfDate,
    sourceAccountIds: [],
  };
}

function aggregatePositions(positions, quotes) {
  const grouped = new Map();
  for (const position of positions) {
    const key = position.assetId;
    const existing = grouped.get(key) ?? { ...position, accountId: "", quantity: ZERO, costBasis: ZERO, openDate: position.openDate };
    existing.quantity = existing.quantity.add(position.quantity);
    existing.costBasis = existing.costBasis.add(position.costBasis);
    existing.symbol = existing.symbol || position.symbol;
    existing.sourceAccountIds = [...new Set([...(existing.sourceAccountIds || []), position.accountId])];
    if (!existing.openDate || position.openDate < existing.openDate) existing.openDate = position.openDate;
    if (!existing.lastActivityDate || position.lastActivityDate > existing.lastActivityDate) {
      existing.lastActivityDate = position.lastActivityDate;
    }
    grouped.set(key, existing);
  }
  const views = [...grouped.values()].map((position) => {
    const view = positionView(position, quotes);
    view.id = position.assetId;
    view.sourceAccountIds = position.sourceAccountIds;
    return view;
  });
  const total = views.reduce((sum, view) => sum.add(decimal(view.marketValue?.base)), ZERO);
  return views.map((view) => ({ ...view, weight: total.isZero() ? 0 : decimal(view.marketValue.base).div(total)?.toNumber() ?? 0 }));
}

function portfolioSummary(positions, quotes, warnings = []) {
  const views = aggregatePositions(positions, quotes);
  const totalValueDecimal = views.reduce((sum, view) => sum.add(decimal(view.marketValue?.base)), ZERO);
  const costBasisDecimal = views.reduce((sum, view) => sum.add(decimal(view.costBasis?.base)), ZERO);
  const unrealizedGainDecimal = views.reduce((sum, view) => sum.add(decimal(view.unrealizedGain?.base)), ZERO);
  const totalValue = totalValueDecimal.toNumber() ?? 0;
  const costBasis = costBasisDecimal.toNumber() ?? 0;
  const unrealizedGain = unrealizedGainDecimal.toNumber() ?? 0;
  const currencies = [...new Set(views.map((view) => view.baseCurrency).filter(Boolean))];
  const currency = currencies[0] || "USD";
  const asOfDate = views.map((view) => view.asOfDate).filter(Boolean).sort().at(-1) || null;
  const allWarnings = [...warnings];
  if (currencies.length > 1) {
    allWarnings.push("Multiple currencies are present; no FX table is available, so totals are summed in local quote currencies.");
  }
  return {
    positions: views,
    holdings: views,
    totalValue,
    costBasis,
    unrealizedGain,
    currency,
    asOfDate,
    warnings: [...new Set(allWarnings)],
    summary: {
      scopeId: "owner",
      baseCurrency: currency,
      cashBalanceBase: 0,
      investmentMarketValueBase: totalValue,
      totalValueBase: totalValue,
      holdingsCount: views.filter((view) => !view.isClosed).length,
      accountCount: new Set(positions.map((position) => position.accountId).filter(Boolean)).size,
      currencySplit: currencies.map((code) => ({ currency: code, valueBase: totalValue, valueLocal: totalValue, percentage: totalValue ? 1 : 0 })),
      cashCurrencySplit: [],
      sourceDataAsOf: asOfDate,
      calculatedAt: new Date().toISOString(),
      warnings: [...new Set(allWarnings)],
    },
  };
}

function performanceResult(summary, filter, body, warnings) {
  const endDate = filter.endDate || summary.asOfDate;
  const amount = decimal(summary.unrealizedGain);
  const basis = decimal(summary.costBasis);
  const percent = basis.isZero() ? (amount.isZero() ? ZERO : null) : amount.div(basis.abs());
  const partial = warnings.length > 0;
  const quality = partial ? "partial" : "ok";
  const scopeId = text(body?.itemId) || (filter.accountIds.length ? `accounts:${filter.accountIds.join(",")}` : "owner");
  const currency = summary.currency || "USD";
  return {
    scope: { id: scopeId, currency },
    period: { startDate: filter.startDate || null, endDate: endDate || null },
    mode: "valueReturn",
    returns: { twr: null, annualizedTwr: null, irr: null, annualizedIrr: null, valueReturn: percent?.toNumber() ?? null, annualizedValueReturn: null },
    attribution: { contributions: 0, distributions: 0, income: 0, realizedPnl: 0, unrealizedPnlChange: amount.toNumber(), fxEffect: 0, fees: 0, taxes: 0, residual: 0 },
    risk: { volatility: null, maxDrawdown: null, peakDate: null, troughDate: null, recoveryDate: null, drawdownDurationDays: null },
    dataQuality: { status: quality, warnings: [...new Set(warnings)], notApplicableReasons: ["D1 worker has no FX, lots, snapshot, or account metadata tables."] },
    basisStatus: partial ? "partialUnknown" : "complete",
    summary: {
      amount: amount.toNumber(),
      percent: percent?.toNumber() ?? null,
      method: "valueReturn",
      basis: "bookBasis",
      quality,
      amountStatus: "complete",
      percentStatus: percent == null ? "unavailable" : "complete",
      basisStatus: partial ? "partialUnknown" : "complete",
      reasons: [...new Set(warnings)],
    },
    series: endDate ? [{ date: endDate, value: percent?.toNumber() ?? 0 }] : [],
    isHoldingsMode: false,
    isMixedTrackingMode: false,
  };
}

function routeKind(pathname) {
  const path = text(pathname).split("?")[0].replace(/\/$/, "") || "/";
  if (path === "/holdings/query") return "holdings";
  if (path === "/holdings/list/query") return "holdings-list";
  if (path === "/performance/summary") return "performance";
  if (path === "/sites/portfolio/summary") return "portfolio";
  return null;
}

/** Handle the read-only portfolio routes; return null when the dispatcher owns the route. */
export async function handlePortfolioRoute(request, pathname, ownerId, env) {
  const kind = routeKind(pathname);
  if (!kind) return null;
  if (request.method !== "GET" && request.method !== "POST") return errorResponse(405, "Method not allowed.");
  if (!text(ownerId)) return errorResponse(401, "Authenticated user is required.");
  let body = {};
  if (request.method === "POST") {
    try {
      body = (await request.clone().json()) || {};
    } catch {
      return errorResponse(400, "Invalid request body.");
    }
  }
  try {
    const filter = parseRequestFilter(request, body);
    if (kind === "performance") {
      if (text(body.itemType).toLowerCase() === "account" && text(body.itemId)) {
        filter.accountIds = [text(body.itemId)];
      } else if (text(body.itemType).toLowerCase() === "symbol" && text(body.itemId)) {
        filter.assetId = text(body.itemId);
      }
    }
    const [activityRows, quoteRows] = await Promise.all([
      rowsFor(env, "activity_records", ownerId),
      rowsFor(env, "asset_quotes", ownerId),
    ]);
    const activities = filterRows(activityRows, filter);
    const positions = calculatePositions(activities);
    const quotes = latestQuotes(quoteRows, filter.endDate);
    const warnings = [];
    const missingQuote = positions.some((position) => !quoteFor(position, quotes));
    if (missingQuote) warnings.push("Current value is unavailable for one or more positions because no owner-scoped quote was found.");
    if (kind === "holdings" || kind === "holdings-list") {
      const views = positions
        .filter((position) => filter.includeClosed || !position.quantity.isZero())
        .map((position) => positionView(position, quotes));
      const total = views.reduce((sum, view) => sum + (view.marketValue?.base ?? 0), 0);
      return json(views.map((view) => ({ ...view, weight: total ? view.marketValue.base / total : 0 })));
    }
    const summary = portfolioSummary(positions, quotes, warnings);
    if (kind === "portfolio") return json(summary);
    return json(performanceResult(summary, filter, body, summary.warnings));
  } catch {
    return errorResponse(500, "Unable to calculate portfolio data.");
  }
}

export { Decimal };
