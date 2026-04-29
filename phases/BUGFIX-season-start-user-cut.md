# BUGFIX: Season-start gate — user must cut to 15, AI is left alone

**Type:** follow-up to `BUGFIX-offseason-roster-cap.md`
**Owner (impl):** codex
**Status:** scoped, not started

## Problem

The previous fix (`BUGFIX-offseason-roster-cap.md`) added `check_season_start_rosters` and gated `PreSeason → Regular` on every team being ≤ 15. That gate fires for **all 30 teams**, but AI teams currently end the offseason at 16 (AI free-agent pass targets 16, draft adds 1 rookie). Result: the regular season cannot start at all without AI cuts.

The user's product decision: **only the user team must comply with the 15-player cap to start the season. AI teams are not auto-cut and are not blocked.** The user cuts manually via the existing `fa cut <player>` command (or the TUI Cut action) until their roster is ≤ 15. AI rosters carrying 16+ continue into the regular season as-is.

This is intentional: it pushes the front-office decision onto the user (NBA 2K-style) without forcing us to write AI auto-cut logic right now.

## Required changes

### 1. Narrow the season-start gate to the user team only

**File:** `crates/nba3k-trade/src/cba.rs` (around the `check_season_start_rosters` function added by the previous bugfix).

Add a new function that takes the user team explicitly and only reports a violation if **that one team** exceeds the limit:

```rust
/// Season-start gate scoped to the user team. AI teams are intentionally
/// not checked — they are allowed to enter the regular season carrying
/// more than `REGULAR_SEASON_ROSTER_MAX` players in this build (no AI
/// auto-cut). Returns `Some` if the user must cut, `None` if cleared.
pub fn check_season_start_user_roster(
    league: &LeagueSnapshot,
    user_team: TeamId,
) -> Option<RosterTooLargeAtSeasonStart>;
```

Keep `check_season_start_rosters` (the all-teams version) in the file for now — it's used by tests and may be useful later. **Do not delete it.** Just stop calling it from the CLI gate.

### 2. Switch the CLI gate to the user-only check

