# M32 — Live importer + `--from-today` flag

**Status**: ✅ Done
**Started**: 2026-04-29
**Completed**: 2026-04-29

## Goal

Wire the M31 ESPN client into a single-call importer that produces a fully-populated save from today's real-world NBA state, then expose it as `nba3k --save x.db new --team BOS --from-today`.

## Sub-tasks

| # | Item | Status |
|---|------|--------|
| T5 | V017 `player_season_stats` migration + struct + Store API | ✅ |
| T6 | `nba3k-cli::from_today` importer module (Rust-native, ESPN) | ✅ |
| T7 | Wire `--from-today` flag in `cmd_new` | ✅ |
| T8 | This phase doc + PHASES.md row | ✅ |

## Decisions

- **Importer lives in `nba3k-cli`, not `nba3k-scrape`**. The importer reuses `populate_default_starters` / `assign_initial_roles` / `seed_free_agents` from `commands.rs`, so it must sit in the same crate. The plan called this out as a choice; placing it in `nba3k-cli/src/from_today.rs` keeps the scrape crate free of game-bootstrap helpers.
- **ESPN-abbrev → seed-abbrev translation**. ESPN uses `BKN` / `CHA` / `GS` / `NO` / `NY` / `PHX` / `SA` / `UTAH` / `WSH`; the seed (BBRef-derived) uses `BRK` / `CHO` / `GSW` / `NOP` / `NYK` / `PHO` / `SAS` / `UTA` / `WAS`. A small `espn_to_seed_abbrev` translates. Without it only 21/30 teams matched; with it 30/30.
- **Past games carry minimal box scores**. ESPN gives final scores but not per-player box scores for completed games. The importer writes `BoxScore { home_lines: vec![], away_lines: vec![] }`. Downstream code (`record_game`, recap, awards) tolerates empty lines. For PPG/RPG leaderboards, the M31 `player_season_stats` table fills the gap.
- **Player matching**: lower-cased letter-only normalize + suffix-strip retry (Jr / Sr / II / III / IV). On collision, prefer the candidate whose seed `team_id` matches ESPN's reported team. Unmatched names log `tracing::warn!` and skip — never panic.
- **Injury text mapping**: `Out` → `LongTerm` 30 games. `Out For Season` → `SeasonEnding`, games_remaining = unplayed schedule count. `Day-To-Day` / `GTD` / `Questionable` → `DayToDay` 1 game. Other text → no injury record.
- **Phase derivation**: `today >= cal.end_date` → `Playoffs`. `today >= cal.trade_deadline` → `TradeDeadlinePassed`. `day <= PRESEASON_LAST_DAY` → `PreSeason`. Otherwise → `Regular`.
- **Offline fail-loud**: pre-flight `HEAD https://site.api.espn.com/.../teams` with 5 s timeout. Failure aborts before any disk write. Any later error inside `run_import` removes the half-written `.db` (+ `-wal` + `-shm`) so retries start clean.
- **News backfill scope**: 30-day window, max 50 entries. Avoids spamming the news feed with everything since October.
- **No new player insertion**. ESPN sometimes lists G-League / two-way players the seed doesn't carry. We log a warning and skip — adding stub players would require synthesizing ratings_json, contracts, and would distort fit/chemistry calc.

## Files touched

- `crates/nba3k-store/migrations/V017__player_season_stats.sql` (new)
- `crates/nba3k-store/src/store.rs` (3 PSS methods)
- `crates/nba3k-store/tests/player_season_stats.rs` (new — 4 tests)
- `crates/nba3k-core/src/player.rs` (`PlayerSeasonStats` struct)
- `crates/nba3k-cli/src/from_today.rs` (new — importer)
- `crates/nba3k-cli/src/main.rs` (`mod from_today`)
- `crates/nba3k-cli/src/cli.rs` (`NewArgs.from_today`)
- `crates/nba3k-cli/src/commands.rs` (`cmd_new` branch + made `DEFAULT_SEED_PATH` `pub(crate)`)
- `crates/nba3k-cli/src/tui/screens/new_game.rs` (NewArgs init)
- `crates/nba3k-cli/Cargo.toml` (`nba3k-scrape`, `rusqlite`, `reqwest`)

## Verification

```bash
cargo test --workspace                                            # 319 passed + 1 ignored
cargo run --bin nba3k -- --save /tmp/legacy.db new --team BOS     # legacy still works
cargo run --bin nba3k -- --save /tmp/today.db new --team BOS --from-today
# created live save /tmp/today.db (team=BOS mode=standard from-today)
#   teams_loaded=30 games_played=1231 games_unplayed=4 players_with_stats=391
#   injuries_marked=98 roster_moves_applied=143 news_backfilled=50
./target/debug/nba3k --save /tmp/today.db roster LAL | head -20
# LAL roster (16 players): Luka Dončić, LeBron James, Deandre Ayton,
# Austin Reaves [INJ:30], Rui Hachimura, Luke Kennard, Jaxson Hayes, ...
./target/debug/nba3k --save /tmp/today.db standings | head -8
#  1  OKC  West  Northwest  64  18  0.780
#  3  DET  East  Central    60  22  0.732
#  4  BOS  East  Atlantic   56  26  0.683
```

## Known polish items (M33 candidates)

- `cmd_records --scope season --stat ppg` falls back to box-score aggregate which is empty after a `--from-today` import. Rewire to consult `player_season_stats` when game logs are sparse.
- Cup table (`cup_match`) not backfilled — players entering after Cup rounds show no Cup history.
- Per-player box scores for past games are absent — `recap`-style commands won't surface top scorers from completed games.
- Date drift: ESPN tags games by UTC. West-coast night games can land on the next calendar day relative to local. Total game count shows ~1235 instead of canonical 1230 because of duplicate-date inserts on edge cases — harmless for sim, cosmetic.
