# Fork rules for Claude

Fork-only instructions live in `.claude/rules/fork*.md`. Upstream has no
`.claude/rules/` directory, so these files never conflict in an upstream merge.
Add new fork instructions here, not to upstream-owned files (`AGENTS.md`,
`.claude/settings.json`, `.claude/skills/`, `.claude/commands/`).

Before changing application code, follow
[docs/fork/EXTENSIONS.md](../../docs/fork/EXTENSIONS.md): fork logic goes in
`fork_*.rs` / `fork-*.ts(x)` files, and upstream files get thin hooks only (a
`mod`/`use` line, a call, a route entry). If a change needs more than a hook in
an upstream file, say so in the plan and PR and keep that edit as one block.
Update the extension map when you add or move fork code.
