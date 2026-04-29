# RUNNING.md — how to build, run, and create saves

## Toolchain

Rust stable, pinned in `rust-toolchain.toml`. `rusqlite` is bundled
(static link), so no system SQLite is required. `rustfmt` and `clippy`
are on the toolchain.

```bash
rustup show          # confirms pinned channel
rustc --version      # should be a recent stable
```

## Build

Use the root `Makefile` for shortcuts (`make build`, `make release`),
or call cargo directly:

```bash
cargo build                      # debug
cargo build --release            # release
cargo build --release --bin nba3k   # release, just the user binary
```

The release binary lands at `target/release/nba3k`. The debug binary
is at `target/debug/nba3k`.

## Run

The binary is `nba3k`. Every save is a single `.db` file (plus
`-shm` / `-wal` siblings while open).

```bash
# Live ESPN snapshot (default, post-M34)
nba3k --save /tmp/today.db new --team BOS

# Legacy fresh-October 2025 save (no internet, deterministic)
nba3k --save /tmp/legacy.db new --team BOS --offline

# Look at the new save
nba3k --save /tmp/today.db status --json
nba3k --save /tmp/today.db roster LAL
nba3k --save /tmp/today.db standings

# REPL
nba3k --save /tmp/today.db
> sim-week
> messages
> trade list
> quit

# TUI (8-menu shell)
nba3k --save /tmp/today.db tui
nba3k --save /tmp/today.db tui --tv       # high-contrast TV preset
nba3k --save /tmp/today.db tui --legacy   # M19 5-tab read-only fallback
```

Without `--save`, `nba3k tui` drops into the new-game wizard.

## Cargo run shortcuts (no rebuild churn)

```bash
cargo run --bin nba3k -- --save /tmp/dev.db new --team BOS
cargo run --bin nba3k -- --save /tmp/dev.db status --json
cargo run --bin nba3k -- --save /tmp/dev.db tui
```

## Scripted season (deterministic)

```bash
cargo run --release --bin nba3k -- \
  --save /tmp/season1.db --script tests/scripts/season1.txt
```

Used as the integ smoke test — total wall time ≈ 5-10 s.

## Pipe mode

stdin not a TTY → `nba3k` reads each line as a command:

```bash
echo "sim-to season-end" | nba3k --save /tmp/x.db
printf "status\nstandings --json\n" | nba3k --save /tmp/x.db
```

## Rebuild the seed DB (rare)

Only when scrape logic or source data changes:

```bash
cargo run -p nba3k-scrape --release -- --out data/seed_2025_26.sqlite
```

Slow — rate-limited at 1 req / 3 s for Basketball-Reference. ~3 minutes
on a cold cache.

## Environment variables

| Var | Effect |
|---|---|
| `NBA3K_SAVE` | Default save path; same as `--save` |
| `NBA3K_LOG` | Tracing filter (e.g. `debug`, `info`, `nba3k_scrape=trace`). Output goes to stderr. |
| `NBA3K_CACHE_DIR` | Override the scrape cache root (default `data/cache/`) |

Example:

```bash
NBA3K_LOG=debug cargo run --bin nba3k -- --save /tmp/dev.db sim-day --days 5
NBA3K_LOG=nba3k_scrape::sources::espn=trace nba3k --save /tmp/today.db new --team BOS
```

## Save management

```bash
# List saves in cwd + /tmp
nba3k saves list

# Show metadata for one save
nba3k saves show --path /tmp/today.db

# Delete (the --yes flag is required as a safety check)
nba3k saves delete --path /tmp/today.db --yes

# Or just rm the three files
rm /tmp/today.db /tmp/today.db-shm /tmp/today.db-wal
```

## Common one-liners

```bash
# Drive a fresh live save through a season-long sim
nba3k --save /tmp/full.db new --team BOS && \
  nba3k --save /tmp/full.db sim-to season-end && \
  nba3k --save /tmp/full.db season-summary

# Replay the integ smoke without the test harness
cargo build --release --bin nba3k && \
  ./target/release/nba3k --save /tmp/replay.db new --team BOS --offline && \
  ./target/release/nba3k --save /tmp/replay.db --script tests/scripts/season1.txt
```

See `../README.md` for the complete CLI subcommand reference (in
Chinese; flags are English).
