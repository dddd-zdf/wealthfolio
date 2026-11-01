# Fork extension map

Keep upstream synchronization as a merge from a pinned release tag. The fork's
public tool contracts and user-visible behavior matter more than preserving old
implementation code. Use `git diff <common-base>..origin/main` to inventory
those contracts before resolving conflicts.

## How fork code attaches to upstream

Upstream files carry thin hooks only; the fork's logic and its self-contained
tests live in `fork_*` files. A hook is a `mod` line, a `use` line, a call to a
fork function, a route or registration entry, or a short branch that calls fork
code. This keeps upstream merges to small, obvious conflicts.

- **Rust.** Put fork logic in a `fork_<topic>.rs` module. When it needs an
  upstream module's private items, make it a child module (for example
  `api/performance/fork_cache.rs` declared from `api/performance.rs`) instead of
  widening upstream visibility. Add fork imports as separate `use` lines so
  rustfmt does not reflow upstream import lists.
- **Wrap, don't edit, upstream handlers.** A fork handler calls the unchanged
  upstream handler (they are plain async functions); only the route line points
  at the fork handler.
- **Replace by name in one place.** MCP tools are adapted in
  `crates/agent-tools/src/tools/fork/mod.rs`, called once from
  `AgentToolCatalog::mcp_catalog`. Upstream tool files stay unchanged.
- **Frontend.** Put fork state and data fetching in a hook file (`fork-*.ts`)
  and fork-only markup in a component file (`fork-*.tsx`); the upstream
  component calls the hook and renders the props it returns. Strings used only
  by fork screens go in the `fork` translation namespace
  (`apps/frontend/src/i18n/locales/*/fork.json`). Keys that upstream code looks
  up by pattern, such as health issue codes and scope labels, stay in upstream's
  namespace.
- **Tests.** Self-contained tests move with the fork module. Tests that need an
  upstream test module's private fixtures go in a child of that test module,
  declared with one `mod fork;` line inside it (for example
  `agent-tools/src/catalog/tests/fork.rs`, or `#[path]` from an integration test
  file as in `apps/server/tests/agent_access/fork.rs`), so a fixture change
  upstream is still compiled against them. A few lines inside an upstream test
  may stay in place. Frontend component tests that rely on the upstream file's
  mocks stay in that file.
- **Additions.** New methods on an upstream trait or `impl` (for example
  intraday fetching in `market-data`) may stay in place as one block, apart from
  upstream code. Edits inside an upstream function body become hooks.
- **Tauri/mobile** are not deployed; their fork additions are left as they are.

To see the fork's footprint in upstream-owned files (renames and new files
excluded):

```bash
git diff --numstat --diff-filter=M "$(git merge-base origin/main upstream/main)" origin/main -- apps crates packages | sort -rn | head -30
```

## Custom behavior kept apart

