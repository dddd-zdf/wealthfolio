# Fork extension map

Keep upstream synchronization as a merge from a pinned release tag. The fork's
public tool contracts and user-visible behavior matter more than preserving old
implementation code. Use `git diff <common-base>..origin/main` to inventory
those contracts before resolving conflicts.

## Custom behavior kept apart

| Behavior                                            | Owning code                                                                          | Upstream integration point                                                                                                                      |
| --------------------------------------------------- | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Adaptive server refresh and quiet background events | `apps/server/src/fork_policy.rs`                                                     | The upstream coordinator remains the calculation path; the scheduler starts one fork cadence loop and the domain-event queue uses the observer. |
| Dashboard response caching and invalidation         | `apps/server/src/perf_cache.rs`                                                      | Performance/holdings handlers use generation-aware entries. The observer invalidates on completion and warms only after edits.                  |
| Intraday chart reconstruction                       | `crates/core/src/portfolio/valuation/intraday.rs` and frontend history-chart helpers | The handler anchors intraday quotes to engine daily valuations. Both request and query cache keys must carry the selected account scope.        |
| Term deposits                                       | `crates/core/src/assets/term_deposit.rs`                                             | Quote sync emits calculated daily prices; the engine reads those prices normally. Keep one valuation engine.                                    |
| Fork-only MCP commits                               | `crates/agent-tools/src/tools/fork_commits.rs`                                       | One extension to the commit-tool list registers the existing category, account and data-admin tools.                                            |
| Transfer payload compatibility                      | `crates/agent-tools/src/tools/transfer_compat.rs`                                    | One catalog entry delegates batch input to the unchanged upstream implementation and retains flat input for older callers.                      |
| Hosted MCP OAuth                                    | `apps/server/src/mcp/oauth.rs` and frontend agent-access consent                     | Preserve discovery, PKCE, consent, PAT creation/revocation and the protocol fallback.                                                           |
| Manual account/holding value editing                | Frontend `managed-value-edit`, `managed-account`, and asset total-value dialog       | Reuse ordinary quote mutation paths; never create a separate balance store.                                                                     |

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
- `categorization:write` and `accounts:write` are persisted fork scope strings.
  Preserve them alongside new upstream scopes. Do not widen a preset simply
  because upstream introduces a new write scope.
- The transfer compatibility tool keeps the legacy write grant for flat calls;
  batched calls retain upstream's read/draft/write requirements. Discovery uses
  the minimum contract, and execution checks the requirements of the input form.
- Direct categorization-rule writes keep `commit_categorization_rule`;
  upstream's reviewed draft contract is registered as
  `commit_reviewed_categorization_rule`. Avoid silently reusing one name for
  incompatible schemas.
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
3. Verify unique catalog names, existing payloads and saved scope grants.
   Exercise legacy and new transfer forms, direct/reviewed rules, OAuth and PAT
   access.
4. Test saved portfolio scope in daily and intraday charts, manual values, GICs,
   quiet background updates, cache invalidation and the engine regressions
   above.
5. Run repository checks and validate first startup on a disposable database
   copy. Compare source records and spending totals; investigate changed derived
   values.
6. Preserve merge ancestry when promoting the upstream integration. Record the
   deployed image revision and verified pre-upgrade backup in the infra
   changelog.
