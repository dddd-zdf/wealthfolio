# Upstream v3.11.0 integration

## Source and scope

- Fork base: `dddd-zdf/wealthfolio` main, `268b3e67f`.
- Upstream release: `wealthfolio/wealthfolio` tag `v3.11.0`, commit
  `d1fda0b65f69d8557b1ef80e555c799215ad28b3` (annotated tag object
  `87814e29d63a9b4c0e007abac1d84cb2ca588b4c`).
- Integration branch: `codex/upstream-3.11`.
- Preserve the upstream merge commit when promoting this branch. Do not squash
  or replay the release as individual commits.
- The open currency-history PR #46 is separate work and is not included here.
  Its branch and working directory are preserved.

## Architecture impact

Adopt upstream's engine, coordinator, persistence, theme and UI updates. Retain
fork behavior through the existing extension modules and thin hooks described in
[EXTENSIONS.md](EXTENSIONS.md). This integration adds no fork storage, provider,
retry or background-worker mechanism and does not widen MCP grants. Upstream
changes to synchronization, calculation and persistence are included as release
changes, rather than described as a behavior-preserving refactor.

## Conflict resolutions

- Keep fork managed account values, period gains, review badges, manual holding
  total entry and calendar-year presets while adopting upstream's Base UI
  trigger API, navigation and layout changes.
- Run fork needs-review health analysis inside upstream's new blocking history
  task. Preserve the existing negative-balance precision filter.
- Accept upstream's shared tracing capture helper for vault tests, including its
  platform-independent use; retain the existing vault protection tests.
- Retain separate web/server Clippy and test jobs and existing cache settings;
  add upstream script, theme and design checks without native build gates.
- Keep unavailable valuations out of intraday chart anchors, matching upstream
  daily-chart availability checks. Forward upstream additive MCP annotations
  through the reviewed-rule adapter and annotate direct rule creation.
- Adapt the dashboard IRR regression to upstream's updated engine signature.
- Supply fork translations for upstream's Arabic locale and align fork styles
  with upstream's design rules, without raising the design-debt baseline.
- Pin generated theme CSS to LF so Windows checkout conversion cannot make the
  generator's exact-content check fail.

## Acceptance and deployment

Use the web/server checks in `pr-check.yml`. Report exact checks and limitations
in the integration PR. A local Windows check does not establish ARM64 runtime or
production-data compatibility. Production deployment requires the infra runbook,
a verified backup, disposable database-copy comparison and a tested ARM64 image.
Deploy the validated integration branch directly at the owner's request; keep
`main` unchanged and record image, backup and verification in the infra runbook.
