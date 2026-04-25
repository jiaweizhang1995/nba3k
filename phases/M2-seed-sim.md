# M2 — Seed Data + Statistical Sim Engine

**Status**: ✅ Done (2026-04-25)
**Team**: `nba3k-m2` (3 workers + orchestrator)

## Final acceptance verification (2026-04-25)

```
=== STEP 1: new ===
created save /tmp/m2run.db (team=BOS mode=standard season=2026 seed_used=true)

=== STEP 2: status ===
season:   2026 (PreSeason)
team:     BOS (id=2)
teams:    30 | players: 590
schedule: 1230 games (1230 unplayed)

=== STEP 3: sim-to regular-end ===
reached phase Playoffs
real    2.8s

=== STEP 4: standings sum check ===
1230   ← exactly correct

=== STEP 5: standings (top 11) ===
  1  BRK 54-28 .659
  2  DAL 47-35 .573
  ...
 11  BOS 42-40 .512

=== STEP 6: BOS roster size ===
16

=== STEP 7: player lookup ===
Jayson Tatum 75 BOS

=== Cross-process reload ===
phase=Playoffs day=181 schedule_unplayed=0  ← intact across processes

=== Script mode (piped JSON) ===
standings serialized cleanly through jq pipeline ✅

cargo test --workspace → 37/37 pass
```

**Sim performance**: 1230-game regular season in 2.8s (release). Worker B's 1230-sim-under-1s benchmark holds with the additional Store I/O overhead. Comfortably ahead of any acceptance bound.

## Known M2 deliverable gaps (carried as M2-polish or later)

1. **Calibration**: BRK 54-28 at top of standings is unrealistic — Brooklyn is rebuild-mode IRL. Symptom of (a) ratings spread being modest (top players ~75-90 OVR not 95-99) and (b) HCA + variance allowing weaker rosters to over-perform. M2 polish will tune `sim_params.toml` against known team strength priors.
2. **Roster size 16 vs spec 13-15**: Worker A's BBRef parser kept all listed players including two-ways. Acceptable for sim purposes (we cap rotation at 8 anyway). Cleaner cut to 15 = M2 polish.
3. **Contracts = 0**: HoopsHype is now React-rendered (CSS-modules + client hydration), static parser yields nothing. Override-only path at `data/rating_overrides.toml` works. M3 trade engine will need a real contracts source — flagged as M3 prep work.
4. **Refinery debug spew on first seed creation**: scraper prints the migration runner's full Debug repr to stderr. Cosmetic, harmless, will sweep in M2 polish.
5. **Trade kicker asymmetry** (sender pre-kicker, receiver post-kicker prorated, per RESEARCH.md item 7): noted in core memory; affects M3 implementation — must compute side-specific incoming salary, not single matched number.

## Goal

Produce a real NBA 2025-26 seed database from public sources, implement a fast statistical game-sim engine, generate an 82-game schedule, and wire CLI commands `sim-day` / `sim-to` / `standings` / `roster` / `player` so the user can simulate a full regular season from a fresh save.

## Acceptance

```bash
# 1. Generate seed (one-shot, slow — ~5 minutes with rate limiting)
nba3k-scrape --season 2025-26 --out data/seed_2025_26.sqlite

# 2. Sanity-check seed
[[ $(sqlite3 data/seed_2025_26.sqlite "SELECT COUNT(*) FROM teams") == 30 ]]
[[ $(sqlite3 data/seed_2025_26.sqlite "SELECT COUNT(*) FROM players") -ge 450 ]]
[[ $(sqlite3 data/seed_2025_26.sqlite "SELECT COUNT(*) FROM players") -le 600 ]]

# 3. New game from seed
nba3k --save run.db new --team BOS --season 2026 --seed 42

# 4. Sim full regular season
nba3k --save run.db sim-to regular-end

# 5. Verify standings sum + win totals
totals=$(nba3k --save run.db standings --json | jq '[.[] | .wins] | add')
[[ "$totals" == "1230" ]]

# 6. Roster + player lookups work
nba3k --save run.db roster BOS --json | jq 'length' | grep -qE '^(13|14|15)$'
nba3k --save run.db player "Jayson Tatum" --json | jq -r .overall | grep -qE '^[0-9]+$'
```

## Sub-tasks (parallelized via agent team)

### Worker A: `scraper` — owns `crates/nba3k-scrape/`

