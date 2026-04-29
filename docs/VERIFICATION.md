# VERIFICATION.md — tests, lints, smoke checks

## Required before committing

```bash
make verify
# or, equivalently:
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

Tests must pass. Clippy must pass without new warnings (regressions are
not OK; existing pre-M31 warnings are accepted while the cleanup
backlog clears). Format must be clean.

For aspirational CI parity:

```bash
make lint-strict        # clippy with -D warnings, currently fails on
                        # legacy nits in nba3k-trade / commands.rs
```

## Test counts

Baseline as of M36 pick trading / commit `bd8deb5`:

- `cargo test --workspace` → **338 passed + 2 ignored**, 74 suites, ~14 s wall

Each new task either keeps the count or grows it. A drop is a
regression.

## Run a single test

```bash
# By package + test name
cargo test -p nba3k-trade -- evaluator::tests::accept_threshold

# Whole module
cargo test -p nba3k-trade -- evaluator::tests

# By integration-test file (under crates/<crate>/tests/)
cargo test -p nba3k-store --test season_calendar
cargo test -p nba3k-cli --test integration_season1

# Across the workspace, filter by name fragment
cargo test --workspace -- builder_asset_row
```

## Ignored tests

```bash
cargo test --workspace -- --ignored          # run them all
cargo test -p nba3k-cli --test integration_season1 -- --ignored
```

The ignored set right now (post-M35):

- `crates/nba3k-cli/tests/integration_season1.rs::full_season_scripted`
  — needs `target/release/nba3k` + `data/seed_2025_26.sqlite`. Drives a
  full scripted season + start of season 2.
- `crates/nba3k-cli/tests/season_advance_calendar.rs::season_advance_writes_calendar_row_for_each_new_year`
  — same — full season sim, then asserts per-year `season_calendar`
  rows.

Both are pinned to `--offline` so they don't need internet.

## Lint / format

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all                                  # apply
cargo fmt --all -- --check                       # CI-style: just check
```

Pre-existing warnings live in `crates/nba3k-trade/` and a couple of
`drop(LeagueSnapshot)` calls in `crates/nba3k-cli/src/commands.rs`.
Don't introduce new ones.

## Smoke checks (manual)

After a change to the live importer, schedule generator, or `cmd_new`:

```bash
# Legacy path byte-identical
rm -f /tmp/legacy.db /tmp/legacy.db-shm /tmp/legacy.db-wal
nba3k --save /tmp/legacy.db new --team BOS --offline
sqlite3 /tmp/legacy.db \
  "SELECT MIN(date), MAX(date), COUNT(*) FROM schedule;"
# → 2025-10-21 | 2026-04-12 | 1230

# Live import works (needs internet)
rm -f /tmp/today.db /tmp/today.db-shm /tmp/today.db-wal
nba3k --save /tmp/today.db new --team BOS
nba3k --save /tmp/today.db status --json
nba3k --save /tmp/today.db roster LAL    # check Doncic / LeBron / Reaves [INJ]
nba3k --save /tmp/today.db standings | head -8

# Scripted-season integ smoke
cargo build --release --bin nba3k
./target/release/nba3k --save /tmp/replay.db new --team BOS --offline
./target/release/nba3k --save /tmp/replay.db --script tests/scripts/season1.txt
```

## Determinism check

When a sim or trade fix lands, pin a regression test with an explicit
seed. Don't rely on `thread_rng`. The `rand_chacha::ChaCha8Rng` is the
only allowed RNG path in non-test code; tests can use `StdRng` if they
seed it.

## Make it easy

The `Makefile` wraps the most-used calls so you don't memorize them:

```bash
make build       # debug build, all crates
make release     # release build, just the bin
make test        # workspace tests
make lint        # clippy + fmt --check
make fmt         # apply rustfmt
make verify      # build + test + lint, all of it
make clean       # nuke target/ and data/cache/espn/
```
