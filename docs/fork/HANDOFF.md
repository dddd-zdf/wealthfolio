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

## At a glance

| What         | Where                                                                         |
| ------------ | ----------------------------------------------------------------------------- |
| Fork         | `github.com/dddd-zdf/wealthfolio` (remote `origin`); upstream is `upstream`   |
| Live app     | `https://dddfwealth.duckdns.org` (web build, password login)                  |
| MCP endpoint | `https://dddfwealth.duckdns.org/mcp` (PAT or OAuth)                           |
| Host         | Oracle Cloud Always Free A1 VM (arm64, Ubuntu 24.04), Toronto region          |
| Images       | `ghcr.io/dddd-zdf/wealthfolio:mcp-<7-char sha>`, built by `docker-branch.yml` |
| Branching    | Feature branch → PR into `main` (squash) → build image from `main` → deploy   |

## What the fork changes

All fork work is in PRs on `dddd-zdf/wealthfolio`. Notable changes, newest
first:

| PR     | Change                                                                                    | Main files                                                                                                                         |
| ------ | ----------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| #25    | GICs/term deposits valued from their terms (daily accrual); MCP `set_term_deposit`        | `crates/core/src/assets/term_deposit.rs`, `crates/core/src/quotes/sync.rs`, `crates/agent-tools/src/tools/data_admin.rs`           |
| #24    | Pencil on a managed account's total (dashboard + account page) sets manual fund prices    | `apps/frontend/src/components/managed-value-edit.tsx`, `apps/frontend/src/lib/managed-account.ts`                                  |
| #23    | Faint start-of-period line on the dashboard chart for 1W–5Y                               | `apps/frontend/src/pages/dashboard/dashboard-content.tsx`                                                                          |
| #21    | 1D chart zooms to the day's move and marks the previous close                             | `apps/frontend/src/components/history-chart*.ts(x)`                                                                                |
| #20    | Zero day change for manual prices not updated today                                       | `crates/core/src/portfolio/holdings/holdings_valuation_service.rs`                                                                 |
| #19    | MCP: clients speaking protocol 2026-07-28 (ChatGPT) fall back to `initialize` (422 → 400) | `apps/server/src/mcp/mod.rs`                                                                                                       |
| #18    | Docker build caches Rust deps with cargo-chef                                             | `Dockerfile`                                                                                                                       |
| #17    | OAuth front door for `/mcp` so Claude/ChatGPT/Muse connectors work                        | `apps/server/src/mcp/{oauth,auth}.rs`, `apps/server/src/api/agent_access.rs`, `pages/settings/agent-access/oauth-consent-page.tsx` |
| #16    | Spending: monthly average beside the total                                                | spending page                                                                                                                      |
| #15    | Server keeps prices/valuations current in the background                                  | `apps/server/src/scheduler.rs`, `apps/server/src/api/shared.rs`                                                                    |
| #14    | Quote sync doesn't count shadowed refetched quotes as changed                             | `crates/core/src/quotes/sync.rs`                                                                                                   |
| #13    | 1D shows the last trading day's move on weekends                                          | dashboard, `lib/holding-performance.ts`                                                                                            |
| #12    | Branch image builds for arm64                                                             | `.github/workflows/docker-branch.yml`                                                                                              |
| #11    | Enter a manual holding's total value (derives price)                                      | `apps/frontend/src/pages/asset/update-total-value-dialog.tsx`                                                                      |
| #10    | Skip the recalc when a price sync changed nothing                                         | `apps/server/src/api/shared.rs`, `crates/core/src/quotes/sync.rs`                                                                  |
| #9     | Don't cancel a cold profile start on client timeout                                       | `apps/server/src/profiles.rs`                                                                                                      |
| #8     | Intraday 1D/1W portfolio chart                                                            | `apps/server/src/api/holdings/*`, `components/history-chart.tsx`                                                                   |
| #6, #7 | Health: surface activities that need review; warm dashboard only after edits              | `crates/core/src/health/*`                                                                                                         |
| #5     | Date-only activity dates stored at noon UTC                                               | `crates/storage-sqlite/src/activities/model.rs`                                                                                    |
| #4     | Dashboard MWR, year quick picks, cash-back attribution, perf cache                        | `apps/server/src/perf_cache.rs`, `apps/server/src/api/performance.rs`                                                              |
| #3     | MCP data-admin tools, performance caching, closed-position backfill cap                   | `apps/server/src/mcp/`, `apps/server/src/perf_cache.rs`                                                                            |
| #2     | Upstream sync (2026-09-27)                                                                | merge commit                                                                                                                       |

To see the full divergence:
`git fetch upstream && git log --oneline upstream/main..main` and
`git diff --stat upstream/main...main`.

### Behaviour worth knowing

- **Background updates (#15).** The server refreshes prices every 5 minutes
  during NYSE/TSX hours and hourly otherwise, and skips the portfolio recalc if
  no price changed (#10). Page loads no longer trigger a sync, and background
  runs show no toasts. Don't add per-request or per-page-load background work.
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

## Upstream sync status

The fork last merged upstream on 2026-09-27 (merge base `392f272c5`). Upstream
has since moved a long way (about 266 commits by 2026-10-06), including a full
replacement of the portfolio calculators with a new portfolio engine (upstream
#1847), loans, and per-account cost-basis methods. Expect real conflicts in
`apps/server/src/perf_cache.rs`, `api/performance.rs`, `api/holdings/*`,
`crates/core/src/portfolio/*` and `crates/core/src/quotes/sync.rs`.

When syncing: branch from `main`, `git merge upstream/main`, re-apply each fork
behaviour above against the new code (some, like #20 or #13, may be fixed or
obsoleted upstream—check before porting), run the validation in AGENTS.md, then
deploy and watch the dashboard numbers against the previous image. Don't rebase
`main`; it is shared history.

## Build and deploy

There is no build on the server. CI builds the image; the server pulls it.

1. Merge the PR into `main` (squash). PR checks are `pr-check.yml`; Android and
   iOS compile checks are skipped on PRs in this fork.
2. Build the image:
   ```bash
   gh workflow run docker-branch.yml --repo dddd-zdf/wealthfolio --ref main
   ```
   It builds arm64 only by default (tick the `amd64` input for the old x86 VM),
   runs `.github/scripts/encryption_smoke.py` against the image, and publishes
   `mcp-<7-char sha>` and the moving `mcp-categorization` tag. Warm builds take
   about 13 minutes. GHA caches are branch-scoped; if `main`'s `Cargo.lock`
   changes, run the workflow on `main` once so branch builds stay warm.
3. Deploy on the server:
   ```bash
   sudo /opt/wealthfolio/deploy.sh mcp-<sha>
   ```
   The script pulls the image, replaces the `wealthfolio` container
   (`--restart unless-stopped`, `127.0.0.1:8088`, volume `wealthfolio-data` at
   `/data`, env from `/opt/wealthfolio/.env`, `CONNECT_API_URL` blank), and
   health-checks `http://127.0.0.1:8088/`.
4. Rollback = run `deploy.sh` with the previous tag. Note the current and
   previous tags somewhere (the private README keeps them).

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

The old x86 E2 micro VM (previous home, until 2026-10-03) still runs the owner's
**Nocturne** service. Never stop, prune or delete Nocturne files, services or
timers there. Its stopped Wealthfolio container is a rollback that can be
cleaned up after about 2026-10-10.

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