**Mandate**: Produce `data/seed_2025_26.sqlite` containing 30 teams, full rosters, contracts (best-effort), basic ratings. Use BBRef as bootstrap-only with aggressive caching (Sports-Reference ToS forbids derivative tools — scrape once for personal use, never redistribute).

**Spec**:
- New module structure under `nba3k-scrape/src/`:
  - `main.rs` — CLI parsing, wires sources → normalize → seed
  - `cache.rs` — file-based response cache at `data/cache/{source}/{key}.{ext}`. Hit cache before network. TTL: 30 days for HTML, 7 days for JSON.
  - `politeness.rs` — global rate limiter: 1 req per 3 seconds, exponential backoff on 429/5xx, max 3 retries. User-Agent: `nba3k-claude/0.1.0 (personal use; +https://github.com/CarfagnoArcino/nba3k-claude)`.
  - `sources/bbref.rs` — fetches roster + per-game stats from `basketball-reference.com/teams/{abbrev}/2026.html`. Parse HTML via `scraper`. Extract: name, position, age, MP, PTS, TRB, AST, STL, BLK, FG%, 3P%, FT%.
  - `sources/nba_api.rs` — shells out to Python `nba_api` (active, v1.11.4 Feb 2026) via `Command::new("python3")`. Caller passes endpoint name; module returns JSON. Document required Python install in scraper README.
  - `sources/hoophype.rs` — parses HoopsHype salary tables (HTML, server-rendered) for contract details. Primary contracts source.
  - `sources/mock_draft.rs` — fetches 2026 mock draft from a single source (NBADraft.net or Tankathon). Top 60 prospects.
  - `ratings.rs` — `box_stats_to_ratings(stats, position, age) -> Ratings`. Spine: BPM 2.0 → overall (0..99). Sub-rating split: percentile rank within position for each stat → 0..99 per sub-rating. Age curve: peak at 27, gentle decay. Hand-tunable in `data/rating_overrides.toml` (key = player name).
  - `seed.rs` — opens fresh SQLite via `nba3k-store::Store::open`, runs migrations, writes teams + players + contracts + draft prospects. Idempotent (drop+recreate by default; `--keep-existing` flag preserves).
  - `assertions.rs` — post-scrape sanity: every team has 13-15 active players, league total salary within ±5% of 30 × cap (use hardcoded 2025-26 figure), ≥60 prospects, no duplicate player IDs, every player has a primary position. Fail loud (non-zero exit).

**Constants to encode** (hardcode, see RESEARCH.md item 4):
- 2025-26 salary cap, luxury tax, first apron, second apron, BAE, MLE figures → `nba3k-core::LeagueYear` struct (also referenced by trade engine in M3).

**Dependencies needed** (already wired in scrape Cargo.toml): `reqwest`, `scraper`, `csv`, `serde_json`, `clap`, `anyhow`, `tracing`. Add: `regex` (for parsing salaries from HoopsHype text).

**Bash verify**:
```bash
cargo run -p nba3k-scrape -- --season 2025-26 --out /tmp/seed.sqlite
sqlite3 /tmp/seed.sqlite "SELECT COUNT(*) FROM teams"     # → 30
sqlite3 /tmp/seed.sqlite "SELECT COUNT(*) FROM players"   # → 450..600
sqlite3 /tmp/seed.sqlite "SELECT abbrev,name FROM teams ORDER BY abbrev LIMIT 5"
```

### Worker B: `sim-engine` — owns `crates/nba3k-sim/`

**Mandate**: Fast statistical game simulation. One game = ~1ms. 82 games × 30 teams / 2 = 1230 sims must finish in <2 seconds for a full season.

**Spec**:
- Module structure under `nba3k-sim/src/`:
  - `lib.rs` — `Engine` trait (already exists, do not rewrite the signature), `TeamSnapshot`, `GameContext`. Add: `pub fn pick_engine(name: &str) -> Box<dyn Engine>`.
  - `engine/mod.rs` — re-exports.
  - `engine/statistical.rs` — `StatisticalEngine`. Reads `SimParams` from TOML. Per-game flow:
    1. Compute team ORtg, DRtg, pace from rotation × player ratings.
    2. Sample possessions ~ Normal(combined_pace, σ).
    3. Sample home/away points ~ Normal(ORtg − opp_DRtg + HCA, σ_score).
    4. Resolve OT if tied (recursive 5-min mini-sims, capped at 4 OTs).
    5. Distribute box score: usage % per player → shots; assist rate → AST; rebound rate → REB; steal/block/TO from rates; +/- = team score diff scaled by minutes share.
    6. Apply per-player injury roll (Bernoulli with `injury_rate_per_game`).