**File:** `crates/nba3k-cli/src/commands.rs` around lines 868-880 (the block added by the previous bugfix inside `cmd_sim_day`'s PreSeason branch).

Replace:

```rust
let offenders = nba3k_trade::cba::check_season_start_rosters(&snapshot);
if !offenders.is_empty() {
    bail!(
        "regular season start blocked: roster cuts required — {}",
        format_season_start_roster_violations(&snapshot, &offenders)
    );
}
```

with the user-only variant:

```rust
if let Some(v) = nba3k_trade::cba::check_season_start_user_roster(&snapshot, state.user_team) {
    let abbrev = snapshot.team(v.team).map(|t| t.abbrev.as_str()).unwrap_or("???");
    bail!(
        "regular season start blocked: {} has {} players (limit {}). \
         Cut a player with `fa cut <name>` until you are at {}. \
         AI teams are not checked.",
        abbrev,
        v.size,
        v.limit,
        v.limit
    );
}
```

The error message must:
- name the user team's current roster size
- name the limit (15)
- tell the user how to cut (`fa cut <name>`)
- explicitly note that AI teams are not gated, so the user knows it's not a global lockout

`format_season_start_roster_violations` (the multi-team formatter) becomes unused for this gate. **Do not delete it** — the all-teams check is still callable and the formatter pairs with it. Mark it `#[allow(dead_code)]` or leave a note that it's retained for the all-teams API.

The gate must keep its `state.mode.enforces_cba()` guard — God / Sandbox modes should still bypass.

### 3. Lower `AI_FA_ROSTER_CAP` so AI never crosses 15 in the first place

**File:** `crates/nba3k-cli/src/commands.rs:4536`.

Currently:

```rust
const AI_FA_ROSTER_CAP: usize = 16;
```

Change to **15** so the offseason AI free-agent pass stops at 15 instead of 16. After draft adds 1 rookie this still puts AI at 16 unless we *also* gate the FA pass differently — see below.

Look at the AI FA pass in `commands.rs` around lines 4461-4530. The roster size used there is the *current* size at the moment the FA pass runs. Trace whether the draft (which also runs in `cmd_season_advance` around lines 4379-4395) executes **before or after** `run_ai_free_agency`. Reading the file:

```
crates/nba3k-cli/src/commands.rs:4379-4395  // draft auto-sim (runs first)
crates/nba3k-cli/src/commands.rs:4400        // run_ai_free_agency (runs after)
```

So FA runs after draft. That means roster size is "post-draft" when FA pass evaluates the cap. If draft puts a team at 15 and FA cap is 15, FA pass adds zero — team stays at 15. Confirm by reading `run_ai_free_agency` and trace the `>= AI_FA_ROSTER_CAP` short-circuit at line 4488. If the comparison is `>=`, lowering the constant to 15 means FA pass refuses to push from 15 to 16 — perfect.

If teams *already* enter the FA pass at >15 (because the draft loop ran), they'd just skip signing. That's fine — they keep whatever they had after the draft. Empirically the season1 dump shows AI ending at 16 = 15 (post-draft) + 1 (FA). Cutting the +1 is exactly what lowering the constant does.

**Edge case:** if a team enters the offseason already over 15 (e.g. carrying training-camp bodies they kept from last year), the lower constant doesn't help — they stay over. **Out of scope for this fix.** If/when this comes up, we'll need an AI auto-cut, but per the user's decision we're not writing that now.

### 4. Tests

**File:** `crates/nba3k-trade/tests/cba_misc.rs` (or the season-start test file).

- Add: `season_start_user_only_15_passes` — user team has 15, several AI teams have 16. Expect `check_season_start_user_roster` returns `None` AND `check_season_start_rosters` returns the AI offenders (verify the all-teams API still flags them, since we're keeping it).
- Add: `season_start_user_only_16_blocks` — user team has 16, all AI at 15. Expect `Some` with `team = user_team, size = 16, limit = 15`.
- Update or remove: any existing test that asserts the *gate* (CLI side) blocks for non-user teams. The all-teams check tests (`season_start_with_15_passes`, `season_start_with_16_flags_team`) stay unchanged — they exercise the still-public function.

**No CLI integration test is required** beyond running `tests/scripts/season1.txt` end-to-end (see verification below). The integ test for that script is `crates/nba3k-cli/tests/integration_season1.rs` (currently `#[ignore]`); after this fix it should pass when run with `cargo test --release -- --ignored`.

### 5. Update docs

**File:** `docs/ARCHITECTURE.md`

The paragraph added in the previous bugfix says:

> blocks `PreSeason` → `Regular` in CBA-enforcing modes if any team still has more than 15 modeled standard-contract players.

Replace with:

> blocks `PreSeason` → `Regular` in CBA-enforcing modes if the **user team** has more than 15 modeled standard-contract players. AI teams are not gated and may enter the regular season above the cap; they are also not auto-cut.

## Bash verification artifact (mandatory)

```bash
# Unit + workspace tests
cargo test -p nba3k-trade --test cba_misc
cargo test --workspace
cargo fmt --check

# Smoke: season1 script must complete (was blocked after the previous bugfix).
cargo build --release -p nba3k-cli
rm -f /tmp/season1.db /tmp/season1.db-shm /tmp/season1.db-wal
./target/release/nba3k --save /tmp/season1.db new --team BOS --offline
./target/release/nba3k --save /tmp/season1.db --script tests/scripts/season1.txt
# Expect: completes without "regular season start blocked" error.

# Manual: confirm user is blocked when they over-roster.
rm -f /tmp/usercut.db /tmp/usercut.db-shm /tmp/usercut.db-wal
./target/release/nba3k --save /tmp/usercut.db new --team BOS --offline
# Sign one extra FA so user team is at 16 (or otherwise pad to 16):
./target/release/nba3k --save /tmp/usercut.db fa list
./target/release/nba3k --save /tmp/usercut.db fa sign "<name>"
./target/release/nba3k --save /tmp/usercut.db sim-day 7
# Expect: "regular season start blocked: BOS has 16 players (limit 15). Cut a player with `fa cut <name>` ..."
./target/release/nba3k --save /tmp/usercut.db fa cut "<name>"
./target/release/nba3k --save /tmp/usercut.db sim-day 7
# Expect: succeeds (now at 15).
```

## Out of scope

- AI auto-cut. Explicitly forbidden by user decision. AI rosters at 16+ are accepted into Regular phase.
- Two-way contract modeling.
- Changing the FA-sign cap behavior during offseason (still 21 per the previous bugfix).
- Trade roster bounds (untouched — already phase-aware from the previous bugfix).
- Touching `cmd_season_advance` itself. The gate must stay at the `PreSeason → Regular` flip in `cmd_sim_day`, not at year-end.

## Risk / blast radius

- Very small. The change is a one-line swap of which CBA helper the CLI gate calls + dropping `AI_FA_ROSTER_CAP` by 1.
- Make sure the all-teams `check_season_start_rosters` function isn't deleted — its tests still pass and another future feature may reuse it.
- If the season1 integration test (`#[ignore]`) is now green, leave it ignored unless the user explicitly opts in to making it part of CI. Plumbing the seed DB into CI is a separate effort.
