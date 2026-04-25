# M5 — Realism v2: NBA 2K-borrowed mechanics

**Status**: Active (started 2026-04-25)
**Team**: `nba3k-m5` (4 workers + orchestrator)
**Reference**: `RESEARCH-NBA2K.md` for what NBA 2K does and why we borrow.

## Why this phase exists

User stated principle: "尽量做到realistic". Research surfaced 2K's core MyGM mechanics. M4 calibrated the existing engine; M5 lays the foundation that lets every future feature read realistic player data. The four headline borrows:

1. **21-attribute / 6-category rating schema** — replaces flat 10-field `Ratings`. Foundation for every model. Without it, archetype inference can't distinguish Curry from a true distributor (M4 carry).
2. **Player Role + morale + chemistry** — `Role` enum unlocks promised-PT contracts, force-out trade signal, and lineup chemistry as one connected mechanic.
3. **Player progression / regression** — `dynamic_potential` track-to-peak per 2K's mental model. Keeps `Player.potential` as the static ceiling.
4. **Awards engine + Playoffs** — MVP/DPOY/ROY/All-NBA + playoff bracket with best-of-7 series. Cheap & narrative-rich.

Plus injury depth (durability/fatigue rolls) and a light Coach struct for scheme fit.

## Architecture decisions (orchestrator-owned, set up-front)

### Decision 1: 21-attribute Ratings refactor lives in `nba3k-core`
- Replaces existing 10-field `Ratings` in `crates/nba3k-core/src/player.rs`.
- 6 categories: Inside Scoring (5) + Ranged Shooting (3) + Handling (3) + Defense (4) + Rebounding (2) + Athleticism (4) = 21 attributes.
- `overall_estimate()` becomes position-aware weighted sum (mirrors 2KLab heat map).
- Every `nba3k-models` consumer reads the new fields directly; no compatibility shim.
- Scrape data — `nba3k-scrape::ratings::box_stats_to_ratings` rewritten to populate all 21 from BBRef per-game stats. Compressed-data calibration is unchanged (still need named-star override).
- **No legacy struct preserved**. We're not shipping yet; v0.1.0 means breaking changes are free.

### Decision 2: `Role` is a sibling field on Player, not a derived score
- `pub enum PlayerRole { Star, Starter, SixthMan, RolePlayer, BenchWarmer, Prospect }`.
- Stored as `Player.role: PlayerRole`. Default = `RolePlayer`. CLI sets via `roster set-role <player> <role>`.
- `morale: f32` (0..=1) is also persistent on Player; updated by season events (PT below role expectation, role mismatch, contract incident).
- Chemistry is computed (`team_chemistry(snap, team_id) -> f32`) as a derived view — not stored.

### Decision 3: Progression is a sibling type, doesn't mutate `Player.potential`
- `Player.potential` stays as the static ceiling (immutable, set at scrape/draft).
- New `PlayerDevelopment` struct in `nba3k-models` carries `peak_start_age`, `peak_end_age`, `dynamic_potential`, `work_ethic`. Populated at scrape time (defaults), updated yearly by progression engine.
- End-of-season pass: each player rolls attribute deltas based on age vs peak, work_ethic, minutes played, training facility tier (M6+).

### Decision 4: Awards engine lives in `nba3k-season`
- `awards.rs` consumes `Store::read_games(season)` aggregates + standings, scores eligible players per award using documented weights (10-7-5-3-1 for MVP/All-NBA; 5-3-1 for ROY/DPOY/Sixth Man/MIP/COY).
- Persists to existing `awards` table (V001 schema already has it).
- All-NBA 1st/2nd/3rd team + All-Defensive 1st/2nd team fall out of the same scoring run.
- All-Star selection at mid-season day (game 41 marker).