| Behavior                                            | Owning code                                                                                                                                                                                                                      | Upstream integration point                                                                                                                                                                                                               |
| --------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Adaptive server refresh and quiet background events | `apps/server/src/fork_policy.rs`                                                                                                                                                                                                 | The upstream coordinator remains the calculation path; the scheduler starts one fork cadence loop. `api/shared.rs` re-exports the fork's `ServerJobObserver`, which keeps the performance cache when a background sync changed no close. |
| Dashboard response caching and invalidation         | `apps/server/src/perf_cache.rs`, `api/performance/fork_cache.rs`, `api/holdings/handlers/fork_intraday.rs`                                                                                                                       | Routes point at fork wrappers that call the unchanged upstream handlers. The observer invalidates on completion and warms only after edits.                                                                                              |
| Stale-while-refresh health status                   | `apps/server/src/api/health/fork_refresh.rs`                                                                                                                                                                                     | `/health/status` route; falls through to the upstream handler when nothing is cached.                                                                                                                                                    |
| Intraday chart reconstruction                       | `crates/core/src/portfolio/valuation/intraday.rs`, `quotes/fork_intraday.rs`, frontend `fork-dashboard.ts`; asset page `api/market_data/fork_intraday.rs`, `pages/asset/fork-asset-intraday.ts`, `fork-intraday-price-chart.tsx` | The handler anchors intraday quotes to engine daily valuations. Both request and query cache keys must carry the selected account scope.                                                                                                 |
| Mixed-scope return % skips dormant accounts         | `crates/portfolio-engine/src/measure/fork_mixed.rs`                                                                                                                                                                              | One match arm in `build_mixed_result`. Upstream golden `EDGE-SPLIT-03` reflects it (200%, not unavailable).                                                                                                                              |
| Quote-sync policies                                 | `crates/core/src/quotes/fork_sync.rs`                                                                                                                                                                                            | Hooks in `sync.rs` (backfill window, deposit pricing, changed-quote count) and one in `sync_state.rs` (backfill start for sold positions).                                                                                               |
| Term deposits                                       | `crates/core/src/assets/term_deposit.rs`, `quotes/fork_sync.rs`                                                                                                                                                                  | Quote sync emits calculated daily prices; the engine reads those prices normally. Keep one valuation engine.                                                                                                                             |
| Fork-only MCP tools                                 | `crates/agent-tools/src/tools/fork/`                                                                                                                                                                                             | `fork::mcp_tools` adapts upstream's list once: renames the reviewed rule commit, swaps in transfer compatibility, and adds the fork's commit tools.                                                                                      |
| Transfer payload compatibility                      | `crates/agent-tools/src/tools/fork/transfer_compat.rs`                                                                                                                                                                           | Replaces `link_transfer_activities` by name; delegates batch input to the unchanged upstream implementation and retains flat input for older callers.                                                                                    |
| Activities awaiting review (health)                 | `crates/core/src/health/checks/data_consistency/fork_needs_review.rs`                                                                                                                                                            | One issue type, one collector call in `HealthService`, one builder call in `DataConsistencyCheck::analyze`.                                                                                                                              |
| Negative-balance precision filter                   | `crates/storage-sqlite/src/portfolio/valuation/repository/fork_negative_balance.rs`                                                                                                                                              | The repository query selects candidate rows in date order and keeps each account's first meaningful deficit.                                                                                                                             |
| Date-only activity storage at noon UTC              | `crates/storage-sqlite/src/activities/fork_dates.rs`                                                                                                                                                                             | The three `From<...> for ActivityDB` conversions call it for bare dates.                                                                                                                                                                 |
| Hosted MCP OAuth                                    | `apps/server/src/mcp/oauth.rs` and frontend agent-access consent                                                                                                                                                                 | Preserve discovery, PKCE, consent, PAT creation/revocation and the protocol fallback.                                                                                                                                                    |
| MCP client compatibility                            | `apps/server/src/mcp/fork_compat.rs`                                                                                                                                                                                             | Two `.layer` lines in `mcp::router`: 422 → 400 for 2026-07-28 clients, and `WWW-Authenticate` resource metadata on 401s. Upstream `mcp/auth.rs` is unchanged.                                                                            |
| Default profile started at boot; detached startup   | `apps/server/src/profiles/fork_startup.rs`, `apps/server/src/api/fork_router.rs`                                                                                                                                                 | `main` serves `api::fork_router::serving_router_from_config`; `command` calls `runtime_detached` instead of `runtime`.                                                                                                                   |
| Manual account/holding value editing                | Frontend `managed-value-edit`, `managed-account`, `fork-accounts-summary.ts`, asset total-value dialog                                                                                                                           | Reuse ordinary quote mutation paths; never create a separate balance store. Accounts holding a Wealthsimple-synced `WSE*` fund get no pencil.                                                                                            |
| Dashboard 1D gain, MWR and period-start guide       | `apps/frontend/src/pages/dashboard/fork-dashboard.ts`, `fork-accounts-summary.ts`, `fork-dashboard-parts.tsx`, `lib/fork-performance.ts`                                                                                         | `DashboardContent` and `AccountsSummary` call the hooks and render what they return.                                                                                                                                                     |
| Quiet background events in the UI                   | `apps/frontend/src/lib/fork-background-events.ts`                                                                                                                                                                                | `use-global-event-listener.ts` skips toasts for `background: true` events; the web build skips the start-up and resume portfolio updates.                                                                                                |
| Manual holding total value (quote grid)             | `apps/frontend/src/pages/asset/fork-total-value.tsx`, `update-total-value-dialog.tsx`                                                                                                                                            | `QuoteHistoryDataGrid` calls `useTotalValueEntry` and renders the toolbar action and dialog it returns.                                                                                                                                  |
| Spending monthly average                            | `apps/frontend/src/features/spending/components/fork-spending-total.tsx`                                                                                                                                                         | `SpendingTabContent` renders `SpendingTotal` in place of `Balance`.                                                                                                                                                                      |
| Review count on the Activities nav item             | `apps/frontend/src/pages/layouts/navigation/fork-nav-badge.tsx`                                                                                                                                                                  | `useNavigation` wraps its primary items with `useNeedsReviewBadge`; the sidebar renders `NavIcon`.                                                                                                                                       |
| Calendar-year quick picks                           | `packages/ui/src/components/common/fork-year-presets.tsx`                                                                                                                                                                        | `DateRangeSelector` renders `YearPresets` when `yearPresets` is set (the performance page passes 5). Its label is in the `fork` namespace.                                                                                               |

