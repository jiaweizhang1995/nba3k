# M35 — "Start From Today" snapshot semantics (match NBA 2K)

**Status**: ✅ Done
**Started**: 2026-04-29
**Completed**: 2026-04-29

## Goal

Tighten `from_today.rs` to match NBA 2K MyNBA "Start Today"'s actual behavior — a *snapshot* of the league's current state, not a historical replay.

NLSC forum, paraphrasing NBA 2K's documented behavior:

> "Start Today only provides the starting point, with no other real results
> used after that; it's all you, at your own pace. ... you just have the
> most recent stats to start your game with."

The user sims forward from today; nothing real-life happens *after* save creation. That same logic should apply to data *before* save creation: standings + rosters + season-to-date stats are loaded as a snapshot, but the per-game history of the season-up-to-now is not preserved (the user already lived it, and 2K does not surface it either).

## Behavior changes

| Data | M34 | M35 |
|---|---|---|
| Standings (W-L per team) | Imported from ESPN standings endpoint | Same — kept |
| Current rosters + injuries | Imported from ESPN per-team roster endpoint | Same — kept |
| Season-to-date player stats (PPG / RPG / etc) | Imported into `player_season_stats` | Same — kept |
| **Past played games** (1230 rows in `games` + `schedule` with played=1) | Bulk-imported from ~190 daily scoreboard fetches | **Dropped** |
| **News feed** (last 30 days of trade headlines, capped at 50) | Inserted into `news` | **Dropped** |
| Future schedule (today..end_date) | Imported | Same — kept |

## Why drop past games + news

- Standings W-L already reflects real played games — duplicating per-game rows is redundant.
- Past games carried only minimal box scores (`{home_pts, away_pts}`); no per-player lines. They were cosmetic-at-best — `recap` / per-player history commands were already empty.
- News spam at game-start: the user sees 50 trade headlines they already lived through. Sim from today onward populates the news feed organically.
- Performance: cuts ~190 daily scoreboard fetches from the cold-cache run.

## Trade-offs

- `compare BOS LAL` head-to-head section will show 0 games against each other (was already misleading — past games had empty box scores).
- `recent_news` is empty until sim-day fires — same as a fresh-October save.
- `schedule_total` reflects only games dated today onward, so a save created late in the season can have a small or empty schedule. Phase resolves correctly (`Playoffs` when today > regular-season end), and `playoffs sim` works to drive the rest.

## Files touched

- `crates/nba3k-cli/src/from_today.rs`:
  - `TodayReport` slimmed: dropped `games_played` and `news_backfilled`.
  - Schedule loop now starts at `today` instead of `cal.start_date`. Past dates aren't fetched.
  - Inside the loop: skip games where `g.date < today` (UTC-vs-local edge cases) and skip already-completed games.
  - Removed the `record_game` block that wrote minimal box scores for past games.
  - Removed the `fetch_news_trades` + `record_news` block.
  - Doc comment updated to describe the snapshot model.
  - `GameId` import removed (no more game-row construction here).
- `crates/nba3k-cli/src/commands.rs`:
  - `cmd_new` success message updated to drop `games_played=` and `news_backfilled=` columns.

## Verification

```bash
cargo test --workspace                                       # 320 passed + 2 ignored
cargo build --release --bin nba3k

# Cold cache (cleared data/cache/espn first):
time ./target/release/nba3k --save /tmp/m35.db new --team BOS
# → 45 s — dominated by 30 sequential roster fetches (gate=100ms).
#   Past-game scoreboard loop is gone; remaining cost is per-team roster.
# → teams_loaded=30 games_unplayed=0 players_with_stats=391
#   injuries_marked=98 roster_moves_applied=143

# Warm cache:
time ./target/release/nba3k --save /tmp/m35_warm.db new --team BOS
# → 1.1 s

# Save shape:
sqlite3 /tmp/m35_warm.db "SELECT COUNT(*) FROM games;"     # → 0
sqlite3 /tmp/m35_warm.db "SELECT COUNT(*) FROM schedule;"  # → 0 if season over, else N future games
sqlite3 /tmp/m35_warm.db "SELECT COUNT(*) FROM news;"      # → 0

# Standings + roster still real:
./target/release/nba3k --save /tmp/m35_warm.db standings    # OKC 64-18, BOS 56-26, ...
./target/release/nba3k --save /tmp/m35_warm.db roster LAL   # Doncic + LeBron + Reaves [INJ:30]
```

## Polish items deferred (M36 candidates)

- 30 sequential roster fetches dominate cold-cache wall time. Parallelize with `std::thread::scope` (10-worker pool) to drop ≈45 s → ≈5 s.
- Schedule loop runs `cal.end_date - today` iterations even when the user starts late in the season. When today > end_date, the loop body never executes; the `phase = Playoffs` branch already handles this, but the user could still benefit from a playoff-bracket import (currently unimplemented).
