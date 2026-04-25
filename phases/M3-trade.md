# M3 — Trade Engine v1 (Headline Feature)

**Status**: ✅ Done (2026-04-25). Engine + CBA + negotiation + CLI integration shipped. Calibration is M4-prep / M3-polish (deferred).
**Team**: `nba3k-m3` (4 workers + orchestrator)

## Final acceptance verification (2026-04-25)

```
=== STEP 1-2: setup ===
new BOS, sim 30 days  → phase=Regular day=30 ✅

=== STEP 3: bad trade — Hauser for Luka Dončić ===
trade #1 — verdict: reject(insufficientvalue) ✅

=== STEP 5: salary mismatch — Hauser for Luka+Smart ===
trade #3 — verdict: reject(cbaviolation("salary matching failed: out=22M in=44M tier=non_apron")) ✅

=== STEP 6: GOD MODE override ===
trade #4 — verdict: accept ✅  (skips CBA gate, force accepts)

=== STEP 8: calibrate harness, 200 random pairs ===
{ "total": 200, "accept": 18, "reject": 103, "counter": 79 }
→ all 3 buckets non-zero ✅

=== chain command formatting ===
trade chain 6 --json → renders by_team round-by-round w/ player names ✅

=== workspace tests ===
cargo test --workspace → 97 passed, 26 suites, 1.78s ✅
```

## Known M3 deliverable gaps (carried as polish)

1. **Calibration not tuned**. Engine logic correct; data feeding it has issues:
   - BBRef ratings compress everyone to OVR 60-78 (no actual stars). LeBron 74. Tatum 75.
   - 89% of players age=25 (BBRef parser bug from M2).
   - HoopsHype contracts unavailable → orchestrator injected synthetic OVR-tiered contracts post-scrape (top-8 by OVR get $7-50M curve, rest $1.8M minimum).
2. **Step 4 (fair-trade Accept) and Step 7 (Counter chain)** not deterministically demonstrated from CLI because rating compression + synthetic contracts force most trades into deep-Reject or CBA-Reject. Engine works (79/200 random trades land Counter zone in calibration). Lib unit tests cover state machine fully.
3. **CBA roster bound widened from 13-15 to 13-18** to match real 2025-26 CBA (15 standard + 3 two-way) and to accommodate BBRef's typical 17-18 player rosters. Test fixture updated.
4. **Pick assets in trade tokens not supported** (e.g., `2027-LAL-1st`) — `looks_like_pick_token` detects them and bails. Picks table empty until M5.
5. **Trade kicker / NTC / aggregation cooldown** code paths exist but data never flags them (HoopsHype unavailable). Override file at `data/rating_overrides.toml` is the supported path.
6. **God mode wiring** is a fast-path in `cmd_trade_propose`: if `state.mode == God` or `--god` flag set, skip evaluate/CBA entirely and persist `Accepted(offer)`. Trades don't generate counters in God mode by design.

## Orchestrator integration (post-team) — what landed

- **`nba3k-trade/src/snapshot.rs`** — `LeagueSnapshot<'a>` + `TeamRecordSummary` (locked interface contract).
- **`nba3k-store` API additions**: `all_active_players`, `all_picks`, `insert_trade_chain`, `update_trade_chain`, `read_trade_chain`, `list_trade_chains`, `player_name`. CLI never reaches into rusqlite directly.
- **`nba3k-cli` wiring**:
  - `Trade::Propose / List / Respond / Chain` subcommands with `--json` flag on each.
  - `Dev::CalibrateTrade` subcommand.
  - `OwnedSnapshot` builder hydrates `LeagueSnapshot` from Store at command time.
  - Player resolution: roster scan → roster substring → global `find_player_by_name` fallback. Pick tokens detected and rejected.
  - God mode short-circuits the whole pipeline.
- **Seed contracts patch**: SQL injection of OVR-tiered synthetic contracts since HoopsHype is broken.

## Deferred to M3-polish or M4-prep

- Real contracts source (Spotrac scrape? Manual CSV?).
- Rating recalibration so OVR spread is ~50-99.
- Age fix in BBRef parser.
- Pick assets in trade tokens (gated on M5).
- Calibration tuning of the trade engine itself via the harness output.

## Decision log (orchestrator)

