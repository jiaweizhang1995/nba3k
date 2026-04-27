# QA Report — flow-tester

## Summary
- **Golden path (one season)**: Walks reasonably end-to-end IF you stay in the prescribed order, but trade execution is broken (silent no-op) and several phase guards are missing.
- **Multi-season loop**: Broken. After `season-advance`, the new season's schedule is empty (0 unplayed of 1230) and `sim-to playoffs` runs 365 days then bails. Step 17 of the charter fails.
- **Phase guards**: Inconsistent. `season-advance` correctly rejects mid-Regular and mid-PreSeason. But `playoffs sim` and `draft sim` run regardless of the current phase, mutating state that the rest of the engine doesn't expect.
- **Empty-save error UX**: Cryptic and inconsistent — three different wordings across commands; one command silently shows blanks.
- **Trade UX**: Counter-offer JSON is emitted raw to the terminal; `respond accept` returns success without moving the players.
- **CLI rendering**: Roster `ID` column doesn't pad to fit 10-digit IDs; `--help` for `trade propose` and `draft pick` is missing every argument description; REPL has no prompt.

12 findings below.

---

## Findings

### F-01 — `trade respond accept` silently fails to move players — severity: high
**What I did**:
```
$ nba3k --save /tmp/qa-flow.db trade propose --from BOS --to LAL --send "Sam Hauser" --receive "Marcus Smart"
trade #1 — verdict: counter | status: open | round: 2 | teams: BOS/LAL

$ nba3k --save /tmp/qa-flow.db trade chain 1
trade #1 — 2 offer(s):
  round 1: {"by_team":[{"players_out":["Sam Hauser"],"team":"BOS"},{"players_out":["Marcus Smart"],"team":"LAL"}],"round":1}
  round 2: {"by_team":[{"players_out":["Sam Hauser","Luka Garza"],"team":"BOS"},{"players_out":["Marcus Smart"],"team":"LAL"}],"round":2}

$ nba3k --save /tmp/qa-flow.db trade respond 1 accept
trade #1 — verdict: accept | status: accepted | round: 2 | teams: BOS/LAL

$ nba3k --save /tmp/qa-flow.db player "Marcus Smart"
Marcus Smart (SG) — LAL
age 25 | OVR 73 | POT 79

$ nba3k --save /tmp/qa-flow.db player "Sam Hauser"
Sam Hauser (PF) — BOS
age 25 | OVR 73 | POT 79
```
**Expected**: After `accept`, Sam Hauser + Luka Garza on LAL; Marcus Smart on BOS.
**Actual**: Trade marked `accepted` in `trade list`, but no players moved on either side. Both teams' rosters unchanged.
**Fix idea**: `respond` path is committing the negotiation status without invoking the actual roster transfer. Verify the post-accept hook is firing the player-move write, and add an integration test that asserts `roster` reflects the trade.

---

### F-02 — `playoffs sim` runs from any phase including PreSeason day 0 — severity: high
**What I did**:
```
$ nba3k --save /tmp/qa-flow.db new --team BOS
created save /tmp/qa-flow.db (team=BOS mode=standard season=2026 seed_used=true)

$ nba3k --save /tmp/qa-flow.db playoffs sim
Playoffs (season 2026):
  R1: ATL 4 - 2 IND
  ...
  Finals: DAL 0 - 4 BOS
Champion: BOS

$ nba3k --save /tmp/qa-flow.db status
season:   2026 (PreSeason)
day:      0
schedule: 1230 games (1230 unplayed)
```
**Expected**: Error like `Error: playoffs sim requires phase Playoffs; current phase is PreSeason`.
**Actual**: Crowns a champion of the league before any regular-season game has been played. Phase doesn't even change. Also runs identically a second time, no idempotency check.
**Fix idea**: Mirror the `season-advance` phase guard onto `playoffs sim`. Reject if not in `Playoffs` phase, and reject (or no-op with a message) if a champion has already been recorded for the season.

---

### F-03 — `draft sim` runs mid-Regular-season and corrupts roster — severity: high
**What I did**:
```
$ nba3k --save /tmp/qa-flow3.db sim-day 30
simulated 30 day(s); 124 game(s) played; phase=Regular day=30

$ nba3k --save /tmp/qa-flow3.db draft sim
Draft sim — 30 picks made:
   1. LAL -> AJ Dybantsa (ovr=84, pot=96)
   ...
   7. BOS -> Boogie Fland (ovr=76, pot=87)
   ...

$ nba3k --save /tmp/qa-flow3.db roster BOS
BOS roster (17 players):
ID     NAME                          POS  AGE  OVR  POT
50000007  Boogie Fland                  PG    20   76   87
...
```
**Expected**: Error like `draft sim requires phase OffSeason`.
**Actual**: Adds drafted rookies to teams in the middle of an ongoing regular season. Save is now in an undefined state.
**Fix idea**: Phase guard on `draft sim` and `draft pick` — only permit in `OffSeason`.