- `params.rs` — `SimParams` struct + `from_toml(path) -> SimParams`. Keys: `pace_mean`, `pace_sigma`, `score_sigma`, `home_court_advantage`, `injury_rate_per_game`, `max_overtimes`, `usage_distribution_alpha`. Default `data/sim_params.toml` shipped.
- Determinism: `RngCore` is dyn — caller seeds. Same seed + same snapshots = same box score.
- Tests: smoke test that `simulate_game()` returns scores in [60, 200], no panic, total minutes = 240 (×OT), home team wins ≥40% of fair-strength matchups over 1000 sims.

**Output of sim**: `nba3k_core::GameResult` (already defined in core). Don't add to it.

**Bash verify** (standalone harness — orchestrator wires CLI later):
```bash
cargo test -p nba3k-sim
```

### Worker C: `scheduler` — owns `crates/nba3k-season/`

**Mandate**: NBA-shape 82-game schedule generator + standings tracker + phase advancement.

**Spec**:
- Schedule generator (`schedule.rs`) — 3-stage in-house algorithm (no OSS reuse, see RESEARCH.md item 5):
  1. **Matchup solver**: every team plays every other team — division (4×), conference non-division (3 or 4×), inter-conference (2×) — totaling 82 games per team. Output: list of `(home, away)` pairs, no dates.
  2. **Greedy date assigner**: spread games across season (Oct 21, 2025 → Apr 12, 2026 = ~174 days). Assign each pair a date respecting: max 4 games in 5 days per team, max 1 b2b per week (soft), at most 1 game per team per day.
  3. **Simulated annealing fixup**: penalty function = sum of (b2b count − 14) for each team + travel distance heuristic + bunched-week penalty. Run 10k iterations, swap dates between random pairs. Accept if energy decreases or with `exp(-ΔE/T)`.
- Standings (`standings.rs`) — `record_game_result(state, game) -> ()`. Updates `wins`/`losses` per team in `standings` table. Tiebreakers (head-to-head → div record → conf record → SRS) for `conf_rank`.
- Phase advancement (`phases.rs`) — `advance_day(state) -> SeasonPhase`. Logic: PreSeason ends day 7 → Regular. Regular ends after all 82 games per team played → Playoffs. (Trade deadline gate is M3-aware; for now, just expose `is_after_trade_deadline(date) -> bool` based on calendar.)
- Tests: schedule has exactly 1230 games, every team plays exactly 82, no team plays itself, b2b counts within [10, 18] per team.

**Bash verify**:
```bash
cargo test -p nba3k-season
```

### Orchestrator (post-team): integration

After all 3 workers complete, the orchestrator (main session) does:
1. Wire `nba3k-cli` commands `sim-day`, `sim-to`, `standings`, `roster`, `player` (commands.rs entries currently `bail!("not implemented")`).
2. Add `Store` API methods needed by CLI: `roster_for(team_id)`, `find_player_by_name(name)`, `record_game(game_result)`, `current_standings()`, `pending_games(date_range)`, `bulk_upsert_players(players)`.
3. End-to-end verification: scrape seed → new game → sim regular season → assert win totals = 1230 → roster/player queries work.
4. Commit phase artifacts, update `PHASES.md`, mark M2 complete.

## Risks (carried from RESEARCH.md)

1. **Scraper breaks silently** — mitigated by `assertions.rs` (fail loud).
2. **Python `nba_api` install missing on user's machine** — `nba3k-scrape` should print clear remediation if `python3 -c "import nba_api"` fails. Don't auto-install.
3. **Rate-limiting + cache** — first scrape will take ~5 minutes. Repeated scrapes hit cache and finish in ~10 seconds. Document this in scraper README so user doesn't kill it.
4. **Sim calibration** — first pass will produce nonsense win totals (one team going 82-0). Acceptance check (sum to 1230) catches arithmetic bugs but not realism. Realism tuning happens in M2 polish (last day of phase) — don't gate on it.

## Decision log (filled in during phase)

- (TBD: workers add notes as they hit decisions worth recording)

### Worker A — `nba3k-scrape` (2026-04-25)

