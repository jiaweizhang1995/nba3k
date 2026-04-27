# M18 — Narrative layer: mandate + recap + export

## Scope

3 parallel workers.

## Pre-locked CLI

```
Command::Mandate { season, json }     → cmd_mandate       (worker-a)
Command::Recap { days, json }         → cmd_recap         (worker-b)
SavesAction::Export { path, to }      → cmd_saves_export  (worker-c)
```

## Worker A — Owner mandate

**Owned crates**:
- `crates/nba3k-store/migrations/V013__mandate.sql` (new).
- `crates/nba3k-store/src/store.rs` — `record_mandate(season, team, kind, target, weight)` + `read_mandates(season, team)`.
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_mandate` + auto-generation hook in `cmd_new` and `cmd_season_advance`.

**Goal**: at season start, owner sets 3 objectives (e.g. "win ≥ 45 games", "develop a player to OVR ≥ 85", "make playoffs"). Mid-season + end-of-season rendering shows progress + grade.

1. **Migration `V013__mandate.sql`**:
```sql
CREATE TABLE mandate (
    season INTEGER NOT NULL,
    team INTEGER NOT NULL,
    kind TEXT NOT NULL,        -- "wins" | "develop_to" | "make_playoffs" | "champion"
    target INTEGER NOT NULL,   -- target value (wins count, OVR threshold, etc.)
    weight REAL NOT NULL,      -- 0..1, contribution to grade
    PRIMARY KEY (season, team, kind)
);
```

2. **Auto-generate** in `cmd_new` and at the start of each new season inside `cmd_season_advance` (before printing the summary). Use deterministic team-ovr-and-roster heuristic:
   - Strong team (avg top-8 OVR ≥ 84): "make playoffs", "win ≥ 50", "win championship" (low weight on champion).
   - Mid team: "win ≥ 38", "develop one player +3 OVR", "make playoffs".
   - Rebuild team: "win ≥ 25", "draft top-3 lottery", "develop two players +3 OVR each".

3. **`cmd_mandate`**:
   - Resolve season (default current).
   - Walk mandates for user team.
   - For each, compute progress (current wins vs target, etc.).
   - Compute final grade if season is over: weighted average pass rate → A (≥0.85) / B (0.70-0.85) / C (0.55-0.70) / D (0.40-0.55) / F (<0.40).
   - Render text + JSON.

Tests in `crates/nba3k-store/tests/mandate.rs`:
- Insert + read round-trip.
- Grade calculation matches weight × pass-rate.

## Worker B — Game recap

**Owned crates**:
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_recap`.
- May need helper in `nba3k-season` or `nba3k-sim` to extract "key player line" from BoxScore.

**Goal**: post-game recap text. Per game in last N days:
```
2025-12-15 — BOS 112, LAL 108
  Tatum led BOS with 38 pts, 11 reb, 6 ast.
  LeBron led LAL with 32 pts, 7 reb, 9 ast.
  4Q: 28-19 BOS pulled away.
```
Skip the period-by-period stuff for v1 — just final score + top scorer per side.

1. **`cmd_recap`**:
   - Read games where `date >= today - days`. (Use `Store::read_games(season)` filtered by date.)
   - For each, extract top-scorer per side (highest pts in `box_score.home_lines`/`away_lines`).
   - Render per-game block.
   - JSON: array of `{date, home, away, home_score, away_score, home_top: {name, pts, reb, ast}, away_top: {...}}`.

Tests in `crates/nba3k-cli/tests/recap_smoke.rs`:
- After `sim-day 1`, `recap --days 1` shows ≥ 1 game.
- JSON parses.

## Worker C — Save export

**Owned crates**:
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_saves_export`.
- May need a `Store::dump_to_json()` helper in `nba3k-store`.

**Goal**: dump a save to a JSON file for sharing or backup. Reverse direction (import) is post-MVP — just export for now.

1. **`Store::dump_to_json(path: &Path) -> Result<serde_json::Value>`**:
   - Walk every persistent table: `meta`, `season_state`, `teams`, `players`, `schedule`, `games`, `standings`, `series`, `awards`, `cup_match`, `all_star`, `news`, `notes`, `mandate` (if migration present).
   - For each, build an array of rows.
   - Return one big JSON object: `{tables: {meta: [...], teams: [...], ...}}`.

2. **`cmd_saves_export`**:
   - Open source save path.
   - Call `dump_to_json`.
   - Pretty-print to `to`.
   - Print `exported /path/save.db → /path/dump.json (N tables, M rows)`.

Tests in `crates/nba3k-cli/tests/saves_export_smoke.rs`:
- Build a fresh save, export to tempfile, parse JSON, assert tables ≥ 5 keys.
- Refuse if `path` doesn't exist (clean error).

## Acceptance

```bash
rm -f /tmp/m18.db
./target/release/nba3k --save /tmp/m18.db new --team BOS

# Worker A: mandate
./target/release/nba3k --save /tmp/m18.db mandate

# Worker B: recap
./target/release/nba3k --save /tmp/m18.db sim-day 1
./target/release/nba3k --save /tmp/m18.db recap --days 1

# Worker C: export
./target/release/nba3k saves export /tmp/m18.db --to /tmp/m18.json
```

## Working agreements

- DO NOT touch `crates/nba3k-cli/src/cli.rs`.
- `cargo test --workspace` green at every commit boundary.
- TaskUpdate completed + send `team-lead` "done — N files, M tests" + go idle.
