# M4 — Realism Engine (user-labeled "Phase 8")

**Status**: Active (started 2026-04-25)
**Team**: `nba3k-m4` (4 workers + orchestrator)

## Why this phase exists (verbatim from user)

> 围绕篮球真实性设计和实现一个窄切片。
> 1. 建立更真实的球员价值模型。
> 2. 建立更真实的球队策略/资产偏好模型。
> 3. 建立球星保护 / franchise player / untouchable 逻辑。
> 4. 改善交易 AI，避免荒谬报价。
> 5. 改善赛季模拟数据，让球星数据更符合直觉。
> 6. 改善自由市场/合同接受逻辑（M4-stretch；可推到下一阶段）。

**Core principle**: no scattered if/else patches. Replace ad-hoc logic with extensible, weighted, **explainable** scoring models. Every score returns its `value` plus a list of `Reason {label, delta}` so we can answer "why did LAL accept this?" deterministically.

## Architecture decision

New crate **`nba3k-models`** — sibling to trade/sim/season. Houses all 7 models. **`LeagueSnapshot` and `TeamRecordSummary` relocate from `nba3k-trade` → `nba3k-core`** so models don't have to depend on trade (avoids the trade ↔ models cycle). The struct shapes are unchanged; only the import path moves. This is **not** a state-model fork.

```
nba3k-core
  ├── (existing types)
  └── snapshot.rs         ← LeagueSnapshot + TeamRecordSummary moved here

nba3k-models               (NEW)
  ├── lib.rs              ← Score + Reason types, model registry
  ├── player_value.rs
  ├── contract_value.rs
  ├── team_context.rs
  ├── star_protection.rs
  ├── asset_fit.rs
  ├── trade_acceptance.rs
  ├── stat_projection.rs
  └── weights.rs          ← TOML loader for tuning

nba3k-trade               (refactored to consume nba3k-models)
nba3k-sim                 (refactored to consume nba3k-models::stat_projection)
```

## Common types — defined upfront by orchestrator before workers spawn

```rust
// nba3k-models/src/lib.rs

pub struct Score {
    pub value: f64,                 // unitless or $-equiv depending on model
    pub reasons: Vec<Reason>,       // ordered by largest |delta| desc
}

pub struct Reason {
    pub label: &'static str,
    pub delta: f64,                 // signed contribution to `value`
}

pub trait ExplainableModel {
    type Input<'a>;
    type Output;
    fn evaluate(&self, input: Self::Input<'_>) -> Self::Output;
}
```

Each model exposes a top-level `pub fn` (the canonical entry) plus its weights struct loadable from `data/realism_weights.toml`. Hardcoded defaults ship inline so the file is purely a tuning layer.

## The 7 models

### 1. `player_value_score`  (Worker A)
Replaces `nba3k-trade::valuation::player_value`. Surplus value of a player to an evaluator team.

