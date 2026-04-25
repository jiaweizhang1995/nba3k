# NBA 2K MyGM/MyNBA Mechanics — Borrow / Skip / Adapt for nba3k-claude

_Research date: 2026-04-25 — current versions: NBA 2K26 (shipped Sep 2025), NBA 2K27 not yet announced. Targeted at our Rust CLI clone living in `crates/nba3k-{core,models,trade,sim,season}`._

## Executive Summary (top 5 mechanics worth borrowing, priority order)

1. **21-attribute schema with 6 categories + tendencies layer** — replace the 10-field flat `Ratings` with a 2K-shaped struct so every downstream model (sim, valuation, fit) gets a richer signal at near-zero runtime cost.
2. **Player roles + morale + chemistry meter** (Star / Starter / Sixth Man / Role Player / Bench Warmer) — single discrete `Role` enum unlocks promised-PT contract clauses, retention, and fit modulation in one shot.
3. **Three-tier sim engine (Normal / Smarter / Faster) with explicit asset+protection awareness** — frames our roadmap: keep current `stat_projection` as "Normal", add a richer offline trade-AI pass for "Smarter" sims.
4. **Potential as a yearly-adjusted "track-to-peak" estimator + peak window** — gives M5 progression a concrete formula instead of a static `potential` cap.
5. **Scouting-fog draft model** (combine + workouts + scout points reveal hidden ratings) — drives M6 design and slots cleanly next to our existing `DraftPick`/`draft.rs` module.

---

## 1. Player Rating Model

