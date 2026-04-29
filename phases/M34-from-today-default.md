# M34 — Live ESPN start is the default

**Status**: ✅ Done
**Started**: 2026-04-29
**Completed**: 2026-04-29

## Goal

Flip `cmd_new`'s default. The legacy fresh-October-2025 seed-anchored path was a stand-in until the live ESPN importer landed; now that M32+M33 are stable, the default user experience matches NBA 2K MyNBA "Today's Game" — every fresh save lands on today's real-world NBA state without needing any flag.

## Behavior changes

- `nba3k --save x.db new --team BOS` now performs a live ESPN import (was: copy seed + RNG schedule).
- New `--offline` flag opt-out replays the legacy path: copy `data/seed_2025_26.sqlite` + `Schedule::generate_with_dates` synthetic 1230-game schedule + 0-0 standings + day=0 / phase=PreSeason.
- `--from-today` flag is deprecated and hidden; kept as a no-op so any user scripts that already use it still parse.
- TUI new-game wizard drops the M33 "Start mode" step entirely. Wizard flow returns to 5 steps: SavePath → Team → Mode → Season → Confirm. The 4 i18n keys added in M33 (`NewGameStartTitle` / `NewGameStartFresh` / `NewGameStartToday` / `NewGameStartTodayHint`) are removed from `i18n.rs` + EN + ZH.

## Decisions

- **Keep `--offline` instead of removing the seed path entirely.** Two reasons: (1) integration tests need a deterministic, network-free `new` path (`integration_season1.rs`, `all_star_smoke.rs`, `rumors_smoke.rs`, etc. all pin `--offline`); (2) advanced users who want to replay a season from opening night still have the option. Using `--offline` keeps the test infrastructure trivial.
- **Hide `--from-today` rather than delete it.** Existing user scripts and the M32 commit message reference the flag. Hiding it (`hide = true`) keeps the help text clean while preserving back-compat.
- **TUI dropping the start step.** The original M33 wizard added a 3rd step to let users opt in to the live import. With the default flipped, that choice is gone — there's nothing to choose between, so the step + state + i18n keys all come out.

## Files touched

- `crates/nba3k-cli/src/cli.rs` (NewArgs: `from_today` hidden; `offline` added)
- `crates/nba3k-cli/src/commands.rs` (cmd_new default branch flipped to `from_today`)
- `crates/nba3k-cli/src/tui/screens/new_game.rs` (Step::Start + StartMode + render_start_picker removed; submit() initializes both flags to false)
- `crates/nba3k-core/src/i18n.rs` + `i18n_en.rs` + `i18n_zh.rs` (4 NewGameStart* keys removed from all 3)
- `crates/nba3k-cli/tests/{integration_season1,all_star_smoke,rumors_smoke,compare_smoke,m22_trade_builder_smoke,offers_smoke,season_advance_calendar}.rs` (each pins `--offline` on bootstrap)
- `README.md` (defaults + `--offline` opt-out)
- `phases/PHASES.md` row M34

## Verification

```bash
cargo test --workspace                                       # 320 passed + 2 ignored
cargo run --bin nba3k -- --save /tmp/today.db new --team BOS  # live ESPN import (default)
cargo run --bin nba3k -- --save /tmp/legacy.db new --team BOS --offline  # opt-out path
sqlite3 /tmp/legacy.db "SELECT MIN(date), MAX(date), COUNT(*) FROM schedule;"
# → 2025-10-21 | 2026-04-12 | 1230  (seed-anchored path still byte-identical)
./target/release/nba3k --save /tmp/today.db status --json
# → phase: Regular | TradeDeadlinePassed | Playoffs (depends on real-world day)
./target/release/nba3k --save /tmp/today.db new --help | grep -E "from-today|offline"
# → --offline (visible), --from-today hidden but accepted
```

## Polish items deferred

- TUI doesn't yet show a progress indicator for the ≈30-second cold-cache import. Users see a frozen wizard while ESPN fetches run.
- README still has a `pip install nba_api` note in section 2 (system requirements) that is no longer accurate for `--from-today`. Should be removed in a follow-up.
- `--offline` help text is in English only; localize when the next i18n pass runs.
