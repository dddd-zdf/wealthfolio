# Fork handoff guide

This repository is a personal fork of
[wealthfolio/wealthfolio](https://github.com/wealthfolio/wealthfolio), run as a
single-user web server for one household. This guide is for an agent (or person)
taking over: what the fork changes, how it is built and deployed, and how to
operate it. Read [AGENTS.md](../../AGENTS.md) first for general repo rules.

Secrets, IP addresses, account ids and notes about the owner's personal data are
deliberately **not** in this public repo. They live in the private Agent
Workspace on Google Drive (`projects/wealthfolio/README.md`), which the owner
can share with you.

The authoritative host inventory and operating runbooks live in the private
`dddd-zdf/infra` repository (local checkout `C:\Users\difei\Developer\infra`).
Read its `README.md`, `vms/home.md`, and `vms/util.md` before server work; do
not infer the current deployment from older application notes.

## At a glance

| What         | Where                                                                         |
| ------------ | ----------------------------------------------------------------------------- |
| Fork         | `github.com/dddd-zdf/wealthfolio` (remote `origin`); upstream is `upstream`   |
| Live app     | `https://wealth.dfhome.dpdns.org` (web build, password login)                 |
| MCP endpoint | `https://wealth.dfhome.dpdns.org/mcp` (PAT or OAuth)                          |
| Host         | `home` A1 VM (arm64, Ubuntu 24.04), Toronto; shared with Immich and Nextcloud |
| Images       | `ghcr.io/dddd-zdf/wealthfolio:mcp-<7-char sha>`, built by `docker-branch.yml` |
| Branching    | Feature branch → PR into `main` → build exact revision → deploy               |

## What the fork changes

All fork work is in PRs on `dddd-zdf/wealthfolio`. Notable changes, newest
first:

| PR     | Change                                                                                           | Main files                                                                                                                           |
| ------ | ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| #49    | No account-total pencil on accounts holding WSE funds (the Wealthsimple sync prices them)        | `apps/frontend/src/lib/managed-account.ts`                                                                                           |
| #47    | Asset page 1D/1W price chart from live intraday bars, with the previous close marked             | `apps/frontend/src/pages/asset/fork-{asset-intraday,intraday-price-chart}.ts(x)`, `apps/server/src/api/market_data/fork_intraday.rs` |
| #42    | Background refresh keeps the dashboard cache when no price changed (3.10 lost #10/#14's gate)    | `apps/server/src/fork_policy.rs`                                                                                                     |
| #41    | Dormant/zero-balance accounts no longer hide the combined return % (lost in 3.10's engine)       | `crates/portfolio-engine/src/measure/fork_mixed.rs`                                                                                  |
| #40    | MCP `recalculate_portfolio` and manual quote writes queue a recalc again (3.10 dropped the sink) | `apps/server/src/main_lib.rs`                                                                                                        |
| #39    | More fork code moved behind hooks (upstream footprint 2,313 → ~1,445 lines)                      | `docs/fork/EXTENSIONS.md`                                                                                                            |
| #36    | Fork code moved behind thin hooks into `fork_*` modules (no behaviour change); see EXTENSIONS.md | `docs/fork/EXTENSIONS.md`, `crates/agent-tools/src/tools/fork/mod.rs`, `crates/core/src/quotes/fork_sync.rs`                         |
| #33    | MCP `tools/list` shows only 12 check/fix tools; importers call the rest by name                  | `crates/agent-tools/src/fork_mcp_listing.rs`, `crates/wealthfolio-mcp/src/handler.rs`                                                |
| #32    | Reverted in #34 (MCP Yahoo quote tool and request pacer slowed the 1D/1W chart)                  | —                                                                                                                                    |
| #27    | 1D/1W intraday curve cached in `perf_cache`, so portfolio updates clear it                       | `apps/server/src/api/holdings/handlers/fork_intraday.rs`, `apps/server/src/perf_cache.rs`                                            |
| #26    | No "skipping orphan cleanup" warning for fully sold accounts (count open lots only)              | `crates/storage-sqlite/src/lots.rs`                                                                                                  |
| #25    | GICs/term deposits valued from their terms (daily accrual); MCP `set_term_deposit`               | `crates/core/src/assets/term_deposit.rs`, `crates/core/src/quotes/fork_sync.rs`, `crates/agent-tools/src/tools/fork/data_admin.rs`   |
| #24    | Pencil on a managed account's total (dashboard + account page) sets manual fund prices           | `apps/frontend/src/components/managed-value-edit.tsx`, `apps/frontend/src/lib/managed-account.ts`                                    |
| #23    | Faint start-of-period line on the dashboard chart for 1W–5Y                                      | `apps/frontend/src/pages/dashboard/fork-dashboard.ts`                                                                                |
| #21    | 1D chart zooms to the day's move and marks the previous close                                    | `apps/frontend/src/components/history-chart*.ts(x)`                                                                                  |
| #20    | Zero day change for manual prices not updated today                                              | `crates/core/src/portfolio/holdings/holdings_valuation_service.rs`                                                                   |
| #19    | MCP: clients speaking protocol 2026-07-28 (ChatGPT) fall back to `initialize` (422 → 400)        | `apps/server/src/mcp/fork_compat.rs`                                                                                                 |
| #18    | Docker build caches Rust deps with cargo-chef                                                    | `Dockerfile`                                                                                                                         |
| #17    | OAuth front door for `/mcp` so Claude/ChatGPT/Muse connectors work                               | `apps/server/src/mcp/{oauth,auth}.rs`, `apps/server/src/api/agent_access.rs`, `pages/settings/agent-access/oauth-consent-page.tsx`   |
| #16    | Spending: monthly average beside the total                                                       | `apps/frontend/src/features/spending/components/fork-spending-total.tsx`                                                             |
| #15    | Server keeps prices/valuations current in the background                                         | `apps/server/src/scheduler.rs`, `apps/server/src/api/shared.rs`                                                                      |
| #14    | Quote sync doesn't count shadowed refetched quotes as changed                                    | `crates/core/src/quotes/fork_sync.rs`                                                                                                |
| #13    | 1D shows the last trading day's move on weekends                                                 | dashboard, `lib/fork-performance.ts`                                                                                                 |
| #12    | Branch image builds for arm64                                                                    | `.github/workflows/docker-branch.yml`                                                                                                |
| #11    | Enter a manual holding's total value (derives price)                                             | `apps/frontend/src/pages/asset/update-total-value-dialog.tsx`                                                                        |
| #10    | Skip the recalc when a price sync changed nothing                                                | `apps/server/src/api/shared.rs`, `crates/core/src/quotes/fork_sync.rs`                                                               |
| #9     | Don't cancel a cold profile start on client timeout                                              | `apps/server/src/profiles/fork_startup.rs`                                                                                           |
| #8     | Intraday 1D/1W portfolio chart                                                                   | `apps/server/src/api/holdings/*`, `components/history-chart.tsx`                                                                     |
| #6, #7 | Health: surface activities that need review; warm dashboard only after edits                     | `crates/core/src/health/*`                                                                                                           |
| #5     | Date-only activity dates stored at noon UTC                                                      | `crates/storage-sqlite/src/activities/fork_dates.rs`                                                                                 |
| #4     | Dashboard MWR, year quick picks, cash-back attribution, perf cache                               | `apps/server/src/perf_cache.rs`, `apps/server/src/api/performance/fork_cache.rs`                                                     |
| #3     | MCP data-admin tools, performance caching, closed-position backfill cap                          | `apps/server/src/mcp/`, `apps/server/src/perf_cache.rs`                                                                              |
| #2     | Upstream sync (2026-09-27)                                                                       | merge commit                                                                                                                         |

To see the full divergence:
`git fetch upstream && git log --oneline upstream/main..main` and
`git diff --stat upstream/main...main`.

### Behaviour worth knowing

- **Background updates (#15).** The server refreshes prices every 5 minutes
  during NYSE/TSX hours and hourly otherwise, and skips the portfolio recalc if
  no price changed (#10). Page loads and returning to a hidden tab no longer
  trigger a sync (3.10's resume check is desktop-only), and background runs show
  no toasts. Don't add per-request or per-page-load background work.
- **MCP auth (#17).** `/mcp` accepts a personal access token (Settings → AI
  Agent Access) or OAuth. Approving the consent screen (`/oauth/consent`, behind
  the normal login) creates an ordinary PAT named `<client> (OAuth)`; revoke it
  in the same settings page. No refresh tokens, no expiry. Any HTTPS redirect
  URI is accepted (an allowlist broke Muse's registration). Only read scopes are
  pre-ticked because Claude requests every scope.
- **MCP protocol (#19).** rmcp 1.8 doesn't speak 2026-07-28; we map its 422 to a
  400 so new clients fall back. Native support would need rmcp 3.x.
- **GICs (#25).** A term deposit is a holding of quantity 1 whose asset has
  `metadata.deposit` terms. Quote sync writes a `CALCULATED` price per day from
  them (no provider); the value is flat after maturity, and the redemption is
  still recorded as a SELL. Set terms with the MCP tool `set_term_deposit`,
  which also deletes the asset's manual prices and switches it to MARKET.
- **Login sessions.** `WF_AUTH_TOKEN_TTL_MINUTES=43200` (30 days, renewed on
  use) at the owner's request.
- **Negative-balance health checks.** Ignore cash and total-value residues below
  half the account currency's smallest unit. A real one-cent CAD deficit still
  warns, and an earlier precision residue does not hide a later real deficit.
  This only filters health warnings; stored balances and returns are unchanged.

## Upstream integration

The 3.10 integration merges release `v3.10.0` (`f9643209e`) from the previous
shared ancestor `392f272c5`. Keep upstream's new engine, coordinator and
projection invalidation. Carry fork behavior through the extension points in
[EXTENSIONS.md](EXTENSIONS.md); do not resurrect the removed calculators.

Use a merge commit for an upstream synchronization PR so future merges retain
release ancestry. Routine feature PRs can still use squash. Validate on a
disposable database copy: first startup rebuilds calculated history and can
change derived numbers. A verified pre-upgrade database backup is required for
production rollback; changing the image tag alone does not undo a migration.

## CI scope

This fork deploys the **web app and ARM64 server**. Default PR checks cover web
lint/types/build, two parallel frontend test shards, browser harnesses, and Rust
Clippy and tests (parallel jobs, tests via `cargo nextest`) for the server and
every shared crate (`--exclude wealthfolio-app`). The Rust suite includes HTTP
certificate policy, vault/config and portable-backup tests; its Linux
mounted-vault fixture runs in the tests job. Both Rust jobs restore the cache
that `cache-warm.yml` seeds, keyed on `rust-toolchain.toml` and `Cargo.lock`;
keep their build flags in sync or PRs compile cold. Do not add desktop or mobile
builds to the acceptance gate unless the owner requests them.

Native secret-store checks are manual. Separate server-vault and HTTP workflows
are also manual, with optional macOS/Windows coverage. Weekly cache warming only
builds server-check dependencies and ARM Docker dependencies.

PR Docker builds run only for packaging/configuration changes and use ARM64.
Ordinary application changes use web/server checks; the deployable image still
gets a real ARM build and encryption smoke test before publication. Release tags
retain both architectures; manual amd64 checks are opt-in.

Keep checks proportional: CI/docs-only edits need helper tests and workflow
validation, not a local full app E2E run or a production redeploy. A
frontend-only PR skips Rust. If a larger PR still contains Rust changes, later
UI edits still rerun Rust; do not treat a previous commit's success as
validation of new code.

## Build and deploy

There is no build on the server. CI builds the image; the server pulls it.

1. Merge the validated PR into `main` (preserve merge ancestry for upstream
   syncs). PR checks are `pr-check.yml`, scoped as described above.
2. Build the image:
   ```bash
   gh workflow run docker-branch.yml --repo dddd-zdf/wealthfolio --ref main
   ```
   It builds arm64 by default (`amd64` is optional for separate compatibility
   checks), runs `.github/scripts/encryption_smoke.py` against the image, and
   publishes `mcp-<7-char sha>` and the moving `mcp-categorization` tag. Main
   exports a shared registry cache; feature builds read it but save no cache of
   their own, so they stay out of the 10 GB Actions cache that holds the Rust
   check cache. Only main can replace the shared cache. The first run with new
   dependencies can still be cold; measure actual times rather than promising a
   fixed duration. If an already-tested image's source tree exactly matches
   merged main, reuse that immutable image revision. Do not rebuild solely for a
   new merge label.
3. Verify the image revision/architecture, take and verify a fresh backup, then
   deploy on `home`:
   ```bash
   ssh home sudo /opt/wealthfolio/deploy.sh mcp-<sha>
   ```
   The script pulls the image, replaces the `wealthfolio` container
   (`--restart unless-stopped`, logs capped at 3 × 10 MB, `127.0.0.1:8088`,
   volume `wealthfolio-data` at `/data`, env from `/opt/wealthfolio/.env`,
   `CONNECT_API_URL` blank), and health-checks `http://127.0.0.1:8088/`.
4. Record current/previous image revisions and the backup in the infra repo.
   Image-only rollback is sufficient only without a schema change; otherwise
   restore the matching pre-upgrade data and image using the infra runbook.

To ship several unmerged PRs together, merge them into a throwaway
`deploy/<date>` branch and dispatch `docker-branch.yml` on that branch.

The `mcp-<sha>` tag uses 7 characters; `git log --oneline` may print 8.

## Server layout

| Path / unit                                | Purpose                                                                                                                                                                                                                              |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `/opt/wealthfolio/.env` (root, 600)        | `WF_LISTEN_ADDR`, `WF_DB_PATH`, `WF_SECRET_KEY`, `WF_CORS_ALLOW_ORIGINS`, `WF_MCP_ENABLED`, `WF_AUTH_PASSWORD_HASH`, `WF_AUTH_TOKEN_TTL_MINUTES`. Never print values. `WF_SECRET_KEY` encrypts stored secrets; losing it loses them. |
| `/opt/wealthfolio/deploy.sh`               | Pull-and-replace deploy (above)                                                                                                                                                                                                      |
| `/opt/wealthfolio/set-password.sh`         | Prompts for a new login password, stores only an Argon2id hash, redeploys the current tag. The owner asked for no password rules.                                                                                                    |
| `/opt/wealthfolio/paycheque.py` + `.token` | Records the owner's biweekly pension contribution via `/mcp` (reads the DB read-only to avoid duplicates). Run by `wf-paycheque.timer` daily at 18:00 America/Vancouver. Logs: `journalctl -u wf-paycheque`.                         |
| Docker volume `wealthfolio-data`           | SQLite DB and in-app backups (`_data/backups/`)                                                                                                                                                                                      |
| `/etc/caddy/Caddyfile`                     | Caddy terminates TLS for the domain and proxies to `127.0.0.1:8088`                                                                                                                                                                  |
| DNS                                        | DuckDNS A record for `dddfwealth`                                                                                                                                                                                                    |

Firewall: both the OCI security list and host iptables allow 80/443.

The account is Pay As You Go with a $1 "stay-free" budget alert. Stay inside the
Always Free limits (4 OCPU / 24 GB A1 total, 200 GB block storage) before
resizing or adding volumes.

The former E2 VM was deleted. Nocturne and Wealthfolio backup reception now run
on `util`; leave those services and the other apps on `home` untouched.

## Runbooks

**App looks slow or times out.** Check `top` for steal time and
`docker stats wealthfolio` before assuming a code bug. Avoid rapid reloads and
bulk mutations when testing on the live host.

**Connector (Claude/ChatGPT/Muse) problems.** `docker logs wealthfolio` and look
at `/oauth/*` and `/mcp` status codes. A 422 on `server/discover` means the #19
fallback regressed.

**Login problems.** `sudo /opt/wealthfolio/set-password.sh`.

**Backups.** Only the in-app backups inside the volume exist today; there is no
off-host copy. Before risky data work, create a backup from Settings (or copy
the volume's DB while the container is stopped).

## Local development (owner's Windows PC)

- `pnpm` isn't on PATH; use `corepack pnpm`.
- Vendored OpenSSL needs Strawberry Perl first on PATH
  (`C:\Strawberry\perl\bin`); Git's perl fails. Run cargo through
  `cmd /c "cargo ..."` from PowerShell.
- The server is the web build: `pnpm dev:web` for development, `pnpm build` to
  check the bundle. The Tauri desktop app isn't used in production here.

## Working agreements with the owner

- Plain, short explanations; minimal fuss; pick sensible defaults instead of
  asking.
- Once a fix is agreed and CI is green, merge and deploy without asking again.
  Still ask about real design choices and anything that changes data.
- Data writes (imports, row fixes) are done by the owner's assistant agent, not
  by the engineer agent: investigate, reconcile against statements with the
  owner, then send exact instructions. Details and the Drive inbox protocol are
  in the private Agent Workspace README.
- Keep this file current when you merge a fork PR, change the deploy pipeline,
  or sync upstream.