**Inputs**: `&Player`, `&GMTraits` (post-context), `&LeagueSnapshot`, current_season.
**Output**: `Score` in cents.
**Components** (each emits a Reason):
- `baseline_for_ovr_position` (positional curves; no flat OVR→$ globally)
- `age_curve` (peak 27, position-specific cliffs — bigs decline slower than guards)
- `star_premium` (nonlinear above OVR 88)
- `contract_surplus` (talent_value − salary, weighted by salary_aversion)
- `loyalty_bonus` (only when player is on evaluator's team)
- `fit_bonus` (positional need; from asset_fit_score sub-call)

Drops the M3 hack of "everyone-25-OVR-75". Bakes in a **named-star override list** (`data/star_roster.toml`) so Luka/Jokic/Giannis/Tatum/SGA/etc. project as stars even when raw scrape data is bad. **This is the calibration backstop while the rating layer is still broken** — the override is documented, file-loadable, hand-curated.

### 2. `contract_value_score`  (Worker A)
Surplus of a contract relative to the player's market value at that OVR/position. Key for Cheapskate vs WinNow divergence.

**Inputs**: `&Contract`, `&Player`, `&GMTraits`, `&LeagueYear`.
**Output**: `Score` in cents (positive = team-friendly deal, negative = overpay).
**Components**:
- `expected_market_for_ovr` (expected $/year for that OVR/age/position from a curve)
- `actual_salary` (current year + future commitments discounted 8%/yr)
- `option_value` (player option = positive for player / negative for team; reverse for team option)
- `expiring_premium` (last-year contracts are tradeable assets — bonus for contender perspective)

### 3. `team_context_score`  (Worker B)
Replaces `nba3k-trade::context::classify_team`. Returns *both* a discrete `TeamMode` and a continuous score vector.

**Inputs**: `team_id`, `&LeagueSnapshot`.
**Output**: `TeamContext { mode: TeamMode, contend_score: f32, rebuild_score: f32, win_now_pressure: f32, reasons: Vec<Reason> }`.
**Components**:
- `roster_age_signal` (rotation avg age vs league avg)
- `top_ovr_signal` (top-3 OVR vs league median)
- `standings_signal` (current rank + pace if season started)
- `cap_commitment_signal` (years × $ committed; future flexibility = rebuild signal)
- `recent_history_signal` (last-N season trajectory; M5+ when we have it — return 0 for v1)

### 4. `star_protection_score`  (Worker B)
**The user-stated gap**: "Luka 是非卖品". Returns a "untouchable factor" 0..1 per (player, owning_team) pair.

**Inputs**: `player_id`, `&LeagueSnapshot`.
**Output**: `Score` with value in `[0.0, 1.0]`. 1.0 = absolute untouchable, 0.0 = "we'd ship him for the right price".
**Components**:
- `franchise_tag` (1.0 if listed in `data/star_roster.toml` for this team)
- `top_ovr_on_team` (highest OVR on team gets +0.4)
- `young_ascending` (age ≤ 24 + potential ≥ 90 → +0.3)
- `recent_signing` (signed/extended in last 12 sim months → +0.2 — defer if no acquired_on data)
- `team_mode_modifier` (Contend mode raises protection on top players; FullRebuild lowers it everywhere)

The trade engine reads this score:
- ≥ 0.85 → reject any offer, regardless of value, with reason "untouchable".
- 0.60-0.85 → require value premium (e.g., +25% over normal Accept threshold).
- < 0.60 → no extra friction.

### 5. `asset_fit_score`  (Worker D)
Position + skill fit for an *incoming* player from the receiving team's perspective. Captures the "we don't need another center" intuition.

**Inputs**: incoming `&Player`, receiving `team_id`, `&LeagueSnapshot`.
**Output**: `Score` in cents (signed: positive = good fit bonus, negative = redundancy penalty).
**Components**:
- `positional_need` (does the team lack rotation depth at this position?)
- `skill_overlap` (high-OVR same-archetype-position = penalty)
- `rotation_minutes_available` (if team's top-8 minutes are saturated → lower fit)

### 6. `trade_acceptance_score`  (Worker D — COMPOSITE)
Replaces the verdict thresholds in `nba3k-trade::evaluate`. Composes models 1-5 into a single accept-probability with explanation.

**Inputs**: `&TradeOffer`, `evaluator_team`, `&LeagueSnapshot`, `&mut RngCore`.
**Output**:
```rust
pub struct TradeAcceptance {
    pub probability: f64,           // 0..1
    pub verdict: Verdict,           // Accept | Reject | Counter
    pub net_value_cents: i64,
    pub reasons: Vec<Reason>,       // composite: top-K from sub-models
    pub commentary: String,         // GM mouth, derived from reasons
}
```
**Composition**:
- For each outgoing player on evaluator's side: `player_value` + `loyalty` + `contract_value` + `star_protection` × outgoing_penalty.
- For each incoming player: `player_value` + `asset_fit` + `contract_value`.
- Picks: pick_value × pick_value_multiplier (current valuation works; just port).
- Net = sum_in − sum_out.
- Star protection on outgoing ≥ 0.85 → hard short-circuit to Reject("untouchable") regardless of net.
- Threshold based on team_mode + season phase (lifted from current evaluate).
- Probability via logistic on net_pct (smooth replacement for the +5%/−15% step thresholds).
- Gaussian noise per gullibility (preserved from M3).

This is the **headline replacement**: instead of binary thresholds, a continuous probability with sample-based verdict. Cleaner reason chaining.

### 7. `stat_projection_model`  (Worker C)
Replaces the per-game box-score distribution in `nba3k-sim::engine::statistical`. The user's stated complaint: "Luka 应该 30+ 三双频率高，普通球员不会".

**Inputs**: `&Player`, `&TeamSnapshot` (team rotation), `&GameContext`, `&mut RngCore`.
**Output**: `PlayerLine` (existing core type) — but generated from a **per-archetype skill profile**, not a single usage% slot.

**Components** (each player has a position-specific archetype tag):
- `archetype_profile` (PG-distributor / PG-scorer / SG-shooter / SF-3andD / PF-stretch / PF-banger / C-finisher / C-stretch). Each archetype has expected per-100-possession rate ranges for PTS/REB/AST/STL/BLK/TOV/3PA/FTA.
- `usage_share` (from team rotation — same as current).
- `pace_modifier` (game-context).
- `star_uplift` (named-star override file boosts ceiling for Luka/Jokic/etc. → triple-double rate, 30+ PPG).
- `injury_throttle` (minutes capped if injury status).

Result: stars produce star lines (Luka 30/8/8 is now plausible). Role players cap at 12-15 PPG. Triple-doubles emerge from high-usage primary creators with rebound bonuses, not random rolls.

## Worker split (4 parallel)

| Worker | Files (paths in `crates/nba3k-models/src/`) | Models owned | Depends on |
|---|---|---|---|
| **A: value** | `player_value.rs`, `contract_value.rs`, plus `data/star_roster.toml` | 1, 2 | core types |
| **B: context** | `team_context.rs`, `star_protection.rs` | 3, 4 | core types |
| **C: sim-stats** | `stat_projection.rs`, plus archetype tagging in `data/archetype_profiles.toml` | 7 | core types |
| **D: composite** | `asset_fit.rs`, `trade_acceptance.rs` | 5, 6 | A+B exposed signatures |

**Critical**: orchestrator pre-stages `lib.rs` with `Score`/`Reason` types AND empty `pub fn` stubs (with `todo!()` bodies) for every model so all 4 workers compile in isolation. Worker D depends on A+B *types* (function signatures) but not bodies — same pattern as M3.

## Acceptance — tied to user's stated expectations

```bash
# 1. Untouchable star — Luka cannot be acquired no matter what.
nba3k --save run.db trade propose \
    --from BOS --to LAL \
    --send "Jaylen Brown,Jayson Tatum,Derrick White" \
    --receive "Luka Dončić"
# Expected: Reject with reason "untouchable" (star_protection >= 0.85)

nba3k --save run.db trade propose \
    --from BOS --to LAL \
    --send "Jaylen Brown,Jayson Tatum,Derrick White,Sam Hauser" \
    --receive "Luka Dončić"
# Expected: STILL Reject. Star_protection short-circuits regardless of value.

# 2. Differential treatment — same offer, different teams.
# Contender (BOS, OKC, BOS) much less likely to give up its top-3 OVR
# vs rebuilder (UTA, WAS) which would.
nba3k --save run.db trade propose --from UTA --to BOS \
    --send "Lauri Markkanen" --receive "Sam Hauser,2027-BOS-1st"
# Expected: rebuilder-UTA accepts when picks are involved, even slight $ premium.

nba3k --save run.db trade propose --from BOS --to UTA \
    --send "Sam Hauser,2027-BOS-1st" --receive "Lauri Markkanen"
# Mirror of above — same trade from BOS POV. Expected: BOS accepts (Markkanen
# is a contender starter; both sides should agree, status: accepted).

# 3. Sim realism — stars produce star lines.
nba3k --save run.db sim-day 30
# Then read box scores aggregated:
sqlite3 run.db "
  SELECT p.name, AVG(line.pts), AVG(line.ast), AVG(line.reb)
  FROM games g, json_each(json_extract(g.box_score_json, '$.home_lines')) line_e,
       json_extract(line_e.value, '$') AS line, players p
  WHERE p.id = json_extract(line_e.value, '$.player')
  GROUP BY p.name ORDER BY AVG(line.pts) DESC LIMIT 10;
"
# Expected: top-10 PPG list dominated by named NBA stars (Luka, Giannis,
# Jokic, Tatum, SGA, Doncic, Jokic etc.), avg PPG of #1 ≥ 25.

# 4. Calibration harness shows healthier distribution.
nba3k --save run.db dev calibrate-trade --runs 500 --json
# Expected: < 50% rejects (M3 was 51%), counter zone broader,
# and reasons explain themselves (per-archetype breakdown reads sensibly).

cargo test --workspace  # all tests pass
```

## Implementation phases (within M4)

This is intentionally a **single-wave 4-worker sprint** since interfaces are pre-locked.

1. **Orchestrator wave 0** (~30 min, before workers): write `nba3k-models` skeleton (lib.rs + type stubs + `LeagueSnapshot` relocation to core + Cargo.toml + workspace add). Update `nba3k-trade` imports. Verify `cargo build --workspace` is clean.
2. **Workers wave 1** (parallel): A/B/C/D fill their assigned models + tests. Each works against the pre-locked signatures.
3. **Orchestrator wave 2**: rewire `nba3k-trade::evaluate` to call `trade_acceptance_score` (replaces threshold logic). Rewire `nba3k-sim::engine::statistical` to call `stat_projection_model`. Add `data/star_roster.toml` (curated list of 30 franchise tags). Add `data/realism_weights.toml`. Run M4 acceptance bash.
4. **M4 polish** (defer to M5-prep if time): rerun calibration harness with new models, tune weights, lock the TOML defaults.

## Risks / gotchas

1. **Bad input data persists**. BBRef ratings + age = 25 are still wrong. The named-star override file (`star_roster.toml`) is the v1 patch; long-term fix is a rating recalibration phase (M4-polish or M7-polish).
2. **Composition order matters**. Star protection short-circuits BEFORE value math — so a Cheapskate evaluator never even sees "what's Luka worth"; the answer is always "no". Worker D must implement the short-circuit in `trade_acceptance_score` *before* summing.
3. **Reason explosion**. Every model produces reasons, the composite can show 30+. Cap composite output to top-5 by `|delta|`.
4. **TOML weights file drift**. Default weights ship hardcoded inline; the TOML file is **only** an override layer. Don't fail to start if missing.

## Decision log (filled in during phase)

### Worker B — team_context + star_protection (2026-04-25)

**TeamMode classifier thresholds** (mirrors M3 trade-engine constants so behavior
is continuous when callers swap in `team_context_score`):

- `ROTATION_TOP_K = 9` — top-K-by-OVR slice used as the "rotation" age + keeper
  proxy.
- `young_age_threshold = 25.0`, `veteran_age_threshold = 29.0` (from
  `TeamContextWeights` defaults).
- `star_ovr = 88`, `keeper_ovr = 82`.
- `PLAYOFF_RANK = 8` for "strong seed", `BOTTOM_RANK = 11` for tank/rebuild
  signal floor.
- `TANK_WIN_PCT = 0.35`, `CONTEND_WIN_PCT = 0.55`. Win-pct is `None` when
  `games_played < 10` (pre-season fall-back via roster shape only).
- Continuous score mix:
  - `contend_score = 0.35·top_ovr + 0.30·standings + 0.20·cap_commitment +
    0.10·(1−age) + 0.05·history`.
  - `rebuild_score = 0.35·age + 0.30·(1−standings) + 0.20·(1−cap_commitment) +
    0.15·(1−top_ovr)`.
  - `win_now_pressure = 0.5·top_ovr + 0.4·(1−age) + 0.1·standings`.
- Discrete priority (first match wins): Contend (star + competing/strong seed)
  → Tank (veteran + losing + no star) → FullRebuild (young + ≤1 keeper +
  losing/weak seed) → SoftRebuild (young + ≥2 keepers) → Retool (everyone else).

**Franchise-tag list** (`data/star_roster.toml`, 24 teams, 28 players, lean
conservative):

| Team | Players |
|---|---|
| BOS | Jayson Tatum, Jaylen Brown |
| OKC | Shai Gilgeous-Alexander, Chet Holmgren |
| DEN | Nikola Jokić |
| MIL | Giannis Antetokounmpo |
| LAL | Luka Dončić, LeBron James |
| MIN | Anthony Edwards |
| PHI | Joel Embiid |
| PHO | Devin Booker, Kevin Durant |
| GSW | Stephen Curry |
| DAL | Anthony Davis |
| NYK | Jalen Brunson, Karl-Anthony Towns |
| CLE | Donovan Mitchell, Evan Mobley |
| ATL | Trae Young |
| NOP | Zion Williamson |
| MEM | Ja Morant |
| ORL | Paolo Banchero |
| SAS | Victor Wembanyama |
| SAC | Domantas Sabonis |
| IND | Tyrese Haliburton |
| HOU | Alperen Şengün |
| MIA | Bam Adebayo |
| POR | Scoot Henderson |
| CHA | LaMelo Ball |
| DET | Cade Cunningham |

Bottom-feeders / mid-table teams without an obvious franchise cornerstone
(BKN, CHI, TOR, UTA, WAS) deliberately have no entry — that's a feature.

**star_protection clamping**: raw deltas can sum > 1.0 (franchise_tag 1.0 +
top_ovr_bump 0.5 + young_ascending 0.3) so the final `value` is clamped to
`[0.0, 1.0]` while individual reasons are preserved verbatim. The Contend-mode
amplifier on `top_ovr_bump` is ×1.25; the FullRebuild dampener is ×0.5 plus a
small `team_mode_full_rebuild` floor pull (-0.05) so a clearout team's #1
never accidentally crosses `absolute_threshold` without an explicit franchise
tag.

### Worker C — stat_projection (2026-04-25)

**Archetype taxonomy** (10, in `data/archetype_profiles.toml` keyed by these
exact names; `infer_archetype()` chooses one for any Player):

| Archetype | Default usage | PTS/100 | REB/100 | AST/100 | Notes |
|---|---|---|---|---|---|
| `PG-distributor` | 0.22 | 22 | 5.5 | 13 | Haliburton / Lillard-lite |
| `PG-scorer` | 0.30 | 38 | 6.5 | 10 | Luka / SGA / Trae bucket |
| `SG-shooter` | 0.20 | 22 | 4.5 | 3.5 | Klay / Hauser / Hield |
| `SG-slasher` | 0.24 | 26 | 5 | 4.5 | Anthony Edwards / Booker |
| `SF-3andD` | 0.16 | 14.5 | 6 | 2.5 | Derrick White / Ariza-type |
| `SF-creator` | 0.28 | 30 | 8.5 | 8 | Tatum / LeBron / KD |
| `PF-stretch` | 0.20 | 19 | 8 | 2.5 | KAT lite / Sabonis lite |
| `PF-banger` | 0.20 | 18 | 12 | 2 | Drummond / classic 4 |
| `C-finisher` | 0.22 | 22 | 14 | 2.5 | Embiid / Adebayo / Sengun |
| `C-stretch` | 0.22 | 22 | 11 | 4 | Jokic / Wemby / KAT |

`infer_archetype()` picks by primary position then a single rating-spread
tiebreaker (playmaking-vs-finishing for guards, 3PT-vs-paint for bigs, etc.) —
deterministic and roundable from sparse rating data.

**Stat-projection formula** (per game, per stat):

```
on_court_team_poss = team_pace × (minutes / 48)
poss_scale         = on_court_team_poss / 100
usage_factor       = (usage_share / archetype.default_usage)^0.8   # clamp [0.05, 3.0]

mean_PTS = (pts_per_100 + star_uplift_pts? + creator_reb_ast_bonus?)
          × poss_scale × usage_factor × injury_scale
mean_REB = (reb_per_100 + star_uplift_reb? + creator_reb_bonus?)
          × poss_scale × injury_scale         # NOT usage-scaled
mean_AST = ... usage-scaled, parallel to PTS
mean_STL/BLK = floor-time scaled (no usage)
```

Each stat is sampled from `Normal(mean, sigma)` with a hybrid sigma:
`sigma = max(sqrt(mean) × 1.2, mean × 0.30)`. The Poisson-floor branch
dominates at low means (single-digit production has many zeros); the
proportional branch gives real game-to-game volatility for stars.

Shooting line is reconciled to the sampled PTS so box arithmetic balances:
sample 3PA/FTA, then derive 3PT-made and FT-made via binomial-normal
approximations using the player's `shooting_3` / `shooting_mid` ratings, and
fill the rest with 2PT-made.

**Star uplift formula**:

A player is "star-active" iff:
  (a) franchise-tagged in `data/star_roster.toml` for their team abbrev, AND
  (b) `Player.overall ≥ StatProjectionWeights.star_uplift_threshold_ovr`
       (default 88).

When star-active, additive bumps to per-100 baselines:
  - `pts_per_100 += star_uplift_pts` (default 4.0)
  - `reb_per_100 += star_uplift_reb` (default 1.5)
  - `ast_per_100 += star_uplift_ast` (default 1.5)

**Primary-creator triple-double bonus** — the user's "Luka 三双频率高" intuition:

```
creator_excess = max(usage_share - 0.25, 0)
reb_per_100   += creator_excess × 12   # +1.2 reb/100 at usage 0.30
ast_per_100   += creator_excess × 14   # +1.4 ast/100 at usage 0.30
```

The bonus is applied to ALL high-usage players (tagged or not) — this models
the fact that ball-dominant initiators rebound more (long-rebound recoveries)
and assist more (they finish possessions). At usage 0.18 (a 3-and-D wing),
the bonus is zero, so non-stars don't accidentally cross the triple-double
floor.

**Injury throttle**: pure multiplicative scale on every mean — DayToDay 0.75,
ShortTerm 0.55, LongTerm 0.30, SeasonEnding 0.10. The sim engine should
already cull season-enders from rotations; this is the belt-and-suspenders
for callers that force a projection through (e.g., previewing a "questionable"
player's box).

**Determinism**: `project_player_line` consumes `&mut dyn RngCore` in a fixed
order (PTS → REB → AST → STL → BLK → TOV → 3PA → FTA → 3PT-makes → FT-makes →
then any 2PT-attempt fallback). Same seed + same input → identical
`PlayerLine` (test pinned).

**Acceptance check** (200-game sims, ChaCha8 seed):

| Player | min | usage | tag | avg PTS | TD% |
|---|---|---|---|---|---|
| Star PG (92 OVR, PG-scorer, LAL) | 36 | 0.30 | tagged | 22-40 | — |
| Star PG (95 OVR, PG-scorer, LAL) | 38 | 0.32 | tagged | — | ≥ 5% |
| Bench guard (72 OVR, SG-shooter) | 14 | 0.10 | untagged | ≤ 12 | — |
| Wing (78 OVR, SF-3andD) | 32 | 0.18 | untagged | — | ≤ 1% |

### Worker A — player_value + contract_value (2026-04-25)

**OVR→baseline curve** (positional baseline, talent-only, no premium):
- Power curve: `baseline_dollars = ((ovr - 50) / 49) ^ 2.6 * 55 * pos_mul`
- Anchors at the SF baseline (pos_mul = 1.0): OVR 70 → ~$5M, OVR 75 → ~$10M,
  OVR 80 → ~$20M, OVR 85 → ~$30M, OVR 88 → ~$38M, OVR 92 → ~$50M,
  OVR 95 → ~$58M, OVR 99 → ~$80M.
- Positional multipliers: PG 1.04, SG 1.00, SF 1.00, PF 0.98, C 0.96. Lead
  guards take a small premium for playmaking scarcity; bigs take a small
  discount at the floor (replacement-level bigs are easier to find).
- The recalibration is intentionally **lower** than the M3 trade-engine seed
  so contract-overpay tests bite. Star tier sits separately on top.

**Star premium** (added on top of baseline, not a multiplier):
- `premium_dollars = 2.0 * (ovr - threshold)^2 * traits.star_premium`
- threshold = 88 by default. With default traits (`star_premium = 1.0`):
  OVR 89 → +$2M, OVR 92 → +$32M, OVR 95 → +$98M, OVR 99 → +$242M.
- StarHunter (`star_premium = 1.6`) multiplies these. Quadratic shape gives
  the required nonlinearity above the threshold (the gap between OVR 87 and
  89 is several times the baseline 86→87 step).

**Age curve** (multiplicative on baseline; peak at `weights.age_peak = 27`):
- Pre-peak: `mul = 1.05 - 0.012 * (peak - age)` clamped at 0.85 below age 19.
- Post-peak: cumulative annual decline by position (guards decline fastest):
  - PG/SG: -0.040/yr through 30, -0.090/yr through 33, -0.12/yr after.
  - SF:    -0.035/yr through 30, -0.085/yr through 33, -0.11/yr after.
  - PF/C:  -0.025/yr through 30, -0.060/yr through 33, -0.10/yr after.
- Sized so a 33-yo SF sits ≥30% below their 28-yo same-OVR self.
- A separate `gm_age_preference` reason layers a ±10% bias from
  `traits.age_curve_weight` (Rebuilder/Analytics tilt young, OldSchool tilt
  veteran).

**Contract surplus formula** (`contract_value`):
- `expected_market = talent_value(player) * years_remaining` — talent valued
  with neutral `PlayerValueWeights` so salary_aversion and loyalty do not
  leak into the talent reference.
- `actual_salary = -(sum_y salary_y * (1 - 0.08)^years_out) * salary_aversion`
- `option_value`: each player-option year subtracts `option_value_player`
  fraction (default 10%) of that year salary; team-option years credit
  `-option_value_team` (default +5% credit).
- `expiring_premium = years_remaining[0].salary * 0.15` — only when exactly
  one year remains.

**Loyalty bonus**:
- Applied **only** when `evaluator == player.team`. Magnitude is
  `baseline * traits.loyalty.clamp(0,1) * weights.loyalty_bonus_default`
  (default 0.20). Loyalist GMs (loyalty=0.6) see a meaningful bump on
  homegrown players; default GMs (loyalty=0.1) see a small one.

**Reason emission**: the Score always returns ≥4 reasons for an on-roster
player with a contract (positional_baseline, age_curve, contract_burden,
loyalty_bonus, plus optional gm_age_preference and star_premium). Reasons
are sorted by |delta| desc; callers `top_k(3)` for display.

**Named-star override is intentionally NOT applied here**. Per the phase
brief, the `star_roster.toml` override is consumed at composite time
(Worker D / `trade_acceptance`) where the franchise-tag flag also gates the
star_protection short-circuit. Keeping it out of `player_value` means the
base model stays a pure OVR/age/contract math layer — a Cheapskate
evaluator never accidentally picks up a star override through value math.

### Worker D — composite-modeler (2026-04-25)

**Logistic curve constants** (in `weights::TradeAcceptanceWeights`):
- `accept_probability_intercept = 0.0` — neutral midpoint at net_pct = 0.
- `accept_probability_slope = 8.0` — converts a +5%/−15% net_pct band
  into roughly the 0.6 / 0.2 verdict thresholds we want.
- `gullibility_noise_pct = 0.08` — gaussian stddev = `0.08 × gullibility ×
  slope`, applied additively in logit space (so a Wildcard GM at
  gullibility 0.7 has stddev ≈ 0.45 in logits).
- `top_k_reasons = 5`.

**Verdict thresholds** (from sampled probability):
- `p ≥ 0.55` → `Verdict::Accept`
- `p ≤ 0.20` → `Verdict::Reject(InsufficientValue)`
- otherwise  → `Verdict::Counter(offer.clone())`

Defined as `pub const ACCEPT_PROBABILITY` / `REJECT_PROBABILITY` in
`nba3k_models::trade_acceptance` so the integration layer can reference
them.

**Short-circuit conditions** (the user's headline behavior):
- For every player on the evaluator's outgoing side, call
  `star_protection`. If any returned `Score.value ≥
  weights.star_protection.absolute_threshold` (default `0.85`):
  - Return `TradeAcceptance { probability: 0.0, verdict:
    Verdict::Reject(RejectReason::Other("untouchable".into())),
    net_value: Cents::ZERO, reasons: ["untouchable star" + forwarded
    star_protection sub-reasons], commentary: "{Player} is not on the
    table." }`.
  - Runs BEFORE any value math (player_value / contract_value /
    asset_fit). Even an offer worth 5 superstars can't unlock a Luka.
- Players whose protection lands in `[premium_threshold,
  absolute_threshold)` (default `[0.60, 0.85)`) are NOT short-circuited
  but apply a soft penalty up to 25% of their `player_value`, scaled
  linearly across the band. This produces the "you'd have to blow us
  away" zone without falling back to a binary threshold.

**Asset_fit components** (`crates/nba3k-models/src/asset_fit.rs`):
- Three components, each in cents and scaled by the incoming player's
  positional baseline so a star fit bonus dwarfs a role-player one:
  - `positional_need`: linear from +20% (no rotation player at the
    position) through 0 at 2 same-position rotation members down to
    −20% with 4 in rotation.
  - `skill_overlap`: −15% × number of high-OVR (≥80) same-position
    rotation players.
  - `rotation_saturation`: flat −10% if the receiving team's top-8
    already has 8 players with OVR ≥ incoming.overall.
- Top-8 by OVR is the rotation definition. A player whose
  `secondary_position` matches the incoming's primary still counts
  toward overlap.

**Mocking strategy for composite tests**
(`crates/nba3k-models/tests/trade_acceptance_basic.rs`):
- `trade_acceptance_with_providers(...)` exposes the composition entry
  with closure-injected `player_value`, `contract_value`,
  `star_protection`, `team_context`, and `asset_fit`. Tests use
  deterministic stubs keyed off `Player.overall` so they don't depend
  on Worker A/B function bodies. Production wiring uses
  `ValueProviders::real(&ComposeWeights)`.
- Coverage: untouchable short-circuit (regardless of incoming volume),
  believable counter zone, filler-for-star reject, premium-zone makes
  trades harder, Contend-vs-FullRebuild raises/lowers the bar,
  reason-list cap and ordering, and seed determinism.