---

### F-04 — `season-advance` does not regenerate schedule; new season unplayable — severity: high
**What I did**:
```
$ nba3k --save /tmp/qa-flow.db season-advance
advanced to season 2027 — progressed 530 players (Δsum=1160), 30 drafted

$ nba3k --save /tmp/qa-flow.db status
season:   2027 (PreSeason)
day:      0
schedule: 1230 games (0 unplayed)

$ nba3k --save /tmp/qa-flow.db sim-to playoffs
Error: sim-to bailing: exceeded 365 sim days without reaching Playoffs

$ nba3k --save /tmp/qa-flow.db status
season:   2027 (TradeDeadlinePassed)
day:      371
schedule: 1230 games (0 unplayed)

$ nba3k --save /tmp/qa-flow.db season-advance
Error: season advance requires phase Playoffs or OffSeason; current phase is TradeDeadlinePassed
```
**Expected**: After `season-advance`, the new season has 1230 unplayed games; `sim-to playoffs` plays through them.
**Actual**: New season shows `1230 games (0 unplayed)` — the prior season's played-game flags carry over instead of a fresh schedule being generated. `sim-to playoffs` ticks days but plays nothing, eventually bails after 365 days. Phase has progressed past TradeDeadline so the game is now stuck — `season-advance` rejects, leaving no path forward without `--god` or replaying playoffs sim.
**Fix idea**: `season-advance` must wipe (or rebuild) the game/schedule table for the new season, OR generate a fresh slate of 1230 games scheduled for the new year. Also: `sim-to` should detect "no unplayed games in current phase" and surface that, not silently burn 365 sim days.

---

### F-05 — `playoffs bracket` does not show results after `playoffs sim` — severity: medium
**What I did**:
```
$ nba3k --save /tmp/qa-flow.db playoffs sim
Playoffs (season 2026):
  R1: MIL 0 - 4 DET
  ...
  Finals: PHO 1 - 4 BOS
Champion: BOS

$ nba3k --save /tmp/qa-flow.db playoffs bracket
R1 bracket (season 2026):
  East MIL (1) v DET (8)
  East BOS (4) v NYK (5)
  ...
  West MEM (2) v GSW (7)
```
**Expected**: After playoffs run, `bracket` shows all rounds with completed series and scores, like a March Madness updated bracket.
**Actual**: Only R1 matchups are shown, no series scores, no later rounds. Replaying the bracket post-sim is therefore useless.
**Fix idea**: Branch `bracket` rendering on whether series results exist, and render Semis/ConfFinals/Finals lines with `(W)` and `4-2`-style scores like the `playoffs sim` output does.

---

### F-06 — `trade chain` dumps raw JSON instead of human-readable text — severity: medium
**What I did**:
```
$ nba3k --save /tmp/qa-flow.db trade chain 1
trade #1 — 2 offer(s):
  round 1: {"by_team":[{"players_out":["Sam Hauser"],"team":"BOS"},{"players_out":["Marcus Smart"],"team":"LAL"}],"round":1}
  round 2: {"by_team":[{"players_out":["Sam Hauser","Luka Garza"],"team":"BOS"},{"players_out":["Marcus Smart"],"team":"LAL"}],"round":2}
```
**Expected**: Pretty-printed offers like `Round 2: BOS sends Sam Hauser, Luka Garza → LAL sends Marcus Smart`.
**Actual**: The default text output is structured JSON inline. `--json` should be the gate for that; default should be human prose.
**Fix idea**: Add a `Display` impl for the chain rendering that walks each round's `by_team` and prints "TM1 sends ... → TM2 sends ...". Keep raw JSON behind `--json`.

---

### F-07 — Empty-save error messages inconsistent and unhelpful — severity: medium
**What I did**:
```
$ rm -f /tmp/x.db && nba3k --save /tmp/x.db status
Error: save has no season_state

$ rm -f /tmp/x.db && nba3k --save /tmp/x.db playoffs bracket
Error: no season_state in save

$ rm -f /tmp/x.db && nba3k --save /tmp/x.db season-summary
Error: no season_state in save

$ rm -f /tmp/x.db && nba3k --save /tmp/x.db awards
Awards (season 2026):
  MVP        —
  DPOY       —
  ...

$ rm -f /tmp/x.db && nba3k --save /tmp/x.db status --json
Error: save has no season_state
```
**Expected**: One consistent error message that points the user at `nba3k new --team <ABBR>`. JSON mode should emit `{"error": "..."}`.
**Actual**: Three different strings, plus `awards` silently produces empty rows instead of erroring. `--json` mode emits plain text errors that JSON consumers can't parse.
**Fix idea**: Centralize the empty-save check, with a message like `No save found at /tmp/x.db. Run "nba3k new --team BOS --save /tmp/x.db" to start.`. Have `awards` go through that same gate. Wrap errors as JSON when `--json` is passed.

---