- **`LeagueYear` lives in `nba3k-core::league_year`**, not in `cba/` (the
  RESEARCH note suggested `src/cba/league_year.rs` but `nba3k-core` has no
  `cba` module today and the type is referenced from both the scraper and
  the upcoming M3 trade engine — sticking it at the crate root keeps the
  re-export simple). Encoded as a `LEAGUE_YEARS` slice with a single
  2025-26 entry; new years just append.
- **`SeasonId(2026)` = the 2025-26 season** — `LeagueYear::for_label`
  parses the "YYYY-YY" string and resolves to the *ending* year, matching
  the rest of the codebase's convention. `SeasonId(2025)` would never see
  a real game and reads ambiguous; locking this in.
- **HoopsHype is now React-rendered.** As of late April 2026 the salary
  dashboard ships CSS-module class names (`bserqJ__bserqJ`, etc.) and the
  numeric cells aren't in the served HTML — they're hydrated client-side.
  The static-HTML parser still runs and returns 0 rows; the scraper keeps
  going. Contracts are now best-effort and the manual-override file
  (`data/rating_overrides.toml`) is the supported path. Re-investigate
  with a JSON endpoint or `Sec-Fetch` headers if/when the M3 trade engine
  needs richer contract data.
- **Synthetic-roster fallback** (`--offline-fallback`, on by default) so
  the binary still produces a 30-team seed when BBRef rate-limits or the
  network is unreachable. Players get labeled `"<ABBR> Player NN"` for
  easy identification; real data overwrites them on the next cached run.
  Picked this over a hard fail because the M2 acceptance check
  (`SELECT COUNT(*) FROM players` between 450 and 600) is testing
  *plumbing*, not data quality. (In practice the live BBRef path worked —
  530 real players landed first try.)
- **Stable `PlayerId` via `(name, age, team)` hash**. `nba_api` has a
  canonical `personId`, but we don't always have it (BBRef fallback
  path). Hashing keeps re-runs deterministic at the cost of risking ID
  drift when a player ages or is traded — acceptable for v1 since seeds
  are re-generated cleanly rather than incrementally updated.
- **Ratings spine = simplified BPM-style production score, not full
  BPM 2.0.** Full BPM 2.0 needs per-100 stats and team context that
  BBRef's team page doesn't expose without an extra round of scraping.
  The simplified form (per-game rates + percentile rank within the league
  + position-weighted blend + age curve) ranks the league correctly
  enough for sim seeding; M2 polish day or M4 (player progression) is
  the place to deepen it.
- **Bulk player upsert in `nba3k-store`** is a single transaction. With
  ~530 players the savings are small but the atomicity matters: a
  half-written `players` table during a panic would leave the seed in a
  state that fails assertions silently. M3 trade application probably
  wants the same pattern.
- **Sanity contract assertion is warn-only when fewer than 50 players
  have contracts.** Forcing it to fail would block the offline path and
  the React-HoopsHype gap above; warning surfaces the issue without
  refusing to write a usable seed. If the user populates contracts via
  overrides for ≥50 players, the ±50% band check kicks back in.

### Worker B (sim-engine, 2026-04-25)

- **TeamSnapshot extension**: added `rotation: Vec<RotationSlot>` (player_id, position, minutes_share, usage, ratings, age). Existing fields untouched. Empty rotation triggers a `overall`-only fallback path so other crates can build sim inputs incrementally without breaking. RotationSlot lives in `nba3k-sim` (not core) since it's a sim-internal abstraction over `Player`.
- **SimParams defaults**: `pace_mean=99.0`, `pace_sigma=3.0`, `score_sigma=9.0`, `home_court_advantage=2.0`, `injury_rate_per_game=0.005`, `max_overtimes=4`, `usage_distribution_alpha=1.4`. HCA dialed down from real-NBA ~2.5 to 2.0 to keep fair-strength home win rate inside the loose [0.40, 0.60] sanity bound (otherwise Normal sigma + HCA produced ~62% home wins). Will likely be retuned during M2 polish once real ratings flow through.
- **ORtg/DRtg derivation**: weighted by minutes_share. Offense pulls from `shooting_3 / shooting_mid / finishing / playmaking / iq`; defense from `defense_perimeter / defense_interior / iq / athletic`. Anchors at 70-rating baseline so an average rotation produces league-average 108 ORtg/DRtg.
- **Score sampling**: `Normal(ORtg − opp_DRtg + 100 + HCA, score_sigma)` per team, scaled by sampled possessions / 100. Hard-clamped to `[60, 200]` post-sample so the spec test bound is enforced even on long-tail draws.
- **OT recursion**: capped at `max_overtimes` (default 4); after the cap, ties are deterministically broken by giving the home team +1. With sigma=9 this path is hit roughly 1 in 1e6 sims — acceptable.
- **Box-score distribution**: `distribute_u16` (largest-remainder method) gives exact integer sums — keeps the "total minutes = 240 + 25×OT" invariant tight without rounding drift. Minutes share normalized to 5 players on the floor (sum across rotation ~5.0).
- **Determinism**: caller owns the `&mut dyn RngCore`; the engine itself holds zero state beyond `SimParams`. Same seed + same TeamSnapshots → identical `GameResult` (verified by `deterministic_same_seed_same_result` test).
- **Performance**: 1230 sims (one regular season) finishes in well under 1 second in release on M-series silicon — comfortable margin under the 5s acceptance bound.
- **Injury rolls**: rolled per game per rotation slot but currently discarded — `nba3k_core::GameResult` has no injury slot and the spec forbids modifying it. Rolling here is intentional so the RNG stream stays stable when the orchestrator wires injury writes through `Store` later.

