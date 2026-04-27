# M13 — League liveness

## Scope

Make the league feel alive between commands: injuries take players out, news feed records what's happening, mid-season award race shows leaders.

3 parallel workers, crate-ownership split.

## Pre-locked CLI

```
Command::AwardsRace(JsonFlag)        → cmd_awards_race    (worker-c)
Command::News { limit, json }        → cmd_news           (worker-b)
```

(no new CLI for injuries — they surface in `roster` / `player` / `messages`)

## Worker A — Injuries

**Owned crates**: `crates/nba3k-sim/src/engine/statistical.rs` (injury rolls during sim), `crates/nba3k-store/src/store.rs` (read/write `injury_json` already exists), `crates/nba3k-cli/src/commands.rs` (extend `roster` / `player` / `messages` to show injuries).

**Goal**: every game, ~1-2% chance per player to suffer an injury (DayToDay = 1-3 games, ShortTerm = 5-15 games, LongTerm = 20-50). Injured players don't appear in rotation; their team uses next-up bench. Decrement games_remaining each sim-day.

1. **Injury roll in sim**:
   - In `simulate_game`, after the box score is generated, for each player in rotation: roll RNG. With probability scaled by minutes (more mins → more injury risk), roll an `InjurySeverity` (DayToDay 70%, ShortTerm 25%, LongTerm 5%).
   - Set `player.injury = Some(InjuryStatus { ... })`.
   - Persist via `Store::upsert_player`.

2. **Decrement on each sim-day**: in `sim_n_days`, before simming today's games, walk all injured players, decrement `games_remaining`; clear injury when reaches 0.

3. **Rotation skip**: in `build_snapshot` (CLI), filter out players with active injuries. Already exists? Verify and add filter if missing.

4. **Surface in CLI**:
   - `roster` text: prefix injured players with `*` and add INJ column. JSON: `injury` field.
   - `player` text: if injured, show "INJURED: <description>, <X> games out".
   - `messages` (M9 worker): add an "injuries" section listing all injured players on user team.

Tests in `crates/nba3k-sim/tests/injuries.rs`:
- 100-game sim with a fixed seed produces ≥ 5 injuries (loose).
- An injured player's `games_remaining` decrements per sim-day.

## Worker B — News feed

**Owned crates**: `crates/nba3k-store/src/store.rs` + new `V008__news.sql` migration, body of `cmd_news` in `crates/nba3k-cli/src/commands.rs`.

**Goal**: append a row to a `news` table whenever a state-mutating event fires (trade accepted, FA signed, FA cut, retirement, draft pick, injury, award). Surface via `news --limit N`.

1. **Migration `V008__news.sql`**:
```sql
CREATE TABLE news (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    season INTEGER NOT NULL,
    day INTEGER NOT NULL,
    kind TEXT NOT NULL,        -- "trade" | "signing" | "cut" | "retire" | "draft" | "injury" | "award"
    headline TEXT NOT NULL,
    body TEXT
);
CREATE INDEX idx_news_season_day ON news(season, day);
```

2. **Store API**:
   - `Store::record_news(season, day, kind, headline, body)`.
   - `Store::recent_news(limit) -> Vec<NewsRow>`.

3. **Hook into the existing flows** (read-only — don't change their logic, just add a `record_news` call after the success path):
   - `cmd_fa_sign` → `signing` kind.
   - `cmd_fa_cut` → `cut` kind.
   - `cmd_retire` → `retire` kind.
   - `apply_accepted_trade` → `trade` kind.
   - `cmd_season_advance` after draft → one `draft` row per pick OR one summary row.
   - Awards: `cmd_awards` writes one row per award winner.

4. **`cmd_news` body**: read recent N, render text:
```
Recent league news (last 30):
  S2026 D45  [trade]    BOS sends Sam Hauser to LAL for LeBron James
  S2026 D60  [injury]   Jayson Tatum (BOS) — strained hamstring, ~6 games
  S2026 D82  [signing]  GSW signs Demarcus Cousins (3yr, $12M/yr)
  ...
```
JSON variant emits structured array.

Tests in `crates/nba3k-cli/tests/news_smoke.rs`:
- After a trade, `news` shows it.
- After season-advance with retirements, `news` includes them.

## Worker C — Award race tracker

**Owned crates**: body of `cmd_awards_race` in `crates/nba3k-cli/src/commands.rs`. May extend `nba3k-season::awards` if needed (read-only — don't change existing computation).

**Goal**: at any point during regular season, run the existing awards aggregation pipeline (`compute_all_awards`) using games-played-so-far and surface the top-5 ballot per award.

1. Read games for current season (via `Store::read_games(state.season)`, filter `is_playoffs = false`).
2. Build aggregate via `aggregate_season(&games)`.
3. Build standings via `Standings::new(&teams)` + `record_game_result`.
4. Call `compute_mvp(&aggregate, &standings, season)` etc. Each returns an `AwardResult` whose `ballot` is `Vec<(PlayerId, share: f32)>`.
5. Print top-5 per award with current vote share %:
```
Award race — through day 45 of 2025-26 (mid-season check):
  MVP             1. Jokić (DEN, 31.2 PPG)    32%
                  2. SGA (OKC, 30.8 PPG)      28%
                  3. Luka (DAL, 33.0 PPG)     19%
                  4. Tatum (BOS, 28.4 PPG)    11%
                  5. Curry (GSW, 29.1 PPG)     8%
  DPOY            1. Gobert (MIN)             ...
```

JSON variant emits `{award: [{rank, player_id, name, team, ppg, share}, ...]}`.

Tests in `crates/nba3k-cli/tests/awards_race_smoke.rs`:
- After 30 games simmed, `awards-race` returns non-empty MVP ballot.
- Top entry's vote share > all others.

## Acceptance

```bash
rm -f /tmp/m13.db
./target/release/nba3k --save /tmp/m13.db new --team BOS

# Worker A: injuries
./target/release/nba3k --save /tmp/m13.db sim-day 30
./target/release/nba3k --save /tmp/m13.db roster BOS  # at least one * INJ entry
./target/release/nba3k --save /tmp/m13.db messages    # injury section

# Worker C: award race
./target/release/nba3k --save /tmp/m13.db awards-race  # MVP/DPOY/ROY/6MOY/MIP top-5

# Worker B: news
./target/release/nba3k --save /tmp/m13.db news --limit 10
./target/release/nba3k --save /tmp/m13.db trade propose --from BOS --to LAL --send "Sam Hauser" --receive "LeBron James" --json
./target/release/nba3k --save /tmp/m13.db news --limit 5  # includes trade
```

## Working agreements

- Each worker keeps `cargo test --workspace` green at every commit boundary.
- DO NOT touch `crates/nba3k-cli/src/cli.rs`.
- TaskUpdate completed + send `team-lead` "done — N files, M tests" + go idle.
