# Shared Yahoo quotes for agents

The fork's `get_market_quote` MCP tool shares Wealthfolio's Yahoo provider. It
is hosted by the existing server and uses its PAT/OAuth authentication and
`holdings:read` scope. There is no separate quote server, database mount,
credential, or always-on PC process.

## Read path

1. Resolve the requested Yahoo symbol against stored asset identities using the
   existing provider resolver. Match the exact listing, including exchange
   suffixes and explicit provider overrides.
2. Return the latest persisted Yahoo quote for that listing if one exists. This
   read does not refresh the portfolio or make an upstream request.
3. Otherwise consult the process-wide Yahoo quote cache. On a cache miss, use
   the configured Yahoo provider, without falling back to a different source.
4. If a refresh fails or the agent allowance is exhausted, return an available
   older cache entry with `cacheStatus: "stale"`. If no cached quote exists,
   return an error.

The tool accepts a single raw Yahoo symbol, such as `AAPL`, `VEQT.TO`,
`BTC-CAD`, `EURUSD=X`, or `^GSPC`. It never creates an asset, holding,
transaction, or stored quote. There is no caller-controlled force-refresh flag.
Normal background sync continues to own portfolio quote persistence and
valuation updates.

Exchange identity is part of the request: `FINN.NE` is the Fidelity ETF on Cboe
Canada (`NEOE`, CAD); bare `FINN` addresses a different US instrument. Likewise
`SHOP.TO` and `SHOP` are different listings. Reuse the exchange registry and
explicit provider overrides; never guess a venue from account currency or
rewrite the currency on a returned quote to match an asset's metadata.

`quoteTime` is the provider's quote/bar timestamp. `fetchedAt` is the shared
cache fetch time, or null for persisted quotes whose last fetch time is not
known. `servedAt` is the tool response time. A recent fetch does not establish a
recent trade, and persisted quotes can be old; inspect `quoteTime` and
`quoteAgeSeconds`. Prices are decimal strings; currency units follow Yahoo.

## Traffic protection

Yahoo request protection is shared across provider instances and profiles in one
server process, including normal sync, searches, profile requests, and agent
lookups. Provider reconstruction does not reset it. The policy spaces upstream
requests, serializes dispatch, and stops new requests during a rate-limit
cooldown. A throttled primary request must not trigger a backup request.

Requests are serialized with at least two seconds between starts. Queues are
bounded (32 pending requests, a 30-second wait); direct HTTP requests time out
after 30 seconds. A Yahoo 429 pauses requests for at least 15 minutes, extended
by a `Retry-After` hint (seconds or HTTP date, capped at 24 hours).

The latest-quote cache is bounded and lives only in memory. Simultaneous misses
for the same symbol share a fetch. Current-range historical sync can populate
the same cache; old historical ranges must not overwrite a current quote.

Agents have an additional allowance of 30 uncached lookup attempts per rolling
hour. Cached reads and persisted reads consume none of that allowance.
Concurrent misses may conservatively consume more than one allowance even when
the provider combines their requests. This bounds additional agent demand
without consuming the background sync's allowance.

These controls apply to this process, not other programs or replicas using the
same public IP. Restarts clear memory state. Do not add a second Yahoo-fetching
service or scale replicas without coordinating that traffic. The chosen limits
are our conservative policy, not a published Yahoo entitlement or a guarantee
against throttling.

## Upstream merge checklist

Keep fork behavior in the following modules, with small hooks in upstream files:

- `crates/market-data/src/provider/yahoo/fork_request_policy.rs`: shared request
  policy and latest-quote cache; Yahoo provider hooks enforce it.
- `crates/market-data/src/provider/yahoo/fork_chart_transport.rs`: explicit
  chart transport using the dependency's response parser. The library's chart
  convenience methods retry internally, so bypassing them keeps every request
  under the shared policy. Preserve history intervals and corporate-action query
  parameters when updating this adapter.
- `crates/core/src/quotes/fork_market_quotes.rs`: stored-first lookup, agent
  allowance, and response contract. `QuoteServiceTrait` and `MarketDataClient`
  provide thin integration hooks.
- `crates/agent-tools/src/tools/fork_market_quotes.rs`: MCP input validation and
  read scope; register it only in `AgentToolCatalog::mcp_catalog`.

On upstream merges:

1. Audit every new Yahoo connector call and HTTP request: it must use the shared
   policy, including fallback and cookie/crumb requests.
2. Preserve the adaptive background cadence in `apps/server/src/fork_policy.rs`.
   Do not introduce another agent polling loop or per-request portfolio sync.
3. Preserve exact symbol resolution, currency units, quote timestamps, and the
   distinction between quote, fetch, and response time.
4. Keep arbitrary market lookups out of portfolio persistence. Stored-first
   reads must not request extra Yahoo traffic.
5. Run the policy/cache, core market-quote, and MCP authorization regressions,
   then the normal Rust consumer checks. Verify cache hits and throttling with
   fixtures rather than intentionally flooding Yahoo.
6. Recheck live MCP discovery and a small quote sample after deployment.

No schema migration, background worker, frontend adapter, or additional MCP
authentication flow is needed for this extension.