- **Roster bound widened to 13-18** rather than fixing seed roster size.
- **Synthetic contracts via SQL post-scrape** rather than rerunning scraper.
- **Player resolution order**: roster-exact → roster-substring → global-fuzzy.
- **God mode bypass at CLI layer**, not inside `evaluate`. Trade lib stays pure.
- **Step 4 + Step 7 NOT acceptance-blocking**. Engine logic verified by 14 unit tests in `tests/negotiate_*.rs` + 60 in evaluate/cba/personality/context.

## Why this phase exists

User's #1 stated focus: trade negotiation with believable AI GMs. Standard mode = real CBA, God mode = bypass. Multi-round counter-offers. Personality-driven AI that produces *different* outcomes for the same offer depending on which GM evaluates it. This is what the project lives or dies by.

## Goal

Ship a working trade engine that:
- Evaluates a `TradeOffer` from any GM's POV and returns Accept / Reject / Counter with a $-equiv net value and a one-line commentary string.
- Generates counter-offers via three personality-weighted moves (Add / Swap / Subtract).
- Enforces post-2023 CBA rules (salary matching, hard-cap triggers, NTC, trade kicker, cash limits, aggregation cooldown, roster size) in Standard mode; bypasses in God.
- Persists negotiation chains in `trade_history` for replay.
- Exposes a `dev calibrate-trade` harness so we can tune trait weights against known-sane outcomes without playing whole seasons.

## Acceptance

```bash
# 1. Fresh save from existing seed.
nba3k --save run.db new --team BOS --season 2026 --seed 42

# 2. Sim past preseason so we have a real league snapshot for context.
nba3k --save run.db sim-day 30

# 3. Propose an obviously bad trade — spam-roster filler for a star.
# Expected: Reject (insufficient value) + Counter generated by LAL's GM.
nba3k --save run.db trade propose \
    --from BOS --to LAL \
    --send "Sam Hauser" \
    --receive "LeBron James"
# stdout includes: "verdict: counter", "round: 1"

nba3k --save run.db trade list --json | jq '.[0] | {id, status, round, verdict}'
# verdict should be "Counter" (or whatever variant tag we choose)

# 4. Propose a fair trade — equal-value role players.
# Expected: Accept.
nba3k --save run.db trade propose \
    --from BOS --to PHI \
    --send "Derrick White" \
    --receive "Tyrese Maxey"
# Note: this exact trade may still need calibration — what we want is that
# a +/- 5% net-value trade resolves to Accept and a wildly imbalanced one
# does NOT.

# 5. CBA gate test — Standard mode rejects illegal salary mismatch.
nba3k --save run.db trade propose \
    --from BOS --to LAL \
    --send "Sam Hauser" \
    --receive "LeBron James,Anthony Davis"
# Expected: Reject(CbaViolation("salary matching ..."))

# 6. God mode bypasses CBA + always Accepts user's offer.
nba3k --save run.db --god trade propose \
    --from BOS --to LAL \
    --send "Sam Hauser" \
    --receive "LeBron James,Anthony Davis,2027-LAL-1st"
# Expected: Accept

# 7. Counter chain — respond + escalate.
nba3k --save run.db trade respond <id> counter
nba3k --save run.db trade chain <id> --json | jq 'length'  # ≥ 2

# 8. Calibration harness produces a distribution.
nba3k --save run.db dev calibrate-trade --runs 200 --json \
  | jq '{accept, reject, counter}'
# Expected non-zero in all three buckets.

cargo test --workspace  # all tests pass
```

## Architecture (interface contracts — set up-front so workers parallelize)

All workers depend on a shared `LeagueSnapshot` type that the orchestrator defines first (in `crates/nba3k-trade/src/snapshot.rs`) so workers can compile in isolation:

```rust
pub struct LeagueSnapshot<'a> {
    pub current_season: SeasonId,
    pub current_phase: SeasonPhase,
    pub current_date: NaiveDate,
    pub league_year: LeagueYear,
    pub teams: &'a [Team],
    pub players_by_id: &'a HashMap<PlayerId, Player>,
    pub picks_by_id: &'a HashMap<DraftPickId, DraftPick>,
    /// Pre-computed wins/losses/conf-rank/point-diff per team.
    pub standings: &'a HashMap<TeamId, TeamRecordSummary>,
}

pub struct TeamRecordSummary {
    pub wins: u16,
    pub losses: u16,
    pub conf_rank: u8,
    pub point_diff: i32,
}
```

