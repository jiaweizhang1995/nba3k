# BUGFIX: Phase-aware roster size (offseason 21, season-start ≤15)

**Type:** bug + small feature
**Owner (impl):** codex
**Status:** scoped, not started

## Symptoms (user report)

1. During **offseason**, attempting a trade is rejected with the post-trade roster falling outside the `13..=18` window. Per NBA CBA the offseason cap is **21**, not 18, so legit offseason trades are being blocked.
2. There is **no gate** preventing a season from starting while a team is carrying more than 15 standard players. Per NBA CBA, by the regular-season opener every team must be cut to **15 standard contracts (+3 two-way)**.

## NBA CBA reference (2025-26)

- **Training camp / offseason / preseason:** up to **21 players** per team. The 21 includes 15 standard + up to 3 two-way + Exhibit-10 / non-guaranteed camp bodies.
- **Regular-season opening night:** **15 standard contracts max + 3 two-way max** = 18 total slots. League floor is 14 standard (with limited 2-week exceptions at 13).
- Two-way players **do not count** against the regular-season 15, but **do count** against the offseason 21.

Sources: [SLAM — CBA Explained: NBA Roster Size Limits](https://www.slamonline.com/news/nba/cba-explained-nba-roster-size-limits/), [Hoops Rumors — 2025-26 NBA Roster Counts](https://www.hoopsrumors.com/2025/07/2025-26-nba-roster-counts.html).

The codebase **does not currently model two-way as a separate contract type** — every player on a team's roster counts equally. So we treat the cap as a single integer that varies by phase. Two-way modeling is out of scope for this fix.

## Root cause (in code)

The trade CBA validator hard-codes a single roster window:

```
crates/nba3k-trade/src/cba.rs:367-377
    let post = current_size - outgoing + incoming;
    // 2025-26 CBA: 15 standard contracts + 3 two-way = 18 total roster spots.
    // Floor of 13 enforces the league minimum carry rule.
    if !(13..=18).contains(&post) {
        return Err(CbaViolation::RosterSize { team, size: post.max(0) as u32 });
    }
```

It runs the same window in OffSeason / FreeAgency / Draft / PreSeason as in Regular. `LeagueSnapshot` already exposes `current_phase: SeasonPhase` (`crates/nba3k-core/src/snapshot.rs:57`), so the validator has the data — it just isn't using it.

There is no season-start roster check anywhere. The transition that ends offseason and begins gameplay is `next_phase` for `PreSeason → Regular` at `crates/nba3k-season/src/phases.rs:19-25` (triggered when `state.day > PRESEASON_LAST_DAY = 6`).

## Required changes

### 1. Phase-aware roster bounds in CBA validator

**File:** `crates/nba3k-trade/src/cba.rs`

Add a helper that returns `(min, max)` based on `SeasonPhase`:

| Phase | min | max | Reason |
|---|---|---|---|
| `OffSeason` | 13 | **21** | Training-camp window |
| `FreeAgency` | 13 | **21** | Treated as offseason |
| `Draft` | 13 | **21** | Treated as offseason |
| `PreSeason` | 13 | **21** | Training-camp window — cuts happen on day before opener |
| `Regular` | 13 | 18 | Current behavior |
| `TradeDeadlinePassed` | 13 | 18 | Current behavior |
| `Playoffs` | 13 | 18 | No trades anyway, but be safe |

Suggested API:

```rust
fn roster_bounds_for_phase(phase: SeasonPhase) -> (i64, i64) {
    use SeasonPhase::*;
    match phase {
        OffSeason | FreeAgency | Draft | PreSeason => (13, 21),
        Regular | TradeDeadlinePassed | Playoffs => (13, 18),
    }
}
```

Update `check_roster_size` (lines 350-377) to read `league.current_phase` and call the helper instead of the hard-coded `13..=18`.

Keep the `CbaViolation::RosterSize` variant unchanged. The error already carries `size`; if we want a clearer message later we can add a `cap` field, but **don't expand the variant in this fix** — minimize blast radius.

### 2. New CBA check: season-start roster ≤ 15

**Don't put this in `validate(...)` for trades** — it's not a trade rule. Add a new public function in the same crate so the season transition site can call it:

**Suggested API (in `crates/nba3k-trade/src/cba.rs` or a new sibling module):**

```rust
/// Per-team violation for the regular-season opener gate.
pub struct RosterTooLargeAtSeasonStart {
    pub team: TeamId,
    pub size: u32,
    pub limit: u32,  // 15
}

/// Returns the list of teams currently carrying > `REGULAR_SEASON_ROSTER_MAX`
/// (= 15) at the moment the regular season is about to begin. Empty Vec
/// means "OK to start". Caller decides how to surface the error.
pub fn check_season_start_rosters(league: &LeagueSnapshot)
    -> Vec<RosterTooLargeAtSeasonStart>;
```

Use **15** here — i.e. the standard-contract cap — *not* 18, because the codebase has no two-way distinction; every roster slot is a "standard" slot in our model. (When two-way support is added later, the cap becomes "≤ 15 standard + ≤ 3 two-way" and this check splits.)

Add a constant `pub const REGULAR_SEASON_ROSTER_MAX: u32 = 15;` so callers and tests reference one place.

### 3. Wire the season-start check at the PreSeason → Regular transition

**File:** `crates/nba3k-season/src/phases.rs` and the CLI advance path.

Two reasonable places:

- **Best:** add a guard in `next_phase` (lines 17-35) that, when stepping from `PreSeason` → `Regular`, returns `PreSeason` if any team is over the limit, **and** surface that to the caller via a separate validator the CLI/TUI calls before calling `next_phase`. Pure functions like `next_phase` shouldn't sniff the league snapshot, so the cleaner version is:
- **Recommended:** Keep `next_phase` pure. Add a new pure helper `pub fn validate_regular_season_start(league: &LeagueSnapshot) -> Vec<RosterTooLargeAtSeasonStart>` (re-export from `nba3k-trade::cba::check_season_start_rosters` or duplicate the logic in `nba3k-season` — pick one home and document it). The CLI's `cmd_sim_day` / wherever it would otherwise let `state.phase` flip to `Regular` should call this **before** the flip. If non-empty, refuse the day-advance and print the offending teams.

**CLI hook locations to inspect before deciding:**
- `crates/nba3k-cli/src/commands.rs:868-869` — `cmd_sim_day` PreSeason branch ("just advance the counter, no games").
- `crates/nba3k-cli/src/commands.rs:4326+` — `cmd_season_advance` (Playoffs/OffSeason → next year's PreSeason). This is **not** the right hook for the new check (we're going *into* PreSeason here, not out of it), but it's the natural place to add a soft warning so users know they need to cut down.
- The TUI screen that drives day advancement — there are also TUI buttons that step time. Make sure they share the same gate by routing through the shared command, not duplicating the transition.

### 4. Update / add tests

**File:** `crates/nba3k-trade/tests/cba_misc.rs` (existing tests at lines ~35, ~52, ~70, ~125 reference `13..=18`).

- Keep the existing "13 too small / 18+1 too large" tests but pin them to `SeasonPhase::Regular`. They should still pass.
- Add: `roster_offseason_allows_up_to_21_passes` — 20 → +1 = 21, phase OffSeason, expect Ok.
- Add: `roster_offseason_22_rejects` — 21 → +1 = 22, phase OffSeason, expect `RosterSize { size: 22, .. }`.
- Add: `roster_preseason_uses_offseason_bounds` — same but phase PreSeason.
- Add: `roster_regular_ceiling_still_18` — 18 → +1 in Regular, expect rejection (regression guard).
- Add: a season-start test in `crates/nba3k-trade/tests/cba_misc.rs` (or a new `tests/season_start.rs`):
  - `season_start_with_15_passes` — every team has ≤ 15 → empty Vec.
  - `season_start_with_16_flags_team` — one team has 16 → returned in Vec with `size = 16, limit = 15`.

Look at the existing test fixtures (they build a fake `LeagueSnapshot`) and add a `phase: SeasonPhase` parameter where they currently default to whatever they default to. Don't introduce a new fixture builder if the existing one already takes a phase.

### 5. Update the TUI banner / FA cap notes

**File:** `crates/nba3k-cli/src/tui/screens/trades.rs:159-160`

The const `FA_ROSTER_CAP: usize = 18` is used for the **free-agent signing UI**, not trades. FA signing should also rise to **21 during offseason**. Update the const to a function that takes the current phase, or replicate the phase-aware bounds logic. Look at every usage of `FA_ROSTER_CAP` and `AI_FA_ROSTER_CAP` (`commands.rs:4536, 4770`, etc. — `grep -n "FA_ROSTER_CAP\|AI_FA_ROSTER_CAP" crates/nba3k-cli/src/`) and decide:

- `FA_ROSTER_CAP` (user-facing FA sign cap, `commands.rs:4770`): **phase-aware** — 21 in offseason, 18 in regular.
- `AI_FA_ROSTER_CAP` (AI target during the season-end FA pass, `commands.rs:4536`, value `16`): leave alone. The AI pass runs once during offseason and "stop at 16" is a charter rule, not a CBA rule.

### 6. Comments / docs

- Update the comment at `crates/nba3k-trade/src/cba.rs:368-369` to explain the phase split.
- Update the comment at `crates/nba3k-cli/src/tui/screens/trades.rs:159` similarly.
- Add a one-paragraph note to `docs/ARCHITECTURE.md` (in the trade-engine section) describing that roster bounds are phase-aware.

## Bash verification artifact (mandatory per project conventions)

```bash
cargo test -p nba3k-trade -- cba_misc
cargo test -p nba3k-trade -- season_start   # if added as a separate file
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Manual smoke (offseason path):

```bash
cargo run -- --save /tmp/bug.db new --team BOS
# fast-forward to offseason any way the project provides; then:
cargo run -- --save /tmp/bug.db trade propose ...   # something that lands roster at 19
# expect: accepted (was: rejected with RosterSize)
```

Manual smoke (season-start gate):

```bash
# With a save where a team currently has 16+ players in PreSeason:
cargo run -- --save /tmp/bug.db sim-day --days 7   # crosses PreSeason → Regular
# expect: refusal with the offending team listed; sim does NOT advance into Regular.
```

## Out of scope (do NOT touch in this fix)

- Two-way contract type modeling. (Future work — would also let us split the offseason 21 into 18+3 properly.)
- Exhibit-10 / camp-invite handling.
- AI auto-cut logic (AI teams shedding players to comply with the 15-cap before the opener). Manual gate first; AI auto-cut is a follow-up.
- Salary-matching changes — unrelated.

## Risk / blast radius

- **Trade tests** are the main thing to keep green; they currently pin `13..=18` so they need a phase tag added.
- The season-start gate will refuse to advance day for users with bloated rosters — make the error message **list every offender** with `team_abbrev` and `current_size`, not just bail on the first one. The user needs to know what to cut.
- Don't break `--script tests/scripts/season1.txt` — run it end-to-end.