What 2K does:
- **21 attributes across 6 categories**: Inside Scoring (Close Shot, Driving Layup, Driving Dunk, Standing Dunk, Post Control), Ranged Shooting (Mid-Range, 3-Point, Free Throw), Handling (Passing Accuracy, Ball Handle, Speed With Ball), Defense (Interior Defense, Perimeter Defense, Steal, Block), Rebounding (Off Reb, Def Reb), Athleticism (Speed, Agility, Strength, Vertical) ([Game Rant — All Attributes Explained](https://gamerant.com/nba-2k26-all-attributes-explained/), [2KRatings — Attribute Definitions](https://www.2kratings.com/nba-2k-attributes-definitions)). Each is 0–99.
- **Tendencies are a separate layer** that drives in-engine AI behavior (how often a player attempts a given action), and in 2K the shooting tendency follows roughly `Tendency = 2*(Usage% − 20) + 50` ([NLSC simulated stats guide](https://forums.nba-live.com/viewtopic.php?f=141&t=84310)).
- **Badges** sit on top of attributes. NBA 2K26 ships **43 badges** across six families (shooting, playmaking, finishing, defense, rebounding, general) at five tiers (Bronze / Silver / Gold / HoF / Legend), and badges have minimum-attribute unlock thresholds — i.e. they're discrete unlocks on top of the continuous attribute number ([AOEAH 2K26 badge guide](https://www.aoeah.com/news/4119--nba-2k26-best-badge-tier-list-unlock-requirements--how-to-get-all-badges-fast), [2KW 2K26 badge requirements](https://nba2kw.com/nba-2k26-badge-requirements)).
- Overall is a weighted combination of attributes; 2KLab publishes a "calculated weights heat map" that confirms weights are non-uniform and position-dependent ([2KLab attribute weights](https://www.nba2klab.com/nba2k-attribute-calculated-weights-heat)).

**For our build: Borrow (Adapt the schema, defer badges).** Replace the 10-field `Ratings` struct in `crates/nba3k-core/src/player.rs` with a 21-field struct grouped by 6 categories. This is a one-time core-types change but it's the foundation for every downstream model — `stat_projection.rs`, `player_value.rs`, `asset_fit.rs`, and the 10 archetype templates in `nba3k-sim` all currently fight against the loss of fidelity (e.g. there's no way to differentiate a great catch-and-shoot wing from a great isolation scorer because both share `shooting_3` + `playmaking`). Keep `overall: u8` as a precomputed cached field on `Player`, but change `Ratings::overall_estimate` to a position-aware weighted sum modeled on the 2KLab heat map. Add a sibling `Tendencies` struct (same shape, semantically different) in `nba3k-models` only — sim consumes it, core does not. **Skip badges in M4–M5**: a 43-entry boolean+tier matrix is a `Vec<Badge>` field that adds query cost without immediate sim payoff; revisit once the rating refactor lands and we can compute "implicit badge tiers" from attribute thresholds for free.

---

## 2. CPU Trade AI

What 2K does:
- 2K26 introduced three sim modes: **Normal**, **Smarter** (slower; "considers over 5,000 potential trade variables, such as trade protections and pick swaps"), and **Faster** (skips pick-protection eval) ([2K Newsroom](https://newsroom.2k.com/news/endless-possibilities-await-as-mynba-levels-up-in-nbar-2k26), [2K Courtside Report MyNBA](https://nba.2k.com/2k26/courtside-report/mynba/)). Performance: "up to 26% faster in NBA 2K26 compared to NBA 2K25" while doing strictly more work.
- All three sims still respect "the core rules behind the salary cap, trades, and free agency" — i.e. CBA validation is non-negotiable across modes ([Operation Sports MyNBA Details](https://www.operationsports.com/nba-2k26-mynba-and-mygm-details-revealed/)).
- 2K exposes **dedicated trade/contract sliders**: trade negotiation difficulty, contract negotiation difficulty, CPU re-signing aggressiveness ([Stumpy/Popboy slider guides](https://forums.operationsports.com/forums/forum/basketball/nba-2k-basketball/nba-2k-basketball-sliders/26863707-stumpy-s-nba-2k26-cpuvcpu-simulation-sliders)).
- Fairness UX: 2K shows a fairness/asset-value indicator in the trade screen before submission (same sliders + Stumpy thread reference it). Trades from players are **gated by morale + games played** in MyCareer (15-game min, post-season only — [Operation Sports — Trade Request 2K26](https://www.operationsports.com/how-to-request-a-trade-in-nba-2k26/)); MyNBA force-out is morale-driven via the role/PT system.

**For our build: Adapt.** Our `nba3k-trade` already has CBA validation, GM personalities, team-context modulation, star protection, and a counter-offer state machine — the architecture is already 2K-shaped. Three concrete deltas: (a) Add a `SimMode { Normal, Smarter, Faster }` enum on the sim/season runner (lives in `nba3k-season`) and wire it so `Smarter` runs the full `evaluate.rs` pipeline including pick-protection lookups, while `Faster` short-circuits to a salary-only sanity check; this matches 2K's published trade-off and gives the user a knob. (b) Surface a `TradeFairnessReport` (asymmetric — asset value delta + each side's GM verdict + CBA legality) so the CLI can show a 2K-style "fair / lopsided" indicator without re-running the engine. (c) Add a player-driven trade request signal in `models/trade_acceptance.rs` keyed off morale + role mismatch (see §5). **Skip**: the 5,000-variable count is marketing — we don't need to enumerate every protection variant to be 2K-grade; what matters is that pick protections are evaluated, which they already are in `cba.rs`.

---

## 3. Player Progression / Regression

What 2K does:
- "Potential is supposed to represent an approximation of the ceiling of a player… he can reach it by peak start, maybe a few years after that, maybe even surpass it by a couple of points. Injuries, lack of playing time, bad training staff, can all hinder that progress." Potential is **adjusted every year**: before peak age, the engine checks whether the player is still on track to hit potential by peak; if not, potential is revised down ([NLSC peak-age thread](https://forums.nba-live.com/viewtopic.php?t=100026)).
- **Training schedule** is a separate weekly system in MyNBA (Prep Hub from 2K22 onward) with intensity tiers that trade fatigue/injury-risk against attribute gain ([2K22 MyNBA training schedule](https://newsroom.2k.com/resources/nba-2k22-mynba-training-schedule)).
- 2K26 ships **Training Facilities** that "improve player strength, hustle, potential, and influence player negotiations" ([2K Courtside Report MyNBA](https://nba.2k.com/2k26/courtside-report/mynba/)).
- 2K26 sliders: explicit `progression_rate` and `regression_rate` with `progression_intensity` and `work_ethic` modifiers ([Popboy 2K26 sliders](https://www.studocu.com/en-ca/document/high-school-canada/sport-theory/popboy-nba-2k26-mynba-mygm-sliders-guide-and-setup-tips/158329516)).

**For our build: Borrow.** This is M5's centerpiece. Add a `PlayerDevelopment` sibling type in `nba3k-models` (no `Player` core change required) holding `peak_start_age: u8`, `peak_end_age: u8`, `dynamic_potential: u8` (mutable, distinct from the static `Player.potential` ceiling), and `work_ethic: u8`. Once a season, run a progression pass that: (1) computes expected attribute gain based on age vs peak window, work_ethic, minutes played, training-facility tier; (2) re-computes `dynamic_potential` as "what we now project this player will hit by `peak_end_age`"; (3) starts regression after peak_end with position-weighted decline (athleticism declines first, IQ last — well-documented NBA aging-curve consensus). Don't mutate `Player.potential` in place; keep it as the ceiling and let `dynamic_potential` track realized trajectory. This explicitly matches 2K's "track-to-peak" check.

---

## 4. Free Agency + Contract Acceptance

What 2K does:
- **Player roles** assignable in MyNBA: Star / Starter / Sixth Man / Role Player / Bench Warmer / Prospect — and "the roles will either boost or reduce your players' morale, and a player's morale will affect how they play on the court, as well as their decision to resign to your team" ([NBA 2K Wiki — Front Office](https://nba2k.fandom.com/wiki/Front_Office)).
- Negotiable contract levers: **money/year, flat or back-loaded, years, player/team options, NTC, role** (same Front Office wiki). Restricted FAs get a one-year qualifying offer.
- Signing must obey real-CBA rules: vet min, two-way deals, MLE, etc. ([2K MyLeague signing rules — Front Office wiki](https://nba2k.fandom.com/wiki/Front_Office)).
- 2K26 facilities feed back: training-facility tier "influences player negotiations" ([2K Courtside Report MyNBA](https://nba.2k.com/2k26/courtside-report/mynba/)).
- Known long-running 2K bug: free agents go unsigned because the engine misprices market value ([Operation Sports — unsigned FAs](https://www.operationsports.com/nba-2k25-mynba-how-to-avoid-too-many-free-agents-staying-unsigned/)) — useful negative example: don't gate FA acceptance solely on team's offer; have a market-clearing pass.

**For our build: Borrow.** M6 free agency should center on a `PlayerPriorities` struct (fields: `winning_weight`, `money_weight`, `playing_time_weight`, `role_weight`, `loyalty_weight`, `market_size_weight`, all f32 normalized) plus the `Role` enum from §5. The acceptance score is a weighted sum where each team's offer is scored against the priorities — extending the pattern we already use in `nba3k-models/trade_acceptance.rs`. Add a `ContractClauses` struct that supports promised-PT, no-trade, trade kicker, player option, team option, ETO — these already partially exist in `nba3k-core/contract.rs` and `Player.no_trade_clause/trade_kicker_pct`, just need to be fleshed out. Run a market-clearing pass at the end of FA day-N (multiple iterations, falling reservation prices) so we don't ship the "unsigned superstar" bug 2K still has.

---

## 5. Lineup Chemistry + Scheme Fit

What 2K does:
- 2K26 ships an explicit **team chemistry meter** — Mike Wang quote: "that's the team chemistry system. contribution maxed means you've filled your portion of the team's meter. once everybody has contributed, the meter activates and gives your whole team boosts" ([Mike Wang via The2KCentral](https://x.com/The2KCentral/status/1961816193041420422)).
- Chemistry is driven by **role acceptance** — assigning a Star to a Bench Warmer role tanks morale, and morale flows into chemistry ([NBA 2K Wiki — Front Office](https://nba2k.fandom.com/wiki/Front_Office)).
- Coaching style → roster fit: 2K shows a 5-star "fit" rating for each player against the active coach system (Balanced / Defense / Grit & Grind / Pace & Space / Perimeter Centric / Post Centric / Triangle / Seven Seconds — [Coaching wiki](https://nba2k.fandom.com/wiki/Coaching)).

**For our build: Adapt.** Don't model chemistry as a new top-level entity — model it as a **derived score** on top of the `Role` assignment + GM/coach scheme + positional balance. Add a `Role` enum to `nba3k-core/player.rs` (sibling to `Position`, doesn't fork `Player` semantics, just adds one field). Compute a `team_chemistry: f32` (0..=1) per `LeagueSnapshot.team(...)` view by combining: (a) role-vs-archetype mismatch penalty using our existing 10 archetypes in `stat_projection.rs`; (b) positional balance (penalize 4-PG lineups); (c) star-stack penalty (n stars sharing usage). Apply chemistry as a small game-day multiplier on team output in `nba3k-sim/engine/statistical.rs` — capped at ±5% so it never dominates raw talent. Skip the literal "meter that activates" UX; it's a HUD device, not a sim mechanic.

---

## 6. Coaching & Schemes

What 2K does:
- Coaching style is a **discrete enum** picked per coach (Balanced / Defense / Grit & Grind / Pace & Space / Perimeter Centric / Post Centric / Triangle / Seven Seconds), and players show a 1–5 star fit rating against the active style ([NBA 2K Wiki — Coaching](https://nba2k.fandom.com/wiki/Coaching)). Playbook is a separate layer with 100+ plays.
- 2K26 also ships a **MyTeam-side Coach Card system** with categories Strategy / Leadership / Mentorship / Knowledge / Team Management ([2K Dispatch tweet](https://x.com/2KDispatch/status/1956015115456729478)) — relevant because it confirms 2K's mental model of coaches as 5-axis cards, not a single OVR.

**For our build: Adapt (light touch in M4 polish, full in M7).** Add a `Coach` struct to `nba3k-core` (sibling to `GMPersonality`) with `scheme_offense: Scheme`, `scheme_defense: Scheme`, plus the five 2K coach axes as f32 weights. Wire `scheme_fit(player, coach) -> f32` into the chemistry calc in §5. Don't add a play catalog or playbook system in M4–M6; the marginal sim realism gain isn't worth the data-collection burden. The big architectural call: **`Coach` lives next to `GMPersonality` in core, not as a sub-field of `Team`** — same as our current GM modeling — so trades and FA can both read coach scheme without a `Team` mutation.

---

## 7. Injuries

What 2K does:
- 2K26 sliders explicitly expose **Injury Severity, Injury Duration, and Career-Ending Injury Frequency** ([Popboy 2K26 sliders](https://www.studocu.com/en-ca/document/high-school-canada/sport-theory/popboy-nba-2k26-mynba-mygm-sliders-guide-and-setup-tips/158329516)). Confirms the engine has a multi-tier severity model, not just a binary.
- Two relevant attributes: **Durability** ("ability to withstand physical demands… less likely to suffer fatigue or injuries") and **Stamina** ("ability to maintain performance over the duration of a game") — [2KRatings durability list](https://www.2kratings.com/lists/overall-durability-attribute).
- 2K26 facilities: **Recovery Facilities** "reduce injury chance and recovery time" ([2K Courtside Report MyNBA](https://nba.2k.com/2k26/courtside-report/mynba/)).
- Community: known durations get reported in weeks (e.g. broken leg 6–8 weeks, Achilles much longer). 2K's `InjurySeverity` granularity isn't published as an enum but slider-driven ranges are real ([Operation Sports — MyNBA Injury Duration thread](https://forums.operationsports.com/forums/forum/basketball/nba-2k-basketball/926689-mynba-injury-duration)).

**For our build: Adapt.** Our `InjurySeverity` enum (`DayToDay / ShortTerm / LongTerm / SeasonEnding`) already mirrors 2K's tiering. The gap is the **roll model**: today injuries are binary per game with no input from durability, fatigue, or minutes load. Add a `durability: u8` field to the new attribute schema (§1) and a per-game `fatigue: f32` accumulator in `nba3k-sim` (decays during off-days). Injury roll = base_rate × f(minutes_load) × f(durability) × f(fatigue) × f(age). Keep the `Option<InjuryStatus>` field on `Player`; just populate `games_remaining` from a severity→duration table and decrement it during phase advancement in `nba3k-season/phases.rs`. Don't model body-part-specific injuries (knee vs ankle) — adds data debt without sim payoff.

---

## 8. Awards + All-Star

What 2K does:
- 2K26 ships **trophy ceremonies for 7 awards**: MVP, Rookie of the Year, DPOY, Sixth Man of the Year, MIP, Clutch Player of the Year, Coach of the Year ([Operation Sports — 2K26 presentation overhaul](https://www.operationsports.com/nba-2k26-presentation-overhaul-includes-dynamic-banners-trophies-and-more/)).
- Real NBA voting math (which 2K mirrors) is well-known: MVP/All-NBA/All-Defense use **10-7-5-3-1** for 1st–5th; DPOY/ROY/Sixth Man/MIP/COY use **5-3-1** for 1st–3rd (Basketball-Reference awards page format, e.g. [BBR 2026 awards](https://www.basketball-reference.com/awards/awards_2026.html)).
- 2K does not publish its scoring formula but community consensus is BPM/PER-style aggregate + win-share + win-pct floor.

**For our build: Borrow.** Awards are M5-cheap and high-impact for narrative. Add an `AwardsEngine` in `nba3k-season` that runs at end-of-regular-season: each award computes a score per eligible player (MVP = box-score composite × team win% gate; DPOY = defensive composite + DRtg; ROY = rookies-only counting stats; MIP = season delta from prior year). Use the real 10-7-5-3-1 / 5-3-1 weighting to simulate a 100-voter ballot; sample voters with controlled noise. All-NBA 1st/2nd/3rd team and All-Defensive 1st/2nd team fall out of the same scoring run. All-Star selection: 2 starters per position per conference + 7 reserves, gated on team standing + score. Persist results in a new `AwardsHistory` snapshot — feeds GM personality (Star Premium GMs respect MVPs) and FA market value (priors based on accolades).

---

## 9. Draft & Scouting

What 2K does:
- Prospects appear with **Name, Position, projected round** visible by default; "if you decide to scout a certain player, then you can reveal the rest of their statistics" ([Operation Sports — MyLeague scouting guide](https://forums.operationsports.com/forums/nba-2k-basketball/896170-guide-scouting-myleague.html)).
- **NBA Draft Combine + Pre-Draft Workouts** are explicit data sources that reveal additional attributes ([NBA 2KW MyNBA features](https://nba2kw.com/nba-2k26-mynba-all-new-features-feat-myplayer-dna-playoffs-online-more)).
- 2K26 added a **Scouting Report** feature that surfaces strengths/weaknesses ([2K official Facebook post](https://www.facebook.com/NBA2K/posts/the-all-new-scouting-report-feature-in-nba2k26-provides-another-helpful-guide-in/1345428580286173/)).
- Scout points are spent per-prospect to incrementally reveal hidden ratings — long-standing 2K mechanic ([NLSC + Operation Sports forum consensus](https://forums.operationsports.com/forums/nba-2k-basketball/896170-guide-scouting-myleague.html)). Reveal tiers go from "round projection" → "OVR range ±5" → "exact OVR" → "individual attributes" → "potential".

**For our build: Borrow.** This is M6's foundation. Add a `Prospect` struct in `nba3k-core/draft.rs` with: `true_attributes: Ratings`, `revealed_mask: ScoutingMask` (bitfield: round/ovr_range/ovr/per_attribute/potential), and `combine_results: Option<Combine>`. The CLI exposes `scout <prospect> --depth=N` which spends scout points to flip mask bits. Mock-board generation: aggregate every team's revealed view + GM personality bias to produce the consensus board. **Critical**: `revealed_mask` must be **per-team** (each team has its own view) — store in `LeagueSnapshot.scouting: HashMap<(TeamId, ProspectId), ScoutingMask>`. Combine and workouts are deterministic seasonal events that flip bits for everyone (combine = athletic + measurables) or just the host team (workouts).

---

## 10. Game-Sim Per-Player Distribution

What 2K does:
- 2K's 2K26 sim respects salary cap, trade, and FA rules across all three sim modes ([Operation Sports — 2K26 details](https://www.operationsports.com/nba-2k26-mynba-and-mygm-details-revealed/)).
- Community-documented sim mechanics: shooting tendencies derived from usage; box-score generation respects pace, possessions, position-locked rebounds. The NLSC simulated-stats guide lays out the formula `ShotTendency = 2*(Usage% − 20) + 50` and similar drive/pass tendencies ([NLSC sim stats guide](https://forums.nba-live.com/viewtopic.php?f=141&t=84310)).
- Long-standing community complaint about 2K sim: **fouls/FTs underweighted, kick-outs over-weighted on drives** ([Stumpy 2K26 sliders thread](https://forums.operationsports.com/forums/forum/basketball/nba-2k-basketball/nba-2k-basketball-sliders/26863707-stumpy-s-nba-2k26-cpuvcpu-simulation-sliders/page4)) — useful negative target.
- 2K does not publicly enforce triple-double rate calibration; community sliders manually re-balance assists vs rebounds.

**For our build: Skip wholesale port; cross-check our existing model.** Our `nba3k-models/stat_projection.rs` already does archetype-driven per-player distribution with star uplift. Specific cross-checks vs 2K to add to M4 polish: (a) usage→shot-attempt ratio should follow the 2K-style `2*(Usage% − 20) + 50` slope so star-vs-role-player split is realistic; (b) keep an explicit team FT rate floor (do not trust derived FT% × foul rate alone, learn from 2K's bug); (c) verify rebound distribution is position-locked (centers don't get assists, PGs don't get blocks) — already true in our archetype templates; (d) add a low-prob "rare line" tail (5+ blocks, triple-double) so the box scores don't compress to means. No new structs; tune `nba3k-sim/params.rs`.

---

## Roadmap Recommendation

Treat M4 polish as a **two-week calibration sprint** focused on borrowed-from-2K rate calibration (sim rare-events, FT floor, usage-tendency curve in §10) — these don't need new types and unlock realism payoff immediately. Land the **21-attribute schema refactor** (§1) at the very start of M5: it's a one-time core-types change and every other M5/M6 feature (progression, fit, scouting) reads from it, so paying that bill first avoids two refactors. Then within M5, ship awards (§8, cheap, narrative-rich), progression/regression (§3, the headline feature), and `Role`+morale+chemistry (§§4–5 — pulled forward from M6 because they're the connective tissue between rosters and FA, and our trade engine can immediately consume role mismatch as a force-out signal). M6 is then "draft + free agency" with the heaviest 2K influence: scouting fog (§9), priorities-based contract acceptance (§4), market-clearing FA pass. Defer playbook depth (§6) and granular injury body parts (§7) to M7 polish — they're lipstick once the substantive mechanics ship. The single risk: don't add badges (§1 deferred portion) or coach Coach Cards (§6) before M7; they're collectible-feel features that bloat snapshots without changing the sim output curve.
