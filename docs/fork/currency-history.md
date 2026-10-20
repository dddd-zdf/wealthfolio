# Historical currency correctness (self-hosted web)

The web settings route uses `settings/fork_currency.rs`. A base-currency change
reuses the portfolio coordinator with `force_full: true`, every non-archived
account, and the existing FX history backfill. The coordinator invalidates from
genesis, replaces daily values and dated flow conversions, and retains its
normal per-account failure handling. Archived accounts remain outside reporting
and the engine's calculation scope.

The current SQLite settings triggers already invalidate `@all` from genesis
atomically with base-currency/timezone changes. The explicit full flag hardens
the web request contract; it is not evidence that the current trigger-backed
path left rows untouched. The confirmed race was settings/cache success arriving
before that rebuild completed.

Settings success waits for the rebuild. The work is detached from the HTTP
request so a browser disconnect does not cancel it. Simultaneous settings writes
are serialized per profile. While currency history changes, financial reads and
settings reads return HTTP 409 with a retry hint; a response started before the
transition is also rejected. A failed rebuild keeps financial reads unavailable;
fix the reported data issue and save the base currency again to retry.
Validation that rejects a preference before changing it keeps the old history
available.

At startup, pending policy markers, the web FX-policy revision and existing row
currencies are checked before serving financial reads. An upgrade from the old
FX policy, interrupted rebuild or partially failed rebuild uses the existing
startup job slot to finish a full rebuild. The web-only revision is saved in
ordinary settings only after success; an empty profile can record it
immediately. This reads stored history off the async workers once at startup; no
recovery table or additional scheduler is introduced.

The response cache is invalidated before and after rebuilding. Completion is
published only after the transition is readable. Web clients cancel and reset
cached financial queries before applying new settings, including intraday,
income, spending and planning queries. The existing completion event also
refreshes other connected tabs. A settings refetch detects a changed currency
and resets amounts before applying it even if that tab missed the event.

## FX policy

Historical conversion uses an observation on or before the valued date, carried
for at most **seven calendar days per conversion edge**. This preserves ordinary
weekend and holiday lookup without borrowing tomorrow's rate. Inverse and
triangulated paths follow the same rule. Missing, stale or nonpositive rates are
unavailable: the service returns a descriptive error and the engine uses its
existing unavailable-value and performance-quality diagnostics. It does not
substitute the latest rate. Latest/live service lookups retain their behavior.
An unresolvable transaction flow amount also makes gain/return calculations
unavailable; treating it as zero would mistake arriving capital for a gain. This
coverage status propagates into mixed holdings/transaction scopes. Paired
transfers with unknown acquisition FX keep their cost-based flow unavailable
instead of inventing a receiver flow at a later rate. A surviving leg retains
its recorded pairing metadata: a later rejection must not change earlier
persisted rows or make a windowed run disagree with a full rebuild.

The rule lives in two small child modules because the engine and the service
have separate FX surfaces. Preserve their matching seven-day bound on upstream
merges. Old snapshots that used future or unlimited carried rates necessarily
change; review their valuation dates, flow dates, return availability and
diagnostics when regenerating them.

## Intraday limits

No dated intraday FX source is configured. The web intraday handler therefore
returns no intraday points for a scope with any open nonzero foreign exposure
(including cash and manual assets), letting the dashboard use stored daily
values. Other intraday curves remain estimates using current position weights;
the visible 1D/1W notice states that within-session FX, trades and cash flows
are not reconstructed. Daily values and performance still use the engine.

## Architecture impact

Calculation ownership, valuation persistence, network providers, quote
scheduling and the coordinator queue are unchanged. Web settings response timing
and failure propagation change: success now means the rebuild finished, and
affected reads fail explicitly during the transition. The guard is web-only
because the coordinator's job lock serializes jobs but cannot protect HTTP reads
between persisted history windows. The web policy revision adds one internal
settings key, excluded by the existing sync allowlist. There is no schema change
or new provider. Shared FX resolution changes are required by web valuation
correctness; no native runtime or native adapter is modified or included in
acceptance checks.

The default frontend test job selects `BUILD_TARGET=web` and excludes native
adapter, native database-gate and cross-runtime command-parity suites. It no
longer runs the Tauri launcher wrapper. Rust acceptance excludes
`wealthfolio-app`, as before.