## Historical currency corrections

The web settings wrapper and read guard live in
`apps/server/src/api/settings/fork_currency.rs`; browser cache reset is in
`apps/frontend/src/lib/fork-currency-cache.ts`. Dated service FX selection lives
in `fx/currency_converter/fork_history.rs` and the engine selector in
`portfolio-engine/src/resolve/fork_fx.rs`. Preserve their matching carry bound
and the engine's suppression of return series when endpoint values are
unavailable. See [currency-history.md](currency-history.md) for semantics and
merge checks.

## Deliberate changes inside upstream code

Some behavior belongs at the calculation or authorization boundary and cannot be
isolated by moving the caller:

- The portfolio engine attributes non-BONUS credits (cash back and rebates) as
  income. BONUS remains contributed capital. The focused compile regression
  records that distinction.
- The dashboard measure profile computes period and annualized IRR for MWR
  display, while still skipping full risk/attribution work. Keep the measure
  regression when adopting a newer engine.
- User-entered bare dates are normalized to explicit timestamps by the activity
  service. This preserves upstream timezone handling without invoking the fork's
  noon-UTC storage fallback, which remains for raw broker dates. Keep both the
  upstream MCP date regression and the storage fallback test.
- A manual (non-market) price not updated today reports no day change
  (`HoldingsValuationService::calculate_security_valuation`); market quotes
  still compare the last two sessions. Its tests are in
  `holdings_valuation_service_tests/tests/fork.rs`, using the upstream mocks and
  the test-only `with_today` override.
- `categorization:write` and `accounts:write` are persisted fork scope strings.
  Preserve them alongside new upstream scopes. Do not widen a preset simply
  because upstream introduces a new write scope.
- The transfer compatibility tool keeps the legacy write grant for flat calls;
  batched calls retain upstream's read/draft/write requirements. Discovery uses
  the minimum contract, and execution checks the requirements of the input form.
- Direct categorization-rule writes keep `commit_categorization_rule`
  (`tools/fork/direct_categorization_rule.rs`); upstream's reviewed draft
  contract, unchanged in `tools/commit_categorization_rule.rs`, is registered as
  `commit_reviewed_categorization_rule`. Upstream's catalog test and the
  `create_categorization_rule` description name it that way. Avoid silently
  reusing one name for incompatible schemas.
- `api/shared.rs` replaces upstream's `ServerJobObserver` and periodic loop with
  re-exports from `fork_policy.rs`. An upstream change to the observer therefore
  conflicts on purpose: port it into `fork_policy.rs`.
- MCP `tools/list` shows only the check and fix tools named in
  `crates/agent-tools/src/fork_mcp_listing.rs`, to keep clients' context small;
  every other tool stays registered and callable by name. A new upstream MCP
  tool stays unlisted until it is added there.

The old snapshot calculators and their lot-cleanup/backfill patches were removed
with 3.10. Recheck the original failure against the new engine before restoring
any old workaround.

## Before accepting another upstream merge

1. Read the current `dddd-zdf/infra` host inventory before server work. It owns
   deployment locations, service ownership, backup and restore procedures.
2. Preserve the upstream engine/coordinator. Port behaviors through these seams;
   keep conflicts in upstream files small and explain unavoidable differences.
3. Compare the hooks that mirror or rely on upstream internals, which compile
   even when upstream's behavior changes: `fork_sync::is_recently_closed` copies
   the recently-closed rule in `sync_state::determine_sync_category`, and
   `health/fork_refresh.rs` relies on upstream's `get_health_status` checking
   the cache before running checks.
4. Verify unique catalog names, existing payloads and saved scope grants.
   Exercise legacy and new transfer forms, direct/reviewed rules, OAuth and PAT
   access.
5. Test saved portfolio scope in daily and intraday charts, manual values, GICs,
   quiet background updates, cache invalidation and the engine regressions
   above.
6. Run repository checks and validate first startup on a disposable database
   copy. Compare source records and spending totals; investigate changed derived
   values.
7. Preserve merge ancestry when promoting the upstream integration. Record the
   deployed image revision and verified pre-upgrade backup in the infra
   changelog.