### Decision 5: Playoff bracket extends nba3k-season
- `playoffs.rs` generates 16-team bracket from final standings (8 East + 8 West).
- Best-of-7 series sim. Rotation tightens (top-7) for playoffs. Each game uses sim engine with `is_playoffs=true` flag (already in `GameContext`).
- Champion + Finals MVP persisted.

### Decision 6: Light Coach struct, no playbook
- `pub struct Coach { id: CoachId, name: String, scheme_offense: Scheme, scheme_defense: Scheme, axes: CoachAxes }`.
- Lives next to `GMPersonality` in `nba3k-core`. Each `Team` gets a coach (default seeded if scrape doesn't provide).
- Used for `scheme_fit(player, coach) -> f32` in chemistry calc.
- Defer playbook depth (M7).

## Worker split (4 parallel)

Interface contracts pre-locked by orchestrator before workers spawn (same pattern as M3/M4).

### Worker A: `attributes` — owns the 21-attribute refactor
- Replace `Ratings` struct in `crates/nba3k-core/src/player.rs` with 21-field 6-category struct.
- Add `position_weighted_overall(&Ratings, Position) -> u8` static fn in core.
- Update `nba3k-scrape::ratings` to map BBRef per-game stats → 21 fields. Document the curve.
- Update sim's `RotationSlot.ratings` consumer (currently uses `shooting_3, playmaking, rebound, defense_perimeter, defense_interior` — pick analogues from new 21).
- Update `nba3k-models::stat_projection::infer_archetype` to use new attributes for better PG-scorer vs PG-distributor distinction.
- Migrate Store: `players.ratings_json` blob is just JSON — no schema change needed (JSON1 column reads any shape; existing saves get a one-time conversion via a `meta` flag).
- Tests: every existing test in core/sim/models that constructs a `Ratings` literal needs updating. Likely 50+ test fixtures.

### Worker B: `role-chemistry` — Role enum, morale, chemistry, light Coach
- Add `PlayerRole` enum + `morale: f32` field to `Player` (`crates/nba3k-core/src/player.rs`).
- Add `Coach` struct + `Scheme` enum + `CoachAxes` to `nba3k-core`. Add `Team.coach: Coach` field.
- Implement `team_chemistry(snap, team) -> Score` in `nba3k-models`. Components: role-vs-archetype mismatch, positional balance, star-stack penalty, scheme fit.
- Implement `scheme_fit(&Player, &Coach) -> f32` in `nba3k-models`.
- Apply chemistry as ±5% game-day multiplier in `nba3k-sim`.
- Tests: chemistry of well-fit team > chemistry of star-stacked team; Star-in-BenchWarmer-role tanks morale.

### Worker C: `progression` — yearly track-to-peak engine
- Add `PlayerDevelopment` struct in `nba3k-models` with `peak_start_age`, `peak_end_age`, `dynamic_potential`, `work_ethic`.
- New module `crates/nba3k-models/src/progression.rs`:
  - `progress_player(player, dev, age, mins_played, work_ethic) -> AttributeDelta`
  - `regress_player(player, dev, age) -> AttributeDelta`
  - `update_dynamic_potential(player, dev, current_age) -> u8`
- New season-end pass in `nba3k-season::phases` that walks all players + applies progression.
- Persistence: store `PlayerDevelopment` as JSON blob next to player ratings (need a `dev_json` column in `players` — V003 migration).
- Tests: 22-yo OVR-78 with potential 90 + 35-min/game season → +2-3 OVR; 32-yo OVR-86 → -1-2 OVR (athleticism declines first).

### Worker D: `awards-playoffs` — Awards engine + Playoff bracket
- New `crates/nba3k-season/src/awards.rs`:
  - `compute_mvp(games, standings) -> AwardResult` (top-5 ballot, 10-7-5-3-1).
  - Same shape for DPOY/ROY/Sixth Man/MIP/COY (5-3-1).
  - All-NBA 1st/2nd/3rd team + All-Defensive 1st/2nd team.
  - All-Star selection (mid-season day-41 trigger).
- New `crates/nba3k-season/src/playoffs.rs`:
  - `generate_bracket(standings, season) -> Bracket` (8 East + 8 West).
  - `simulate_series(home, away, sim_engine, rng) -> SeriesResult` (best-of-7, home-court 2-2-1-1-1).
  - Per-game sim uses `GameContext.is_playoffs = true`.
  - Finals MVP at conclusion.
- CLI commands: `playoffs bracket`, `playoffs sim`, `awards [--season]`, `season-summary`.
- Store API: `record_award`, `read_awards(season)`, `record_series`.
- Tests: 16-team bracket has correct seeding + brackets; best-of-7 ends 4-0 to 4-3.

## Acceptance

```bash
# Setup with M5-refactored core (CRITICAL: regenerate seed first since
# scraper writes new attribute schema).
cargo run -p nba3k-scrape --release -- --season 2025-26 --out data/seed_2025_26.sqlite

# 1. Confirm new schema in core types.
cargo test --workspace
# 146+ tests pass after refactor.

# 2. Curry now distinguishable from Halliburton — Curry's 3pt rating > 90,
#    playmaking ~75; Halliburton's 3pt ~78, playmaking ~95. infer_archetype
#    picks SG-shooter for Curry, PG-distributor for Halliburton.
nba3k --save run.db new --team BOS --season 2026
nba3k --save run.db sim-day 60
# Curry post-sim: ~26 PPG / ~6 APG (was 14.5 APG in M4-polish).

# 3. Role + chemistry — assign Star role, see morale stay high.
nba3k --save run.db roster set-role "Jayson Tatum" star
nba3k --save run.db chemistry BOS --json
# {"team": "BOS", "score": 0.85, "reasons": [...]}

# 4. Progression — sim full season + draft + simulate next season; 22-yo
#    high-potential player gains OVR.
nba3k --save run.db sim-to playoffs
nba3k --save run.db season advance     # triggers end-of-season progression
nba3k --save run.db player "Jaylen Brown" --json | jq .overall

# 5. Playoffs.
nba3k --save run.db playoffs bracket --json
nba3k --save run.db playoffs sim
nba3k --save run.db season-summary --json
# { "champion": "...", "finals_mvp": "...", "awards": {"mvp": "...", "dpoy": "..."} }

# 6. Awards.
nba3k --save run.db awards --json | jq '. | {mvp: .mvp.player, dpoy: .dpoy.player}'
```

## Implementation phases

1. **Orchestrator wave 0** (~2 hours): pre-stage 21-attribute `Ratings` shape (with `todo!()` overall function), `PlayerRole` + `morale` fields, `Coach` + `Scheme` types, `PlayerDevelopment` shape, `Bracket` + `AwardResult` placeholders. Update existing tests' `Ratings` literals to compile (defaults). Run `cargo build --workspace` clean.
2. **Workers wave 1** (parallel A/B/C/D): each fills their owned modules and tests in isolation. Workers A's refactor is the biggest; others build against pre-locked shapes.
3. **Orchestrator wave 2**: rewire CLI commands, regenerate seed via scraper, run M5 acceptance bash. Mark complete, shutdown team.

## Risks

1. **Worker A's cascade is large.** Every test fixture in core/sim/models constructs `Ratings`. Orchestrator pre-staging keeps the field count consistent so workers don't fight test breakage.
2. **Progression interacts with player IDs across seasons.** Need to ensure `Player` survives across season boundaries (it does — saved in `players` table, just attributes mutate).
3. **Awards rely on accurate stats** — M4's stat_projection is sufficient for MVP/scoring titles; defensive metrics (DRtg) less so. DPOY may be noisy until M7 polish.
4. **Coach data**. We don't currently scrape coaches. Either: (a) hardcoded stub coaches per team in `data/coaches.toml`; (b) scrape from BBRef coach pages. Worker B picks one — recommend (a) for v1.

## Decision log (filled in during phase)
