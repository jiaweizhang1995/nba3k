# M14 — Meta-game depth

## Scope

Coaching, scouting fog, league records. Three workers, crate-ownership split.

## Pre-locked CLI

```
Command::Coach(CoachAction::Show|Fire|Pool)        → cmd_coach_*    (worker-a)
Command::Scout { player }                          → cmd_scout      (worker-b)
Command::Records { scope, stat, limit, json }      → cmd_records    (worker-c)
```

Stub bodies in `crates/nba3k-cli/src/commands.rs` already bail.

## Worker A — Coaching

**Owned crates**: `crates/nba3k-core/src/coach.rs` (extend), `crates/nba3k-store/src/store.rs` (read/write helpers — `Coach` is already inside `Team`), `crates/nba3k-cli/src/commands.rs` (bodies of `cmd_coach_show/fire/pool`).

**Goal**: hire/fire coaches, see traits, view a generated free-agent coach pool.

1. **Coach trait scoring**: existing `CoachAxes { strategy, leadership, mentorship, knowledge, team_management }` (0..99). Add `Coach::overall() -> u8` averaging axes.
2. **Hot-seat metric**: a coach with ovr < 65 → "on the hot seat" warning in `coach show`.
3. **`cmd_coach_show`**:
   - Resolve team (default user_team).
   - Print:
     ```
     BOS coach: Joe Mazzulla (overall 78)
       schemes: PaceAndSpace / Defense
       axes: strategy 82 / leadership 75 / mentorship 70 / knowledge 80 / team_management 84
     ```
   - JSON variant returns the structured object.
4. **`cmd_coach_fire`**:
   - Fire current coach. Generate replacement from a deterministic per-team pool (use a fresh `Coach::default_for(abbrev)` perturbed by season/seed).
   - Persist via `Store::upsert_team`.
   - Record news kind=`coach`. Print: `fired Joe Mazzulla; hired Wilson Tillis (overall 71)`.
5. **`cmd_coach_pool`**:
   - Generate ~12 candidate coaches deterministically (use a hash of season + i). Show their overall + schemes.
   - Future-extension: actually swap in a pool member. v1 just lists.

Tests in `crates/nba3k-core/tests/coach.rs` (or extend an existing one):
- `Coach::overall()` averages axes.
- `default_for` produces stable schemes from same abbrev.
- Hot-seat threshold matches docs.

## Worker B — Scouting fog

**Owned crates**: `crates/nba3k-store/src/store.rs` + new V009 migration, `crates/nba3k-cli/src/commands.rs` (extend `cmd_draft_board` to mask un-scouted, body of `cmd_scout`).

**Goal**: prospect ratings (overall, potential, full Ratings) hidden until the user "scouts" them.

1. **Migration `V009__player_scouted.sql`**:
```sql
ALTER TABLE players ADD COLUMN scouted INTEGER NOT NULL DEFAULT 0;
```
   Real NBA players (team_id IS NOT NULL) keep `scouted = 0` but it doesn't matter for them — only prospects use it. Set existing prospects to `scouted = 0` by default.

2. **`cmd_draft_board`** (existing handler) now hides un-scouted prospect data. Show name, age, position, but render `???` for OVR/POT/Ratings unless `scouted = 1`.

3. **`cmd_scout`**:
   - Resolve prospect by name (must have `team_id IS NULL` and not retired).
   - Set `scouted = 1`.
   - Print: `scouted Cooper Flagg — OVR 83, POT 91, archetype "PG-distributor"`.
   - Charges nothing (no scouting budget yet — could add a meta key `scouts_used:<season>` cap of 5/season for v1).

4. **Hide draft board mock-rank ordering** when un-scouted: rank by name alphabetically until scouted, otherwise rank by potential.

Tests in `crates/nba3k-store/tests/scouting.rs`:
- Default prospects are un-scouted.
- After scout, `scouted = 1`.
- `list_prospects` (or new `list_prospects_visible`) returns scouted ratings populated and un-scouted with placeholder values.

## Worker C — Records leaderboards

**Owned crates**: `crates/nba3k-cli/src/commands.rs` (body of `cmd_records`); may add helper to `crates/nba3k-season/` if needed.

**Goal**: `nba3k records --scope season --stat ppg --limit 10` shows current-season scoring leaders.

1. **Scope `season`**: read current-season `games`, aggregate per-player like awards-race, return top-N for the requested stat. Compute PPG as `pts / games_played`.
2. **Scope `career`**: aggregate ALL seasons' games. Sum totals, then compute career averages.
3. **Stat options**: `ppg`, `rpg`, `apg`, `spg`, `bpg`, `three_made`, `fg_pct`. Reject unknown stat with helpful error.
4. **Output text**:
   ```
   Records — 2025-26 season, top 10 PPG (min 20 GP):
   RANK  NAME              TM    GP  PPG
      1  Stephen Curry     GSW   45  29.3
      2  Devin Booker      PHO   42  28.7
      ...
   ```
5. **JSON variant**: array of structured rows.
6. **Min-GP filter**: 20 GP for season scope, 100 GP for career scope. Avoids 1-game outliers gaming the table.

Tests in `crates/nba3k-cli/tests/records_smoke.rs`:
- After 30-day sim, `records --scope season --stat ppg` returns top-N with valid teams + averages.
- Unknown stat → error.
- Career on fresh save → empty (no games).

## Acceptance

```bash
rm -f /tmp/m14.db
./target/release/nba3k --save /tmp/m14.db new --team BOS

# Worker A: coaching
./target/release/nba3k --save /tmp/m14.db coach show
./target/release/nba3k --save /tmp/m14.db coach fire

# Worker B: scouting fog
./target/release/nba3k --save /tmp/m14.db draft board   # OVR/POT shown as ???
./target/release/nba3k --save /tmp/m14.db scout "Cooper Flagg"
./target/release/nba3k --save /tmp/m14.db draft board | head  # Cooper Flagg revealed

# Worker C: records
./target/release/nba3k --save /tmp/m14.db sim-day 30
./target/release/nba3k --save /tmp/m14.db records --scope season --stat ppg --limit 5
```

## Working agreements

- Each worker keeps `cargo test --workspace` green.
- DO NOT touch `crates/nba3k-cli/src/cli.rs`.
- TaskUpdate completed + send `team-lead` "done — N files, M tests" + go idle.