### F-08 — `trade propose` and `draft pick` `--help` show no argument descriptions — severity: medium
**What I did**:
```
$ nba3k trade propose --help
Usage: nba3k trade propose [OPTIONS] --from <FROM> --to <TO>

Options:
      --from <FROM>
      --to <TO>
      --send <SEND>
      --receive <RECEIVE>
      --json

$ nba3k draft pick --help
Usage: nba3k draft pick [OPTIONS] <PLAYER>

Arguments:
  <PLAYER>
```
**Expected**: Each arg has a one-line `help =` doc — what to pass (team abbrev? player name? player id?), whether multiple values are allowed, the format.
**Actual**: Args are bare. New users have to guess (e.g. is `--send` repeatable? do I pass quoted "Sam Hauser" or an ID? does `draft pick <PLAYER>` take a name or rank?).
**Fix idea**: Add `#[arg(help = "...")]` (or doc comments) on each of these args in the clap derive.

---

### F-09 — REPL has no prompt; cannot tell it's interactive — severity: medium
**What I did**:
```
$ nba3k --save /tmp/qa-flow4.db
status
save:     /tmp/qa-flow4.db
season:   2026 (PreSeason)
...
roster BOS
BOS roster (16 players):
...
quit
```
**Expected**: Some `nba3k>` prompt before each line, plus a one-line greeting like "REPL — type help, quit to exit".
**Actual**: Stdin is silently read. No prompt, no banner, no indication the user is in a REPL. New users will hit Ctrl-C wondering if it hung.
**Fix idea**: When stdin is a TTY and no `--script` is given, print a greeting and write `nba3k> ` before each readline.

---

### F-10 — REPL surfaces "error: error:" double prefix on bad commands — severity: low
**What I did**:
```
$ echo -e "asdfgh\nquit" | nba3k --save /tmp/qa-flow4.db
error: error: unrecognized subcommand 'asdfgh'

Usage: nba3k <COMMAND>

For more information, try '--help'.
```
**Expected**: Single `error: unrecognized command 'asdfgh' — try help`. No double `error:`. The hint should refer to REPL `help`, not the global `--help` (which doesn't apply once you're inside the REPL).
**Actual**: clap's error wrapper is itself prefixed with `error:`, then we prefix again, so the user sees `error: error: ...`. The "try `--help`" suggestion misleads — typing `--help` in the REPL won't do what they think.
**Fix idea**: Strip the inner clap prefix or use `cmd.try_get_matches_from` and format the error ourselves. Replace the `--help` hint with `try 'help'`.

---

### F-11 — Two-Way (TW) player wins Sixth Man of the Year — severity: medium
**What I did**:
```
$ nba3k --save /tmp/qa-flow.db awards
Awards (season 2026):
  MVP        Devin Booker
  DPOY       Bam Adebayo
  ROY        —
  Sixth Man  Sharife Cooper  (TW)
  MIP        —
  COY        —
```
**Expected**: 6MOTY excludes two-way contracts (NBA rule: must have an NBA contract & ≥65 games / 20 minutes per game).
**Actual**: A `(TW)` player won. Also `ROY`, `MIP`, `COY` are blank — voter pool seems to be partially unwired. The seed gives literally everyone OVR 73-74 age 25 so there's no ROY-eligible player, but `COY` (a coach award) and `MIP` (most-improved player) should be derivable from W-L and OVR delta even with the flat seed.
**Fix idea**: Filter 6MOTY voter pool to non-two-way contracts. Backfill the COY (coach of best record) and MIP (highest OVR delta over season) selectors so they are never blank in a fully-simmed season.

---

### F-12 — `season-summary` is inconsistent across seasons; missing Finals MVP and COY — severity: low
**What I did**:
```
$ nba3k --save /tmp/qa-flow.db season-summary    # season 2026, post-playoffs
Season 2026 summary:
  champion : BOS
  MVP        Devin Booker
  DPOY       Bam Adebayo
  ROY        —
  Sixth Man  Sharife Cooper  (TW)
  MIP        —

$ nba3k --save /tmp/qa-flow5.db season-summary   # season 2027, fresh after season-advance
Season 2027 summary:
  MVP        —
  DPOY       —
  ROY        —
  Sixth Man  —
  MIP        —
```
**Expected**: Per the charter ("Champion + finals MVP + awards bundle"), `season-summary` should always include both Champion and Finals MVP rows, and consistent column formatting. Once the user advances seasons, last season's summary should still be reachable somehow (`season-summary --season 2026`?).
**Actual**: Champion line only shows when one exists (silently dropped otherwise — fine, but it shifts the layout). Finals MVP row is missing entirely. COY missing from the bundle though `awards` shows it. Two different alignment styles: `champion : BOS` (space-colon) vs `MVP        Devin Booker` (column-padded). Once advanced, no way to view 2026's summary again.
**Fix idea**: Define a fixed schema for the summary, render every row even when blank, add `Finals MVP` and `COY`. Normalize the colon/spacing style. Consider `season-summary --season N` so prior seasons are reviewable.