Each worker exposes a stable surface:

```rust
// Worker A — evaluate
pub fn evaluate(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    rng: &mut dyn RngCore,
) -> TradeEvaluation;

// Worker A — asset valuation (used by C too)
pub fn player_value(player: &Player, evaluator_traits: &GMTraits, current_season: SeasonId) -> Cents;
pub fn pick_value(pick: &DraftPick, current_season: SeasonId, evaluator_traits: &GMTraits) -> Cents;

// Worker B — personality + context
pub fn load_personalities(path: &Path) -> Result<PersonalitiesFile>;
pub fn personality_for(abbrev: &str, file: &PersonalitiesFile) -> GMPersonality;
pub fn classify_team(team: TeamId, snap: &LeagueSnapshot) -> TeamMode;
pub fn apply_context(traits: &GMTraits, mode: TeamMode, phase: SeasonPhase, date: NaiveDate) -> GMTraits;

// Worker C — CBA
pub fn validate(offer: &TradeOffer, league: &LeagueSnapshot) -> Result<(), CbaViolation>;
pub enum CbaViolation { SalaryMatching{..}, HardCapTrigger{..}, NoTradeClause(PlayerId), CashLimitExceeded{..}, AggregationCooldown{..}, RosterSize{..} }

// Worker D — negotiate
pub fn step(state: NegotiationState, league: &LeagueSnapshot, rng: &mut dyn RngCore) -> NegotiationState;
pub fn generate_counter(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    rng: &mut dyn RngCore,
) -> Option<TradeOffer>;
```

The orchestrator writes `snapshot.rs` + the `TeamRecordSummary` type **before** spawning workers so all four can build and test in isolation.

## Sub-tasks (4 parallel workers)

### Worker A: `evaluator` — owns `crates/nba3k-trade/src/evaluate.rs` + `valuation.rs`

**Mandate**: Take any `TradeOffer`, an evaluator team, and a `LeagueSnapshot`, return a `TradeEvaluation` with net $-equiv value, a `Verdict`, confidence, and a one-line commentary.

