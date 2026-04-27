# M10 — Post-MVP stretch

## Scope

User said: skip possession-by-possession sim. Build everything else.

4 parallel workers, crate-ownership split. Lead pre-locks CLI surface, integrates, runs M10 acceptance.

## Pre-locked CLI (DO NOT EDIT in workers)

```
Command::Career { name, json }                      → cmd_career
Command::Fa(FaAction::{List|Sign|Cut})              → cmd_fa_*
Command::Training { player, focus }                 → cmd_training
TradeAction::Propose3 { leg, json }                 → cmd_trade_propose3
```

Stub handlers in `crates/nba3k-cli/src/commands.rs` already bail with `not yet implemented`. Replace the bodies, do not change the signatures.

## Worker A — 3-team trades

**Owned crates**: `crates/nba3k-trade/`, plus body of `cmd_trade_propose3`.

**Goal**: extend the trade engine so `assets_by_team.len() == 3` works end-to-end.

The original M3 plan said `v1 hard-asserts == 2`. Find and lift that assert. Then:

1. Each of the 3 teams independently runs `evaluate::evaluate(&offer, that_team, &snap, &mut rng)`.
2. **Unanimous Accept** required to actually fire (any single Counter or Reject from any leg → the whole thing falls through).
3. Use the existing `apply_accepted_trade` round-robin player mover (already in `crates/nba3k-cli/src/commands.rs:1063`-ish) — it already handles `len() ≥ 2`.

CLI body for `cmd_trade_propose3`:
- Parse each `--leg` arg as `ABBR:player1,player2,...` (split on `:` then `,`).
- Build a 3-entry `IndexMap<TeamId, TradeAssets>`.
- Call `evaluate` once per team, require all Accept, then fire the trade and persist via `insert_trade_chain`.

Tests in `crates/nba3k-trade/tests/three_team_trades.rs`:
- 3-team "balance" trade where all 3 sides get peer-OVR — should accept.
- 3-team trade where one side is dumping a bad contract — that side should counter or reject.

## Worker B — Dynasty career stats

**Owned crates**: `crates/nba3k-store/` (migration + reads), `crates/nba3k-season/` (career aggregation), body of `cmd_career`.

**Goal**: aggregate per-season `games` rows into per-player career totals across all simulated seasons. Show GP / PPG / RPG / APG / SPG / BPG / FG% / 3P% / FT% per season + career totals.

1. **No new table needed** — `games` table already has full box scores per game in `box_score_json`. Just aggregate at read time.
2. New Store API: `read_career_stats(player_id) -> Vec<SeasonAvgRow>` — walk all `games` rows, parse box scores, sum per player per season.
3. CLI `cmd_career(player_name)` text:

```
Stephen Curry — career
SEASON   TM    GP   PPG  RPG  APG  SPG  BPG  FG%   3P%   FT%
2025-26  GSW   72   28.4 4.5  6.3  1.2  0.4  .465  .405  .908
2026-27  GSW   75   27.1 4.2  6.8  1.1  0.3  .448  .398  .912
career         147  27.7 4.4  6.6  1.2  0.4  .456  .402  .910
```

4. JSON variant returns the same data as a structured object.

Tests in `crates/nba3k-season/tests/career_stats.rs`:
- 2-season aggregate; assert PPG = total_pts / total_gp.
- Season-over-season ordering by SeasonId asc.
- Player who switched teams shows the correct TM column per season.

## Worker C — Free agency v2

**Owned crates**: `crates/nba3k-store/` (FA pool semantics), bodies of `cmd_fa_list`/`cmd_fa_sign`/`cmd_fa_cut`.

**Goal**: turn `team_id IS NULL` players (currently used only for prospects) into a true free-agent pool. Cut a player → goes to FA. Sign a free agent → joins user's team.

1. Augment `players` schema check: prospects vs FA distinguished by `dev_json` content (`"draft_class": <year>`) — prospects keep theirs; cut players have `null` or no `draft_class`. If unclear, add a `is_free_agent` column via V006 migration.
2. **Roster size guard**: refuse `fa sign` when user team already has 18 players (CBA cap).
3. CLI:
   - `fa list` → top-30 free agents by OVR, text + JSON.
   - `fa sign "Player"` → assign to user team, set role `RolePlayer`, morale `0.5`.
   - `fa cut "Player"` → remove from user team's roster.

Tests in `crates/nba3k-store/tests/fa_pool.rs`:
- Cut a player → `roster_for_team` shrinks by 1, `list_prospects` (or new `list_free_agents`) includes them.
- Sign-then-cut round-trips cleanly.
- `fa sign` over the 18-player cap returns a clean error.

## Worker D — Training camp / dev points

**Owned crates**: `crates/nba3k-models/` (progression module already has `apply_progression_step`), body of `cmd_training`.

**Goal**: 2K MyGM-style "training camp" — once per off-season, the user picks a focus area for one player and gets a deterministic +1..3 attribute bump in that bucket.

1. Map `focus` argument → attribute cluster:
   - `shoot` → mid_range/three_point/free_throw
   - `inside` → close_shot/driving_layup/post_control
   - `def` → interior_defense/perimeter_defense/steal/block
   - `reb` → off_reb/def_reb
   - `ath` → speed/agility/vertical/strength
   - `handle` → passing_accuracy/ball_handle/speed_with_ball
2. Apply +2 to the highest current attribute in the cluster, +1 to the rest. Cap at 99.
3. Track a per-season "training used" flag on the team via `meta` table key like `training_used:<season>:<team_id>` — refuse second use in same season.
4. Persist mutated player via `Store::upsert_player`.
5. New unit tests in `crates/nba3k-models/tests/training_focus.rs` check the cluster mapping is deterministic and bumps the right attributes.

CLI text:
```
$ nba3k --save x.db training "Jayson Tatum" shoot
Tatum: shoot training applied (+2 mid_range, +2 three_point, +1 free_throw). New OVR: 84.
```

## Acceptance

```bash
# Setup
rm -f /tmp/m10.db
./target/release/nba3k --save /tmp/m10.db new --team BOS

# Worker A: 3-team
./target/release/nba3k --save /tmp/m10.db trade propose3 \
    --leg "BOS:Sam Hauser" \
    --leg "LAL:LeBron James" \
    --leg "DAL:Brandon Williams"

# Worker B: career
./target/release/nba3k --save /tmp/m10.db sim-to playoffs
./target/release/nba3k --save /tmp/m10.db season-advance
./target/release/nba3k --save /tmp/m10.db sim-to playoffs
./target/release/nba3k --save /tmp/m10.db career "Stephen Curry"  # 2-season table

# Worker C: FA
./target/release/nba3k --save /tmp/m10.db fa list
./target/release/nba3k --save /tmp/m10.db fa cut "Sam Hauser"
./target/release/nba3k --save /tmp/m10.db fa sign "Sam Hauser"

# Worker D: training
./target/release/nba3k --save /tmp/m10.db training "Jayson Tatum" shoot
./target/release/nba3k --save /tmp/m10.db player "Jayson Tatum"
```

## Working agreements

- Each worker keeps `cargo test --workspace` green at every commit boundary.
- Workers MUST NOT touch `crates/nba3k-cli/src/cli.rs` — only `commands.rs` body of their stub handler.
- When done, mark task completed via TaskUpdate AND send `team-lead` a one-line "done — {N} files touched, {M} tests added" message. Then go idle.
- No commits — lead does final aggregate.

## Decision log