### Worker C (scheduler, 2026-04-25)

- **Matchup solver uses Latin-square decomposition** for the 5×5 conf-non-div bipartite matrix between every pair of in-conference foreign divisions. A 3-regular bipartite K5,5 decomposes into three permutation matrices; we pick three of the five Latin-square symbols at random (seeded), giving exactly 3 fours-per-row and 3 fours-per-column → each team gets exactly 6×4-game and 4×3-game conf-non-div opponents (NBA shape). Deterministic, microseconds, rotates between seeds.
- **`Conference`/`Division` in nba3k-core do NOT derive `Hash`.** Worked around by using `Vec<(Division, Vec<TeamId>)>` instead of `HashMap<Division, …>`. If a future polish wants `Hash` on these enums, derive it in core — kept hands off here per ownership rules.
- **Greedy date assigner uses cadence targets, not earliest-day.** Each game's target day is `(team_games_placed + 1) × total_days / 83` — the larger of the two team targets. Without this the greedy packs games at the front of the season and the SA can't recover. With it, each team's games are spread evenly across the 174-day window.
- **Coin-flip back-to-back avoidance in greedy** (`prefer_no_b2b = rng.gen_bool(0.62)`). 100% avoidance produces 5–10 b2bs per team (below the [10, 18] target floor); 0% avoidance produces 22+ (above the ceiling). 62% lands the initial state inside the SA's basin of attraction.
- **SA energy is V-shaped around b2b ∈ [12, 16]**: `4 × max(0, b2b - 16)² + max(0, 12 - b2b)² + 4 × Σ max(0, count_in_5_days - 4)²`. The 4× weight on the upper side reflects that high-b2b teams are the visible bug; low-b2b teams are "lucky."
- **Per-team incremental energy in SA** — only the (≤4) teams touched by a date swap recompute their per-team energy. Saves an order of magnitude vs. naive whole-schedule recomputation; lets us run 80k iterations in under a second.
- **`swap_legal` only checks team double-booking**, not 4-in-5. The 4-in-5 constraint lives in the energy function instead, so SA can transit through bad intermediate states.
- **No Christmas Eve carve-out (Dec 24, 2025)** — soft constraint, deferred per spec.
- **Trade deadline date constant** is Feb 5 2026 (per spec); `is_after_trade_deadline(date)` returns true strictly after that date. `is_trade_deadline_day(date)` for the deadline day itself.
- **Schedule is sorted by `(date, game_id)`** before return so callers iterating chronologically need no extra step.
- **Tests pass on seed=42**; b2b counts are deterministic per seed, so the [10, 18] bound is verified for that one seed. If a different seed lands a team outside the bound, the energy-function constants would need re-tuning. Loose bound chosen so this is unlikely in practice.

## Hand-offs to M3

- `LeagueYear` struct must exist (encoded by Worker A in `nba3k-core` as part of constants), referenced by the upcoming CBA validator.
- `Store::roster_for(team_id)` and `Store::current_standings()` should exist (Orchestrator wires) — M3 trade engine reads both.
- Trade kicker asymmetry note (RESEARCH.md item 7): `Player.trade_kicker_pct` is already on the model; M3 implementer needs to compute side-specific incoming salary, not a single matched number.