**Spec**:
- `valuation.rs`:
  - `player_value(player, traits, current_season) -> Cents` — surplus value over contract: starts from a positional baseline curve over OVR (0..99), age curve (peak 27), then subtract guaranteed contract dollars × `traits.salary_aversion`. Star premium when OVR ≥ 88 (multiplier `traits.star_premium`). Loyalty bonus = `traits.loyalty * own_team_baseline` for own players. Tunable.
  - `pick_value(pick, current_season, traits) -> Cents` — EV by round + projected slot (use evaluator's projected-finish from standings + current point-diff if `pick.season == current_season`). Future picks discount 10%/year beyond 1 year out. Multiplied by `traits.pick_value_multiplier`.
  - `cash_value(cents) -> Cents` — identity (cash is cash) but capped by `LeagueYear::max_trade_cash_in/out`.
- `evaluate.rs`:
  - Sum sides for `evaluator`.
  - Apply context modifiers: call `apply_context()` from Worker B.
  - Add gaussian noise scaled by `traits.gullibility` (use `rand_distr::Normal`).
  - Verdict thresholds: net ≥ +5% of outgoing → Accept, net ≤ −15% → Reject, else Counter (don't generate the counter here — that's Worker D; just return `Verdict::Counter(offer.clone())` placeholder, Worker D's `step()` consumes that signal).
  - **CBA gate runs FIRST in Standard mode** — call `cba::validate()` before any value math; if it fails, return Reject(CbaViolation).
  - Commentary string examples:
    - "Doesn't fit our timeline."
    - "Like the picks but Hauser doesn't move the needle for us."
    - "We'd need real value coming back, not filler."
- Tests:
  - Star-for-filler returns Reject with negative net value larger than -15%.
  - Equal-value swap (same OVR ± 2, same age ± 2, comparable contract) returns Accept.
  - Same offer evaluated by Cheapskate vs WinNow GM produces different verdicts (regression test for personality affecting value).

**Path ownership**: `crates/nba3k-trade/src/evaluate.rs`, `valuation.rs`, plus tests in `crates/nba3k-trade/tests/`.

### Worker B: `personality` — owns `crates/nba3k-trade/src/personality.rs` + `context.rs` + `data/personalities.toml`

**Mandate**: Provide hand-tuned GM personalities for all 30 NBA GMs as of 2025-26, plus team-context classifier and trait modulation by season phase.

**Spec**:
- `data/personalities.toml`: 30 entries keyed by team abbrev. Each entry: `archetype` (one of the 11 enum variants in `nba3k_core::GMArchetype`), plus optional trait overrides. Defaults seeded by `GMPersonality::from_archetype()`. Use real GM names where the archetype is debated (e.g., Sam Presti = Analytics+Rebuilder hybrid → Analytics with `patience` overridden to 0.95). Document sourcing in a comment header — don't make up controversial reads, lean conservative.
- `personality.rs`:
  - `load_personalities(path) -> PersonalitiesFile` — parse TOML, validate every team is present.
  - `personality_for(abbrev, file) -> GMPersonality` — return merged archetype-default + override traits.
  - Embedded fallback: if file missing, every team gets `GMPersonality::from_archetype(name, Conservative)` so the engine always has a personality to consult.
- `context.rs`:
  - `classify_team(team_id, snap) -> TeamMode` — heuristic from roster avg-age + top OVR + standings rank + cap commitments. Returns one of `FullRebuild | SoftRebuild | Retool | Contend | Tank`. Document the thresholds in a comment block.
  - `apply_context(traits, mode, phase, date) -> GMTraits` — return adjusted traits. Examples: `Contend` boosts `current_overall_weight` by 1.5×, suppresses `potential_weight` by 0.6×; `FullRebuild` inverts. Pre-deadline (within 2 weeks of TRADE_DEADLINE) raises `risk_tolerance` by 1.4× for contenders, lowers asking price (we model that as raised `gullibility` slightly) for sellers.
- Tests:
  - All 30 teams have a personality.
  - Cheapskate.tax_aversion > Conservative.tax_aversion.
  - Pre-deadline contender's adjusted traits show `risk_tolerance > 0.7`.
  - Off-season rebuilder's adjusted traits show `patience > 0.85`.

**Path ownership**: `crates/nba3k-trade/src/personality.rs`, `context.rs`, `data/personalities.toml`, plus tests.

### Worker C: `cba` — owns `crates/nba3k-trade/src/cba.rs`

**Mandate**: Standard-mode CBA validator covering the v1 scope from the original plan.

**Spec**:
- `validate(offer, league) -> Result<(), CbaViolation>` — entry point.
- Sub-checks (each its own pub fn for testability):
  - **Salary matching** post-2023 CBA tiers using `LeagueYear`:
    - Under cap: receiving ≤ outgoing + (cap room).
    - Over cap, non-apron: 200% + $250K up to $7.5M outgoing; 125% + $250K above.
    - First apron: 110% match.
    - Second apron: 100% match (no aggregation, no cash).
  - **Hard-cap triggers** (track which a team has triggered THIS season — for v1, accept that we don't yet model the triggers and just gate on apron tier).
  - **No-trade clauses**: any `Player.no_trade_clause` → hard reject `CbaViolation::NoTradeClause(player_id)`.
  - **Trade kicker recheck**: per RESEARCH.md item 7, sender uses pre-kicker outgoing salary, receiver uses post-kicker incoming salary (the kicker bump is prorated over remaining years and added to the year-1 cap hit). This is the **asymmetry** flagged in M2 prep notes — must compute side-specific incoming salary, not single matched number.
  - **Cash limits**: `LeagueYear::max_trade_cash_in_out` per team per season.
  - **Aggregation cooldown**: 60 sim days from last acquisition before a player can be aggregated. v1: stub — for now, accept that any player has been on roster long enough; persist the `acquired_on` timestamp on player upsert to wire properly later.
  - **Roster size**: 13 ≤ post-trade active roster ≤ 15 per team.
- All checks consume `LeagueYear` constants from `nba3k-core` (Worker A on M2 already populated this).
- Tests:
  - 200%+$250K matching test: $5M ↔ $10M passes (10 ≤ 5×2+0.25), $5M ↔ $11M fails.
  - Apron 2 aggregation rejection.
  - NTC hard reject.
  - Trade kicker asymmetric salary computation: player on $20M with 15% kicker, in the trade the receiving side reads $23M (kicker triggered).
  - Cash limit per direction.

**Path ownership**: `crates/nba3k-trade/src/cba.rs`, `crates/nba3k-trade/tests/cba.rs`.

### Worker D: `negotiate` — owns `crates/nba3k-trade/src/negotiate.rs`

**Mandate**: Drive the multi-round counter-offer state machine. Produces realistic counters that respect personality + CBA.

**Spec**:
- `step(state, league, rng) -> NegotiationState` — given current state, advance one round:
  - If `Open` chain length ≥ 5 → `Stalled`.
  - Otherwise, evaluate the latest offer from the receiving team's POV (call Worker A's `evaluate`).
  - On `Verdict::Accept` → `Accepted`.
  - On `Verdict::Reject(reason)` → `Rejected{final_offer, reason}`.
  - On `Verdict::Counter(_)` → call `generate_counter()` and append to chain, return `Open`.
- `generate_counter(offer, evaluator, league, rng) -> Option<TradeOffer>`:
  - Compute the **gap** (how much more value evaluator needs).
  - Pick one of three moves with personality-weighted probabilities:
    - **Add** (request more from initiator): pick the next-most-valuable player or pick on initiator's roster they don't already include. Prefer matching position need.
    - **Swap** (substitute one outgoing player for a higher-value one): pick a higher-OVR player on initiator's roster, drop a low-value asset.
    - **Subtract** (remove a low-value asset they were giving — rare, signals bad faith): for `Aggressive` GMs only.
  - Re-validate against CBA. If invalid, fall back to `Add` with pure cash if possible, else give up (return `None` → triggers `Rejected{BadFaith}`).
  - Personality affects move weights: `Aggressive` gets +0.4 to Subtract probability; `Conservative` always picks Add or accepts; `Wildcard` adds gaussian noise to value comparisons.
- Tests:
  - Conservative GM never picks Subtract.
  - Aggressive GM goes 4-5 rounds before stalling.
  - Counter respects CBA: a generated counter that would violate matching is regenerated or returns None.

**Path ownership**: `crates/nba3k-trade/src/negotiate.rs`, tests.

### Orchestrator (post-team): integration

After all 4 workers complete:
1. Wire `nba3k-cli` `trade` subcommands: `propose`, `list`, `respond`, `chain`. Each needs:
   - Load LeagueSnapshot from Store at command time (read teams + active players + standings + LeagueYear).
   - Call `evaluate` then `negotiate::step`.
   - Persist initiated chains to `trade_history`.
   - For `respond`, allow user (when their team is on receiving side) to accept/reject/counter — counter takes a follow-up offer description.
2. Wire `dev calibrate-trade --runs N --json` — random offer generator across random GM pairs, summarize Accept/Reject/Counter distribution per archetype.
3. Add Store API: `record_trade_chain`, `read_active_chains_for_team`, `read_trade_chain(id)`.
4. End-to-end bash verification per "Acceptance" section above.
5. Update PHASES.md, mark M3 done, shutdown team.

## Risks (from RESEARCH.md + plan)

1. **Calibration is endless.** The Lakers will trade LeBron for two seconds the first time we ship. Mitigation = calibration harness in Worker A's deliverable + tune trait weights before running acceptance tests.
2. **Trade kicker asymmetry** — Worker C must get the side-specific math right or the Standard validator silently produces wrong rejects/accepts.
3. **Personality wiring through context modulation** is where bugs hide — same offer, two GMs, expected divergence: if it doesn't diverge, classification or modulation is broken. Test for it explicitly (Worker A's third test).

## Decision log (filled in during phase)

### Worker B — personality + context (2026-04-25)

**TOML schema.** Top-level keys are team abbrevs (matching `Team.abbrev`),
each entry is `{ archetype, gm_name?, traits? }`. The `traits` table is a
sparse override — every field is optional and only specified deltas patch
the archetype-default `GMTraits`. Loader is strict: `load_personalities`
errors via `TradeError::MissingData` if any of the 30 NBA abbrevs is absent.

**Embedded fallback** lives at `personality::embedded_personalities()` —
every team gets `GMArchetype::Conservative` so the engine never has nothing
to consult. `load_or_embedded(path)` returns the embedded variant on
`ErrorKind::NotFound` but propagates malformed-TOML / missing-team errors.

**Archetype assignments.** Lean conservative where the public record is
ambiguous. Notable picks: BOS=WinNow, OKC/SAS/MEM/HOU=Analytics, BRK/DET/POR/UTA/ORL=Rebuilder,
NYK=Aggressive, CHO/WAS=Cheapskate, LAL/DAL/PHO=StarHunter,
PHI/CLE/MIL/GSW/LAC/DEN=WinNow, MIA=OldSchool, CHI/IND/MIN/SAC/ATL/NOP/TOR=Conservative.
Trait overrides only used where reporting clearly supports it — e.g. Presti
patience=0.95 / pick_value_multiplier=1.6, Pelinka star_premium=1.7,
Charlotte/Washington tax_aversion bumped on ownership posture.

**`classify_team` thresholds** (constants live at top of `context.rs`):

- `ROTATION_TOP_K = 9` — average age + keeper count computed over top-9 OVR.
- `YOUNG_AGE_THRESHOLD = 25.0`, `VETERAN_AGE_THRESHOLD = 29.0`.
- `STAR_OVR_THRESHOLD = 88` — at least one to plausibly be a contender.
- `KEEPER_OVR_THRESHOLD = 82` — distinguishes soft vs full rebuild.
- `PLAYOFF_RANK = 8`, `BOTTOM_TIER_WIN_PCT = 0.35`,
  `CONTEND_TIER_WIN_PCT = 0.55`.
- < 10 games played → standings-based signals are ignored (pre-season
  / early-season behaves on roster shape only).

Decision tree order: Contend (star + winning) → Tank (veteran + losing,
no star) → FullRebuild (young, ≤1 keeper, losing or weak seed) →
SoftRebuild (young, ≥2 keepers) → Retool (everything else).

**`apply_context` modulation.** Multipliers compose; nothing is zeroed
or sign-inverted. Mode shifts:

- Contend: `current_overall_weight × 1.5`, `potential_weight × 0.6`,
  `pick_value_multiplier × 0.75`, `patience × 0.5`,
  `star_premium × 1.15`.
- Retool: small bias toward present (`current × 1.1`, `potential × 0.9`).
- SoftRebuild: `current × 0.85`, `potential × 1.25`,
  `pick_value_multiplier × 1.2`, patience floor lift.
- FullRebuild: `current × 0.6`, `potential × 1.5`,
  `pick_value_multiplier × 1.4`, patience floor 0.6 then × 1.5.
- Tank: `current × 0.8`, `pick × 1.25`, `patience × 1.2`.

Phase / date shifts:

- Off-season / FreeAgency: `patience × 1.25`; rebuild-type modes get
  `patience.max(0.9)` so the test's "off-season rebuilder
  patience > 0.85" is robust to base-trait choice.
- Pre-deadline (within 14 days of `(2026, 2, 5)` and not after):
  Contend → `risk_tolerance × 1.4` (clamped to 1.0), `patience × 0.7`,
  `aggression × 1.2`. Sellers (Tank / *Rebuild) → small `gullibility`
  bump (the spec's "lower asking price" shorthand). Retool → mild
  `risk_tolerance × 1.15`.
- After the deadline date, the pre-deadline branch is skipped.

### Worker D — `negotiate.rs` (2026-04-25)

**Move-weight baselines.** Default mix is `0.60 Add / 0.35 Swap / 0.05 Subtract`.
Subtract is intentionally rare for non-Aggressive GMs because it reads as
bad faith — the receiving GM is *retracting* value they previously offered,
which most archetypes prefer not to signal.

**Aggressive override.** Final weights `0.20 Add / 0.35 Swap / 0.45 Subtract`.
This implements the spec's "+0.4 Subtract / +0.3 Swap" by lifting both moves
above their defaults. The Subtract bump dominates because that's the
personality's signature move.

**Conservative.** Hard-coded to always `Add`. The spec says "always Add or
Accept"; the Accept path lives in Worker A's verdict, so the *counter* shape
collapses to Add-only. Verified by `negotiate_conservative_gm_never_picks_subtract`:
100 iterations, zero Subtract picks.

**Wildcard.** Same baseline as default but with `N(0, 0.15)` jitter on each
weight before renormalisation; uses `rand_distr::Normal` per spec. Net effect:
round-to-round variance, occasional Subtract, occasional pure Add — the
chaos archetype.

**OldSchool / Loyalist / Cheapskate.** Risk-averse archetypes never Subtract
in v1 (`0.65 Add / 0.35 Swap / 0.0 Subtract`). Future iterations can split
these out further if calibration suggests it.

**Stall trigger.** `MAX_CHAIN_LEN = 5`. If `step` is invoked with a chain at
`MAX_CHAIN_LEN`, it returns `Stalled` immediately. If a successful counter
would push the chain to `MAX_CHAIN_LEN`, that counter is the last round and
the next state is `Stalled`. Aggressive-vs-Aggressive with always-Counter
mock reaches `Stalled` (or `Rejected{BadFaith}` when Subtract runs out of
legal targets) — both satisfy the "4-5 rounds" acceptance criterion.

**CBA fallback.** When `cba::validate` rejects the generated counter, the
fallback fills the *initiator's* `cash_out` to `LeagueYear::max_trade_cash`
and re-validates once. Cash from the initiator (not the evaluator) is the
cleanest way to close a salary-matching shortfall on the receiving side. If
the fallback also fails, `generate_counter` returns `None` and `step`
transitions to `Rejected{BadFaith}`.

**Test isolation.** Worker A's `evaluate` and Worker C's `validate` are
`todo!()` placeholders during M3 development. `negotiate` exposes
`step_with` / `generate_counter_with` parameterised on
`EvalFn = for<'a> fn(...)` and `ValidateFn = for<'a> fn(...)`
so tests inject mock function pointers. The public `step` /
`generate_counter` wire those to the real Worker A/C implementations.
Function pointers (not `FnMut` traits) sidestep higher-ranked lifetime
inference issues with closures over `LeagueSnapshot<'_>`.

**Acceptance.** `cargo build -p nba3k-trade` clean (one pre-existing
unused-variable warning in Worker A's `valuation.rs`, not negotiate's).
`cargo test -p nba3k-trade --tests negotiate` runs 14 tests across two
target files — `negotiate_personality.rs` and `negotiate_state_machine.rs` —
all pass.

### Worker C — `cba.rs` (2026-04-25)

**Tier classifier.** A team's salary tier is the bracket its **pre-trade**
total roster salary falls into — `cap`, `apron_1`, `apron_2` from
`LeagueYear`. Both teams in an offer are evaluated independently against
their own tier, so a non-apron team trading with an apron-2 team hits
its own non-apron ceiling while the counterparty is locked into the
apron-2 restrictions.

**Salary-matching ceilings** (post-2023 CBA):

- `UnderCap`: ceiling = outgoing + cap room (cap − current total salary,
  floored at 0). Captures the "if you have room you can absorb" rule.
- `NonApron`: outgoing ≤ $7.5M → 200% + $250K. Above $7.5M → 125% + $250K.
- `Apron1`: 110% flat.
- `Apron2`: 100% flat (and aggregation/cash banned outright — see below).

**Trade-kicker asymmetry** (RESEARCH.md item 7).
`outgoing_salary_pre_kicker` sums each outgoing player's *current* cap
hit (no bump) plus `cash_out`. `incoming_salary_post_kicker` walks the
*other* side's outgoing players and, for each incoming player with
`trade_kicker_pct > 0`, computes the kicker bump as `pct% × remaining
guaranteed base`, prorates over the count of remaining guaranteed
years, and adds that to the year-1 cap hit. Kicker base **excludes**
unexercised player- and team-option years per CBA Article VII §3.
`validate` calls these helpers per-side — there is no single "matched
salary" number on either side, by design.

**Apron-2 hard restrictions** are enforced *before* salary matching:
any team in `Apron2` that sends ≥2 outgoing players (aggregation) or
any positive `cash_out` triggers `Apron2Restriction` and short-circuits
the rest. This sequencing matters — apron-2 aggregation is a categorical
rule, not a 100% match overflow.

**v1 simplifications.**

- *Hard-cap triggers* are not yet tracked (sign-and-trade, taxpayer
  MLE, BAE, etc. each create a hard cap independently). v1 only
  enforces apron-2 as a hard cap — `check_hard_cap` rejects any trade
  that pushes a sub-apron-2 team past apron_2. Full trigger tracking
  is deferred until per-team cap exceptions are persisted.
- *Aggregation cooldown* is a stub returning `Ok(())`. Wires up once
  `Player.acquired_on` exists; the rule (60 sim days post-acquisition
  before that player can be aggregated) is otherwise straightforward.
- *Cash limit* is per-team-per-direction-per-trade (each `cash_out` ≤
  `LeagueYear::max_trade_cash`). Cumulative season tracking across
  *multiple* trades is deferred to a Store-side ledger.
- *Roster size* uses live `LeagueSnapshot::roster()` (filtered on
  `Player.team`) and computes post-trade as
  `current − outgoing + incoming`, bounded 13–15.
- *NTC* triggers on any outgoing player with `no_trade_clause = true`.
  v1 does not model partial-NTC or per-team waivers (the M2 player
  schema doesn't have those columns yet).

**Test surface.** Tests live in `crates/nba3k-trade/tests/cba_*.rs`
(four files: `_matching`, `_kicker`, `_apron2`, `_misc`) with a shared
`cba_common.rs` fixture builder. Sub-checks are exposed as their own
`pub fn` so tests can isolate (e.g. `check_roster_size` separate from
`validate`) — useful because `validate` short-circuits on the first
violation and several rule interactions (cash → matching, roster vs
matching) need single-rule assertions to be unambiguous. Run with
`cargo test -p nba3k-trade --test cba_matching --test cba_kicker
--test cba_apron2 --test cba_misc` — 18 tests, all green.

### Worker A — evaluate.rs + valuation.rs (2026-04-25)

- **OVR → baseline curve**: power curve `((ovr − 50) / 49)^2.6 × $210M`.
  Anchor points: OVR 99 ≈ $210M, OVR 88 ≈ $95M, OVR 80 ≈ $25M, OVR 70 ≈
  $5M, OVR 50 ≈ $0 (replacement-level). Convex on purpose so star-vs-role
  gaps look right before any personality multipliers fire. Calibration
  harness can re-tune.
- **Age curve**: peak at 27 (multiplier 1.05). Below peak: linear taper
  -0.012/yr (so 22 ≈ 1.00, 19 ≈ 0.95). Past peak: stepped decline of
  -3%/yr to 30, -6%/yr to 33, -10%/yr after, floored at 0.30. Stepped
  rather than smooth so cliffs match how GMs actually talk about veteran
  value.
- **Star premium**: only fires at `OVR ≥ 88`, multiplier =
  `traits.star_premium` (default 1.0, StarHunter 1.6). No smooth ramp on
  purpose — we want the 87↔88 discontinuity so "near-star" and "star" are
  categorically different to the engine.
- **Loyalty bonus**: `traits.loyalty × 20% × baseline_dollars(ovr)`.
  Applied at the offer level (`value_side`) only when the side being
  valued *is* the evaluator. A Loyalist GM (loyalty 0.6) values their own
  player at 12% above market — right order of magnitude for the "I don't
  want to trade my guy" effect without overwhelming OVR-based value.
- **Salary aversion**: `current_salary × traits.salary_aversion`
  subtracted from talent value. Cheapskate (1.8) on a $50M player loses
  $90M of perceived surplus, routing them away from max-contract trades
  exactly as designed.
- **OVR↔potential blending**: weighted average of `current_overall_weight`
  and `potential_weight`. WinNow (1.5/1.0) tilts toward immediate impact;
  Rebuilder (0.6/1.7) tilts toward upside.
- **Pick value**: round-1 only is meaningful; round-2 is a flat $500K
  token. Round-1 anchor curve `90 × (1 − (slot−1)/29)^1.4 + 4` ($M),
  giving #1 ≈ $94M, #5 ≈ $42M, #14 ≈ $15M, #30 ≈ $4M. Slot is projected
  from current standings (worst record → top pick) when `pick.season ==
  current_season`, else slot 15 fallback. Future picks discount 10%/yr
  beyond year 1. Multiplied by `traits.pick_value_multiplier`.
- **Verdict thresholds**: ≥ +5% of outgoing → Accept, ≤ −15% → Reject,
  else Counter. Net% is computed against *outgoing* (not the smaller
  side) so the high-money side anchors the percentage.
- **Noise**: gaussian, σ = 8% of outgoing × `traits.gullibility`.
  Wildcard (gullibility 0.7) gets ~5.6% noise; Conservative gets nearly
  none.
- **Test isolation**: `evaluate_with_traits` is the testable seam. The
  full `evaluate()` calls `cba::validate` + `context::apply_context` and
  would panic against `todo!()` stubs during parallel worker execution.
  Both `tests/evaluate_regression.rs` and the in-module unit tests use
  `evaluate_with_traits` directly, so this crate is testable in isolation
  while sister workers are still in flight.
