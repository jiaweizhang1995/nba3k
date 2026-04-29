# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`nba3k` is an NBA 2K MyGM–style GM simulator. Rust workspace, single SQLite save file, three interaction modes: CLI subcommands, interactive REPL, and a `ratatui` TUI. Personal / non-commercial.

## Common commands

```bash
# Build (debug / release)
cargo build
cargo build --release

# Tests — run the whole workspace; CI parity
cargo test --workspace

# A single test (filter by path::name)
cargo test -p nba3k-trade -- evaluator::tests::accept_threshold
cargo test -p nba3k-cli --test integ -- season1   # integration test

# Lint / format
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all

# Run a save against the dev binary (no rebuild churn)
cargo run -- --save /tmp/dev.db new --team BOS
cargo run -- --save /tmp/dev.db status --json
cargo run -- --save /tmp/dev.db tui

# Replay a deterministic season (used as integ smoke)
cargo run --release -- --save /tmp/season1.db --script tests/scripts/season1.txt

# Rebuild seed DB (slow; only when scrape logic changes)
cargo run -p nba3k-scrape --release -- --out data/seed_2025_26.sqlite

# Verbose logs (env filter)
NBA3K_LOG=debug cargo run -- --save /tmp/dev.db sim-day --days 5
```

The release toolchain is pinned in `rust-toolchain.toml` (stable, with `rustfmt` + `clippy`). `rusqlite` is bundled — no system SQLite needed.

## Three entry points share one parser

`crates/nba3k-cli/src/main.rs` decides between four modes based on argv + stdin:

1. **Subcommand given** → `commands::dispatch` and exit.
2. **`--script <file>`** → replay each line via `repl::run_script`.
3. **stdin not a TTY** → pipe mode (`repl::run_pipe`).
4. **Otherwise** → interactive `rustyline` REPL.

The same `Command` enum (`crates/nba3k-cli/src/cli.rs`) drives all four. REPL lines are tokenized with `shlex` and fed back into `clap`. **When you add a new command, you only wire it once** — argv and REPL pick it up together. Add to the `Command` enum, then handle it in `commands::dispatch`.

The TUI is launched via the `tui` subcommand (`nba3k --save x.db tui`). It is implemented as screens that *call into the same command/state APIs*, not a parallel codebase.

## Workspace layout (8 crates, all pinned via `[workspace.dependencies]`)

```
nba3k-core      Public types — Player, Team, LeagueYear, TradeOffer, LeagueSnapshot.
                Everything downstream depends on this.
nba3k-models    7 explainable scoring models — value, contract, context, star protection,
                fit, trade-accept, stat projection. Pure functions.
nba3k-sim       Statistical match sim + 9-D team strength vector. Per-possession sim is v2 (not built).
nba3k-trade     Trade engine: evaluator, CBA validator, GM personality, multi-round negotiation,
                3-team support. Stateless given a snapshot.
nba3k-season    Schedule generation, playoffs, awards, HoF, all-star, Cup.
nba3k-store     SQLite persistence + `refinery` migrations. Owns ALL schema.
nba3k-scrape    Bootstrap scraper (Basketball-Reference, rate-limited 1 req / 3s) + calibration tools.
nba3k-cli       argv parser, REPL, TUI, command implementations. The thick top of the stack.
```

`nba3k-cli/src/commands.rs` is intentionally one large file (~255 KB) — it is the dispatch table for every command across CLI/REPL. Don't split it speculatively; phases have specifically chosen to keep it as one router.

## Data flow

The single source of truth is the SQLite save file. The pattern is:

1. Open save → `Store` from `nba3k-store`.
2. Build a `LeagueSnapshot` (in `nba3k-core`) for the active season.
3. Pass the snapshot (read-only) into `nba3k-sim` / `nba3k-trade` / `nba3k-season` / `nba3k-models`.
4. Persist results back through `Store`.

Snapshots are derived state — they are NOT cached across commands. Each command rebuilds from the DB, so commands compose cleanly in scripts.

## Schema is migration-first

Schema lives only as `.sql` files in `crates/nba3k-store/migrations/V###__name.sql` (currently V001–V015). `refinery` runs them in order on every store open. **Never edit a committed migration** — add a new one. Migration numbers are referenced in `phases/PHASES.md` as part of phase verification (e.g., V006 = free agents, V014 = rotation), so the numbering doubles as a phase changelog.

## Save file convention

A save is a `.db` file plus `-shm` / `-wal` siblings (WAL mode). `data/seed_2025_26.sqlite` is the read-only league seed — every `new` clones it. Don't touch the seed for game-state writes; if you need to alter the starting universe, regenerate via `nba3k-scrape`.

## Testing patterns

- Unit tests live next to code in each crate's `src/` plus a per-crate `tests/` directory.
- The end-to-end smoke is `crates/nba3k-cli/tests/` driven by `tests/scripts/season1.txt` — running it should complete a full season deterministically. Total wall time ≈ a few seconds.
- Total count tracked in `phases/PHASES.md` (currently ~281 unit + 1 integ); a phase doesn't sign off without a passing Bash-verifiable artifact.
- Determinism comes from explicit RNG seeds threaded through `rand_chacha`. When fixing a sim/trade bug, prefer a regression test that pins the seed over a new heuristic.

## Game-mode flag

`--god` (or `mode = god` at save creation) skips CBA validation and forces AI to accept trades. Several commands branch on this; new validation logic must respect it (`AppState::god` / per-save `mode` column).

## Phases & docs

- `docs/` is the canonical project reference. Start at `docs/AGENTS.md` for an onboarding tour, then `ARCHITECTURE.md` / `RUNNING.md` / `VERIFICATION.md` / `PROGRESS.md`. The root `Makefile` wraps the common cargo targets.
- `phases/PHASES.md` is the per-milestone status board (currently through M35). Each milestone has its own `phases/M{N}-*.md` with goals + verification commands.
- `data/*.toml` files are the tunable knobs (archetypes, personalities, weights, sim params). Treat them as content, not config — changes affect simulation balance.
- README.md is the user-facing reference for every CLI subcommand (in Chinese; CLI flags are in English).
- `RESEARCH.md` and `RESEARCH-NBA2K.md` capture domain research; they are reference, not a spec.

## Conventions worth knowing before editing

- **Don't add a new top-level command without REPL parity** — they share the parser.
- **Don't introduce a new persistence path that bypasses `nba3k-store`.** All writes go through the migration-managed schema.
- **Snapshot rebuilds are cheap; cross-command caching is not allowed.** Each command must work after a fresh open.
- **Workspace-pinned deps only.** Add new deps in the root `Cargo.toml` `[workspace.dependencies]` then reference with `workspace = true`.
- **Phase work uses agent teams** named `nba3k-m{N}` with non-overlapping crate ownership; integration is reserved for the orchestrator. See `phases/PHASES.md` "Working agreements".
- **Bash verification artifact ends every phase.** Don't mark a phase done without the assertion command passing.
