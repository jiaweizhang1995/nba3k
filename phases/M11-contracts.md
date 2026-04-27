# M11 — Contracts + Retirement + Sim Calibration

## Scope

3 parallel workers. Crate-ownership split.

## Pre-locked CLI (DO NOT EDIT)

```
Command::Cap { team, json }     → cmd_cap        (worker-a)
Command::Retire { player }      → cmd_retire     (worker-b)
```

Stub bodies in `crates/nba3k-cli/src/commands.rs` already bail.

## Worker A — Contracts + cap surfacing

**Owned crates**: `crates/nba3k-models/` (new `contract_gen.rs`), `crates/nba3k-cli/src/commands.rs` (body of `cmd_cap` + extend `cmd_fa_sign` to generate a contract).

**Goal**: real salary contracts on every roster player. FA `sign` generates a contract scaled to OVR. Cap status visible via new `cap [team]` command.

1. **Contract generation function** in `nba3k-models::contract_gen`:
   - `pub fn generate_contract(player: &Player, season: SeasonId) -> Contract`
   - OVR ≥ 90 → 4-yr max ($55M/yr)
   - OVR 85-89 → 3-4yr ~$30M/yr
   - OVR 80-84 → 3yr ~$15M/yr
   - OVR 70-79 → 2-3yr ~$5M/yr
   - OVR < 70 → 1-2yr veteran-min (~$2M/yr)
   - Use existing `Contract { years: Vec<ContractYear> }` shape from core.

2. **Wire into `cmd_fa_sign`** (modify worker-c's M10 body): when signing a free agent, call `generate_contract` and persist via `Store::upsert_player`.

3. **`Store::team_salary(team, season) -> Cents`** — sum contracts across roster.

4. **`cmd_cap` body**:
   - Resolve team (default user_team).
   - For current season, sum salaries.
   - Compare against `LeagueYear` constants (cap, luxury_tax, first_apron, second_apron).
   - Print:
   ```
   BOS salary cap (2025-26):
     payroll:        $148.2M
     cap:            $140.6M  ($7.6M over)
     luxury tax:     $170.8M  ($22.6M under)
     first apron:    $178.1M  ($29.9M under)
     second apron:   $189.5M  ($41.3M under)
   roster size: 16
   ```
   - JSON variant returns the same data structured.

Tests in `crates/nba3k-models/tests/contract_gen.rs`:
- OVR 95 player gets ≥ $40M/yr.
- OVR 65 player gets veteran-min.
- Contract length scales with age (younger → longer years).

## Worker B — Retirement engine

**Owned crates**: `crates/nba3k-models/` (new `retirement.rs`), `crates/nba3k-cli/src/commands.rs` (body of `cmd_retire` + hook into `cmd_season_advance`).

**Goal**: aging players naturally retire. League stays at ~530 active players via the draft pipeline filling the gap.

1. **Retirement decision function** in `nba3k-models::retirement`:
   - `pub fn should_retire(player: &Player, mins_played_this_season: u32) -> bool`
   - Hard retirement at age ≥ 41.
   - Conditional retirement: age ≥ 36 AND (OVR < 70 OR minutes < 800/season).
   - Stochastic at age 39-40: 50% chance.

2. **Hook into `cmd_season_advance`**: after the progression pass, walk every active player; call `should_retire`; if true, mark retired (set `team_id = NULL` AND a new `is_retired = 1` flag on `players` — V007 migration).

3. **Surface retirements**: `cmd_messages` already exists — add a "retirements" section listing names that retired this off-season. Or, simpler, print a one-line summary at end of `season-advance`:
   ```
   advanced to season 2027 — progressed 530 players, 30 drafted, 18 retired
   ```

4. **Manual retire CLI**: `cmd_retire` — find player, mark retired regardless of age. Useful for God mode.

5. **`Store::list_retired_players() -> Vec<Player>`** for future HOF UI.

Tests in `crates/nba3k-models/tests/retirement.rs`:
- 41yo retires regardless of stats.
- 38yo OVR-90 with 2500 min/season does not retire.
- 37yo OVR-65 with 600 min retires.

## Worker C — Sim FG% calibration

**Owned crates**: `crates/nba3k-sim/src/engine/statistical.rs` and existing tests.

**Goal**: per-player FG% in box scores lands in NBA-realistic range (.42-.55 for shooters; .55+ for rim-runners). Currently a player can hit .887, which the QA flagged.

1. **Read current FG% logic** in `statistical.rs`. Likely `fg_made` is sampled via Normal centered on the player's three_point/mid_range rating. Adjust so:
   - Final `fg_pct` per game ≈ Normal(player_efficiency, 0.05) clamped [0.30, 0.65].
   - `player_efficiency` = composite of three_point + mid_range + finish (close_shot/driving_layup), weighted by usage profile.

2. Verify by re-simming a season and grepping a few stars (Curry, Jokić, SGA). Curry FG% should be ~.45-.50, Jokić ~.55-.60.

3. Tests in `crates/nba3k-sim/tests/`: existing 5-test sim suite must stay green. Add 1 new test that asserts a 100-game sample for an OVR-90 player has FG% mean in [.40, .60].

## Acceptance

```bash
rm -f /tmp/m11.db
./target/release/nba3k --save /tmp/m11.db new --team BOS
./target/release/nba3k --save /tmp/m11.db cap BOS

# Worker A: FA sign now generates a contract
./target/release/nba3k --save /tmp/m11.db fa cut "Sam Hauser"
./target/release/nba3k --save /tmp/m11.db fa sign "Sam Hauser"
./target/release/nba3k --save /tmp/m11.db cap BOS  # payroll changed

# Worker B: retirement
./target/release/nba3k --save /tmp/m11.db sim-to playoffs
./target/release/nba3k --save /tmp/m11.db season-advance  # prints "X retired"
./target/release/nba3k --save /tmp/m11.db retire "LeBron James"

# Worker C: realistic FG%
./target/release/nba3k --save /tmp/m11.db career "Stephen Curry"  # FG% in [.40, .55]
```

## Working agreements

- Each worker keeps `cargo test --workspace` green at every commit boundary.
- DO NOT touch `crates/nba3k-cli/src/cli.rs`.
- TaskUpdate completed + send `team-lead` "done — N files, M tests" + go idle.

## Decision log
