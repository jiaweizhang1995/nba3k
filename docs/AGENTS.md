# AGENTS.md — onboarding for a fresh session

You are coding on `nba3k`. This doc gets you from zero to productive without asking the human anything.

## What this is

`nba3k` is an NBA 2K MyGM-style GM simulator. Single SQLite file = one save. Three interactive surfaces share one parser:

1. **CLI subcommands** — `nba3k --save x.db <cmd>`
2. **REPL** — `nba3k --save x.db` with no subcommand drops into `rustyline`
3. **TUI** — `nba3k --save x.db tui` (ratatui-based)

Personal / non-commercial. Public NBA data scraped politely from Basketball-Reference + ESPN.

## What you need to know in 60 seconds

- 8-crate Rust workspace (`crates/nba3k-{core,models,sim,trade,season,store,scrape,cli}`). See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the boundaries and data flow.
- All persistence goes through `nba3k-store`. Schema is migration-first (`crates/nba3k-store/migrations/V###__*.sql`). Never edit a committed migration; add a new one.
- All runtime knobs live in `data/*.toml` (archetypes, GM personalities, sim params, weights). Treat them as content.
- `crates/nba3k-cli/src/commands.rs` is one large dispatch file. Don't split it speculatively.
- Three interactive surfaces share one `Command` enum (`crates/nba3k-cli/src/cli.rs`). Add a command once, get all three for free.
- New saves default to a **live ESPN snapshot** (post-M34); pass `--offline` for the legacy 2025-10-21 fresh start. See [`RUNNING.md`](RUNNING.md) for the difference.

## Read in this order

1. **This file** — orient.
2. **[`ARCHITECTURE.md`](ARCHITECTURE.md)** — workspace, data flow, schema rules.
3. **[`RUNNING.md`](RUNNING.md)** — how to build, run, and create test saves.
4. **[`VERIFICATION.md`](VERIFICATION.md)** — every test / lint / smoke command. Run these before committing.
5. **[`PROGRESS.md`](PROGRESS.md)** — what's done, what's in flight, where the backlog is.

If anything in those files contradicts code on disk, code wins. Update the doc.

## Conventions

These are non-negotiable. The repo will reject changes that break them.

- **REPL parity for every CLI command.** Everything goes through `Command` enum + `commands::dispatch`.
- **No persistence outside `nba3k-store`.** All writes route through migration-managed schema.
- **No cross-command caching.** Snapshots rebuild from the DB each call so commands compose cleanly under `--script`.
- **Workspace-pinned deps only.** Add to root `Cargo.toml` `[workspace.dependencies]`, then reference with `workspace = true` from member crates.
- **Phase work uses agent teams.** Team name format: `nba3k-m{N}`. Workers own non-overlapping crate paths. Integration is the orchestrator's job.
- **Bash-verifiable artifact ends every phase.** PHASES.md row records the exact command.
- **i18n parity.** Every TUI string routes through `t(lang, T::...)`. Adding a key means updating `i18n.rs` + `i18n_en.rs` + `i18n_zh.rs` together.
- **Player names + team abbreviations + team full names stay English.** Even in Chinese UI mode — they are data, not chrome.
- **Bash-tool grep is rewritten by the rtk hook.** Use the Grep tool directly when searching. `grep` via Bash will silently mangle output.

## How to commit

- Tests must pass before marking a task done. Baseline (post-M36 pick trading): **338 unit + 2 ignored** across 74 suites. Each task either holds or grows the count.
- Use `git commit` not `git commit --amend` unless the user explicitly asked. Co-author trailer is in CLAUDE.md.
- Do NOT push to remote unless the user asks.

## When in doubt

- The repo's `CLAUDE.md` (root) has the same shape as this doc but is auto-loaded by Claude Code.
- For commands, lean on root `Makefile`. It calls cargo through the same toolchain pinned by `rust-toolchain.toml`.
- For domain questions about NBA mechanics: `RESEARCH.md` and `RESEARCH-NBA2K.md` capture the original homework.
