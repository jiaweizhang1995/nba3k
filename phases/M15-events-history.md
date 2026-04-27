# M15 — Events + history + save management

## Scope

3 parallel workers:
- A: All-Star weekend (auto-trigger at day 41, view roster)
- B: Standings history (per-season recall via `--season N`)
- C: Save management (list/show/delete saves)

## Pre-locked CLI

```
Command::AllStar { season, json }                  → cmd_all_star    (worker-a)
Command::Saves(SavesAction::List|Show|Delete)      → cmd_saves_*     (worker-c)
Command::Standings { season, json }                → cmd_standings   (worker-b extends)
```

Stubs in `crates/nba3k-cli/src/commands.rs` already bail.
`cmd_standings` already takes `season_arg: Option<u16>` per the lead's pre-lock.

## Worker A — All-Star weekend

**Owned crates**:
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_all_star`, hook into `sim_n_days` to trigger at day 41.
- `crates/nba3k-store/src/store.rs` — persist all-star roster per season (new table `all_star` via V010 migration).

**Goal**: at the day-41 mid-season marker, run the existing `compute_all_star` from `nba3k_season::awards`, persist the roster, surface via `cmd_all_star`. Print news.

1. **Migration `V010__all_star.sql`**:
```sql
CREATE TABLE all_star (
    season INTEGER NOT NULL,
    conf TEXT NOT NULL,        -- "East" | "West"
    player_id INTEGER NOT NULL REFERENCES players(id),
    role TEXT NOT NULL,        -- "starter" | "reserve"
    PRIMARY KEY (season, player_id)
);
```

2. Store API:
   - `record_all_star(season, conf, player_id, role)`.
   - `read_all_star(season) -> Vec<(Conference, role: String, PlayerId)>`.

3. Hook in `sim_n_days`: if today's `state.day == 41` (or first day at/after 41 if user crossed it), AND no all-star roster recorded yet for `state.season`, run:
   - `nba3k_season::compute_all_star(aggregate, standings, season, position_of, conference_of)`.
   - `compute_all_star` returns `AllStarRoster { east_starters, east_reserves, west_starters, west_reserves }`. Persist each via `record_all_star`.
   - Print one-line news entry kind=`all_star` (worker-b's record_news from M13).

4. `cmd_all_star`:
   - Resolve season (default current).
   - Read `all_star` rows.
   - Print:
     ```
     2025-26 All-Star — East starters
       PG  Trae Young (ATL)
       ...
     East reserves
       ...
     West starters
       ...
     ```
   - JSON: `{"east_starters": [...], "east_reserves": [...], "west_starters": [...], "west_reserves": [...]}`.

Tests in `crates/nba3k-cli/tests/all_star_smoke.rs`:
- Sim 41 days → all-star roster recorded.
- `cmd_all_star --season 2026 --json` parses.

## Worker B — Standings history

**Owned crates**:
- `crates/nba3k-cli/src/commands.rs` — extend `cmd_standings` to accept any past season (dispatcher already passes `season_arg`).
- `crates/nba3k-cli/src/commands.rs` — extend `cmd_season_advance` to ensure final standings persist for the just-finished season before rolling state.
- New `crates/nba3k-cli/tests/standings_history.rs`.

**Goal**: `nba3k --save x.db standings --season 2026` recalls last season's final standings, even after `season-advance` rolled to 2027.

1. The `standings` table already has a season column. Verify `Store::read_standings(season)` filters correctly. (It already does — confirm.)
2. `cmd_season_advance` already calls `rebuild_standings(app, state.season)` before rolling state — verify and add the call if missing. Once the new season starts, rebuild standings for `next_season` from games as they're played, but the prior season's row stays untouched.
3. Test: sim two seasons, assert `standings --season 2026` and `standings --season 2027` return distinct results.

Light scope — this is mostly verification + a smoke test.

## Worker C — Save management

**Owned crates**:
- `crates/nba3k-cli/src/commands.rs` — bodies of `cmd_saves_list/show/delete`.
- New tests in `crates/nba3k-cli/tests/saves_smoke.rs`.

**Goal**: list / inspect / delete `.db` save files. Useful for users juggling multiple campaigns.

1. **`cmd_saves_list`**:
   - Default scan dir: current dir + `/tmp` (skip `/tmp` if not on Unix).
   - For each `*.db` file, attempt `Store::open` (read-only — close after metadata read).
   - On success: read `meta` table for `app_version`, `season`, `created_at`, `user_team` (from `meta`).
   - Print:
     ```
     Save files:
       /tmp/m14.db        team=BOS season=2026 created=2026-04-26
       /tmp/run.db        team=LAL season=2027 created=2026-04-25
     ```
   - JSON variant: array of structured rows.

2. **`cmd_saves_show <path>`**:
   - Open path, dump status-like info: team, season, phase, day, version, schedule games, players. Reuse the existing `cmd_status` helpers if accessible.

3. **`cmd_saves_delete <path> --yes`**:
   - Refuse without `--yes`.
   - With `--yes`, `std::fs::remove_file(path)`. Print confirmation.
   - Refuse if `path` matches the currently-open save (the lead's `app.save_path`).

Tests in `crates/nba3k-cli/tests/saves_smoke.rs`:
- Create a temp save, `saves show <path>` returns parseable info.
- `saves delete` without `--yes` errors out helpfully.
- `saves delete <path> --yes` actually removes the file.

## Acceptance

```bash
rm -f /tmp/m15.db
./target/release/nba3k --save /tmp/m15.db new --team BOS

# Worker A: all-star
./target/release/nba3k --save /tmp/m15.db sim-day 41
./target/release/nba3k --save /tmp/m15.db all-star --json
./target/release/nba3k --save /tmp/m15.db news --limit 5  # all-star entry

# Worker B: standings history
./target/release/nba3k --save /tmp/m15.db sim-to playoffs
./target/release/nba3k --save /tmp/m15.db standings --season 2026  # current
./target/release/nba3k --save /tmp/m15.db season-advance
./target/release/nba3k --save /tmp/m15.db standings --season 2026  # historical
./target/release/nba3k --save /tmp/m15.db standings --season 2027  # current

# Worker C: save mgmt
./target/release/nba3k saves list
./target/release/nba3k saves show /tmp/m15.db
./target/release/nba3k saves delete /tmp/scratch.db --yes
```

## Working agreements

- Each worker keeps `cargo test --workspace` green at every commit boundary.
- DO NOT touch `crates/nba3k-cli/src/cli.rs`.
- TaskUpdate completed + send `team-lead` "done — N files, M tests" + go idle.
