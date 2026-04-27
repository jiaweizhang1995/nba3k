# M16 — NBA Cup + rumors + roster compare

## Scope

3 parallel workers.

## Pre-locked CLI

```
Command::Cup { season, json }            → cmd_cup       (worker-a)
Command::Rumors { limit, json }          → cmd_rumors    (worker-b)
Command::Compare { team_a, team_b, json }→ cmd_compare   (worker-c)
```

## Worker A — NBA Cup

**Owned crates**:
- `crates/nba3k-store/migrations/V011__cup.sql` (new).
- `crates/nba3k-store/src/store.rs` — `record_cup_match` / `read_cup_bracket`.
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_cup` + day-30 trigger in `sim_n_days`.

**Goal**: at day 30, kick off the NBA Cup. 30 teams split into 6 groups of 5 (3 East groups + 3 West groups). Group stage = 4 games per team (round-robin within group, count toward regular standings as exhibition or pure-cup-only — your call; simplest is cup-only and don't touch standings). Days 30-45 group stage, day 50 quarterfinals (8 teams), day 53 semifinals, day 55 final. Persist all matches; `cmd_cup` shows bracket + winner.

1. **Migration `V011__cup.sql`**:
```sql
CREATE TABLE cup_match (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    season INTEGER NOT NULL,
    round TEXT NOT NULL,         -- "group" | "qf" | "sf" | "final"
    group_id TEXT,               -- "east-A" / "west-C" for group; NULL for KO
    home_team INTEGER NOT NULL,
    away_team INTEGER NOT NULL,
    home_score INTEGER NOT NULL,
    away_score INTEGER NOT NULL,
    day INTEGER NOT NULL
);
CREATE INDEX idx_cup_season_round ON cup_match(season, round);
```

2. **Group split**: deterministic from team list — first 5 East teams = group east-A, next 5 = east-B, etc. Same for West.

3. **Day-30 hook in `sim_n_days`**: if `state.day == 30` AND no cup matches recorded for `state.season`, generate the group-stage round-robin schedule and sim ALL group games at once via the existing `Engine::simulate_game`. Store each match.

4. Day-45 advance: top-2 from each group (8 teams) → quarterfinals (single-elimination). Sim immediately. Continue to SF (day 53) and Final (day 55).

5. **`cmd_cup`** — render full bracket: groups + KO ladder + champion. JSON: structured.

6. News kind=`cup` for the winner.

Tests in `crates/nba3k-store/tests/cup.rs`:
- 6 groups, 5 teams each, group stage produces 60 matches (10 per group × 6).
- KO bracket has 8 → 4 → 2 → 1.

## Worker B — Trade rumors

**Owned crates**:
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_rumors`.
- May add helper to `nba3k-trade` or `nba3k-models`.

**Goal**: surface AI interest signals — "5 teams interested in Player X". Compute heuristically from existing data: teams with cap room targeting players whose cap-tier they can afford and whose archetype fills their need.

1. For each player on every roster:
   - Compute `team_need(player, team)` for every other team (skip own team). Need = high if (player's archetype matches team's biggest weakness) AND (cap-room enough for first-year salary × 1.30).
   - Count teams where `need >= 0.6` → that's the "interest count".
2. Select top-N players ranked by interest count + OVR.
3. Print:
```
Trade rumors (top 20):
RANK  PLAYER             TM   OVR  ROLE     INTEREST  TOP-3 SUITORS
   1  Jaylen Brown       BOS   83  Star          7    DAL, GSW, NYK
   2  ...
```
4. JSON: array of structured rows.

Tests:
- After fresh save, `rumors` returns ≥ 1 row.
- JSON parses.

## Worker C — Roster compare

**Owned crates**:
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_compare`.

**Goal**: side-by-side text + JSON of two teams. Show top-8 rotation, payroll, chemistry score, key headline stats.

1. Resolve both teams.
2. For each team:
   - Top-8 rotation (existing logic in `build_snapshot`).
   - Payroll via `Store::team_salary`.
   - Chemistry via `team_chemistry`.
   - Average rotation OVR.
3. Render side-by-side:
```
                       BOS              LAL
roster size            16               16
top-8 OVR (avg)        81.2             82.5
payroll                $167.4M          $182.6M  (over $15.2M)
chemistry              0.62             0.71

  TOP 8                BOS              LAL
  PG                   Pritchard 81     Reaves 78
  SG                   White 82         Russell 80
  ...
```
4. JSON: structured object with both teams' breakdowns.

Tests in `crates/nba3k-cli/tests/compare_smoke.rs`:
- Compare BOS vs LAL → both columns populate.
- Same-team comparison errors out.

## Acceptance

```bash
rm -f /tmp/m16.db
./target/release/nba3k --save /tmp/m16.db new --team BOS
./target/release/nba3k --save /tmp/m16.db sim-day 30
./target/release/nba3k --save /tmp/m16.db cup     # group stage just finished
./target/release/nba3k --save /tmp/m16.db sim-day 30 # → KO + champion
./target/release/nba3k --save /tmp/m16.db cup
./target/release/nba3k --save /tmp/m16.db rumors --limit 10
./target/release/nba3k --save /tmp/m16.db compare BOS LAL
```

## Working agreements

- DO NOT touch `crates/nba3k-cli/src/cli.rs`.
- `cargo test --workspace` green at every commit boundary.
- TaskUpdate completed + send `team-lead` "done — N files, M tests" + go idle.
