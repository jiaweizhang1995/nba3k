# M31 — Calendar decoupling + ESPN fetch layer

**Status**: ✅ Done
**Started**: 2026-04-29
**Completed**: 2026-04-29

## Goal

Lay the foundation for "Start From Today" without changing user behavior. Two pieces:

1. **Calendar decoupling** — break the load-bearing `SEASON_START` / `SEASON_END` / trade-deadline constants out of `nba3k-season` so per-save calendars can drive schedule + phase math. New table `season_calendar` stores one row per season.
2. **ESPN client** — Rust-native module replacing the failed nba_api shellout attempt. Six fetch+parse pairs cover everything M32's importer needs: teams, standings, daily scoreboards, rosters (with injuries inline), full-league season stats, and trade news.

## Sub-tasks

| # | Item | Status |
|---|------|--------|
| T1 | V016 `season_calendar` migration + `SeasonCalendar` struct + Store API | ✅ |
| T2 | Parameterize `Schedule::generate_with_dates` + calendar-aware phases helpers | ✅ |
| T3 | `nba3k-scrape::sources::espn` — 6 fetchers, fixture-driven tests | ✅ |
| T4 | This phase doc + PHASES.md row | ✅ |

## Decisions

- **Backwards compatibility**: legacy `Schedule::generate(season, seed, teams)` and the const-anchored `is_after_trade_deadline(date)` / `is_trade_deadline_day(date)` still work and use the 2025-26 hardcoded values. New code paths use `Schedule::generate_with_dates(...)` and `is_after_trade_deadline_for(date, &cal)`.
- **`SeasonCalendar::default_for(year)`**: extrapolates from the 2025-26 anchor by 365 days × `(year - 2026)`. Used as fallback when no row exists in the table. M33's `season-advance` will write a real row per year and replace this fallback.
- **ESPN politeness**: separate 100 ms gate from BBRef's 3 s gate (different `Mutex<Option<Instant>>`). ESPN tolerates parallelism within a process; the gate just prevents bursting.
- **Cache TTLs**: 12 h (teams / standings / player_stats), 6 h (scoreboard / roster), 1 h (news).
- **404 handling**: real "no data" answer for off-day scoreboards. `fetch_*` returns `Ok(None)`; caller treats as empty.

## Files touched

- `crates/nba3k-store/migrations/V016__season_calendar.sql` (new)
- `crates/nba3k-core/src/season.rs` (`SeasonCalendar` struct + `default_for`)
- `crates/nba3k-store/src/store.rs` (`get_season_calendar`, `upsert_season_calendar`)
- `crates/nba3k-store/tests/season_calendar.rs` (new)
- `crates/nba3k-season/src/schedule.rs` (`generate_with_dates`)
- `crates/nba3k-season/src/phases.rs` (`trade_deadline`, `is_after_trade_deadline_for`, `is_trade_deadline_day_for`)
- `crates/nba3k-season/src/lib.rs` (re-exports)
- `crates/nba3k-season/tests/schedule_tests.rs` (custom-window test + calendar deadline test)
- `crates/nba3k-scrape/src/sources/espn.rs` (new — 6 fetchers + 6 parsers + types)
- `crates/nba3k-scrape/src/sources/mod.rs` (`pub mod espn`)
- `crates/nba3k-scrape/tests/espn_parse.rs` + `tests/fixtures/espn/*.json` (new)
- `crates/nba3k-cli/src/commands.rs` (`season_calendar_or_default` helper; `generate_and_store_schedule` reads calendar; sim-day deadline check uses calendar)

## Verification

```bash
cargo test -p nba3k-store --test season_calendar
cargo test --workspace                         # 309 passed + 1 ignored
cargo run --bin nba3k -- --save /tmp/legacy.db new --team BOS
sqlite3 /tmp/legacy.db "SELECT MIN(date), MAX(date), COUNT(*) FROM schedule;"
# → 2025-10-21 | 2026-04-12 | 1230  (legacy path byte-identical)
sqlite3 /tmp/legacy.db "SELECT * FROM season_calendar;"
# → 2026|2025-10-21|2026-04-12|2026-02-05|41|30|45|53|55  (V016 seeded automatically)
cargo test -p nba3k-scrape --test espn_parse  # 6 passed
```

## Known gaps for M32

- ESPN's player-stats endpoint exposes per-game shooting splits but not USG / TS%. M32's importer leaves `usage = 0.0` and `ts_pct = 0.0` — downstream features should tolerate this.
- Date conversion: ESPN tags games with UTC timestamps, so a West-coast night game can land on the next calendar day. M32's date loop must be ±1 day tolerant.
