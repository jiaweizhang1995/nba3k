# QA Report — bugs

## Summary
- **Phase ordering**: HIGH — `playoffs sim` and `draft sim` run successfully from PreSeason with 0 games played, printing a fake "Champion: BOS" and a draft order. No phase guard.
- **Idempotency / re-run**: HIGH — `playoffs sim` and `draft sim` can be run repeatedly; each run prints a fresh sim with no warning that prior results existed.
- **REPL crash on error**: HIGH — any single invalid command kills the entire REPL/script run. Subsequent valid commands never execute.
- **REPL `--help`**: MEDIUM — `--help` inside REPL leaks an internal Rust doc-comment as the error message ("A single REPL line is parsed using the same `Command` enum...").
- **Empty trade accepted**: MEDIUM — `trade propose --from BOS --to LAL` with no `--send` or `--receive` is accepted as a valid completed trade.
- **Player-name disambiguation inconsistent**: MEDIUM — `roster-set-role James star` rejects ambiguous "James" (4 candidates), but `player James` silently picks James Harden.
- **Duplicated error prefix**: LOW — clap-derived error messages print `error: error:` twice.
- **REPL with no TTY**: LOW — running with no subcommand and piped/empty stdin exits silently with no banner or prompt indication.
- **Read commands on empty save**: LOW — `awards`, `playoffs bracket`, `season-summary` print formatted output on a fresh save with no warning that nothing has been simmed.
- Pass: bad save path, save overwrite refusal, bad team abbrev, bad mode, self-trade, missing player, role typo, sim-day numeric edges, concurrent SQLite access, god-mode bypass.

## Findings

### F-01 — `playoffs sim` runs from PreSeason and announces a "Champion" — severity: high
**What I did**:
```
$ rm -f /tmp/qa-bugs-flow.db
$ ./target/release/nba3k --save /tmp/qa-bugs-flow.db new --team BOS
created save /tmp/qa-bugs-flow.db (team=BOS mode=standard season=2026 seed_used=true)
$ ./target/release/nba3k --save /tmp/qa-bugs-flow.db playoffs sim
Playoffs (season 2026):
  R1: ATL 4 - 2 IND
  R1: CHO 4 - 3 CHI
  ...
  Finals: DAL 0 - 4 BOS
Champion: BOS
$ ./target/release/nba3k --save /tmp/qa-bugs-flow.db status
season:   2026 (PreSeason)
day:      0
schedule: 1230 games (1230 unplayed)
```
**Expected**: Reject with something like `Error: cannot sim playoffs in phase PreSeason; finish regular season first` (the same way `season-advance` correctly enforces phase).
**Actual**: Sim runs. Prints "Champion: BOS" in a save where 0 games have been played. Save state is unchanged afterward (still PreSeason, 1230 unplayed) — so the champion is fictional and misleading.
**Fix idea**: Add a phase guard at the top of the playoffs-sim handler matching `season-advance`'s style: require `current_phase ∈ {Playoffs, RegularEnd}` and that all regular-season games are played.

### F-02 — `draft sim` runs from PreSeason — severity: high
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-flow.db draft sim
Draft sim — 30 picks made:
   1. ATL -> AJ Dybantsa (ovr=84, pot=96)
   2. BOS -> Cameron Boozer (ovr=82, pot=93)
   ...
$ ./target/release/nba3k --save /tmp/qa-bugs-flow.db status
season:   2026 (PreSeason)
```
**Expected**: Reject — drafts only happen in OffSeason (per the in-game flow). Should surface a phase error.
**Actual**: Runs and prints 30 picks regardless of phase.
**Fix idea**: Same phase guard pattern as F-01 (require OffSeason).

### F-03 — `playoffs sim` is idempotent / silently re-runs — severity: high
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-repeat.db playoffs sim
...
Champion: BOS
$ ./target/release/nba3k --save /tmp/qa-bugs-repeat.db playoffs sim
... (different sim with no error)
Champion: BOS
```
**Expected**: Once a champion is crowned, further `playoffs sim` calls should refuse (`Error: playoffs already simulated for season 2026`) or at minimum show the prior result with a warning.
**Actual**: Re-sims silently. Combined with F-01 this is even worse — it suggests the command is a stateless side-computation rather than an actual phase advance.
**Fix idea**: Persist playoff results once on first sim and reject subsequent calls; or require explicit `--rerun` flag.

### F-04 — `draft sim` is idempotent / silently re-runs — severity: high
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-draft2.db draft sim
... 30 picks ...
$ ./target/release/nba3k --save /tmp/qa-bugs-draft2.db draft sim
... 30 different picks ...  (no error)
```
**Expected**: Refuse second call once draft is done for the season.
**Actual**: Re-runs with different prospects (the second run shows lower-OVR prospects, suggesting the first run's picks were not persisted at all).
**Fix idea**: Same as F-03.

### F-05 — Single REPL error kills the entire REPL — severity: high
**What I did**:
```
$ printf "bogus\nstatus\nquit\n" | ./target/release/nba3k --save /tmp/qa-bugs-repl.db
error: error: unrecognized subcommand 'bogus'

Usage: nba3k <COMMAND>

For more information, try '--help'.

Error: error: unrecognized subcommand 'bogus'

Usage: nba3k <COMMAND>

For more information, try '--help'.

$ echo $?
1
```
**Expected**: REPL prints the error, then keeps reading lines. `status` should still execute, `quit` should exit cleanly.
**Actual**: After the bad first line the binary exits with code 1. `status` and `quit` are never read. This makes the REPL effectively unusable for any user who fat-fingers a command.
**Fix idea**: Catch the parse/execution error in the REPL loop, print it, and continue. Only break on EOF or explicit `quit`/`exit`.

### F-06 — `--script` halts on first invalid line, ignores remaining lines — severity: medium
**What I did**:
```
$ cat /tmp/qa-bugs-script.txt
status
bogus_command
status
quit
$ ./target/release/nba3k --save /tmp/qa-bugs-repl.db --script /tmp/qa-bugs-script.txt
... first status output ...
Error: /tmp/qa-bugs-script.txt:2: `bogus_command`

Caused by:
    error: unrecognized subcommand 'bogus_command'
```
**Expected**: This may be intended ("strict script mode"), but there is no `--continue-on-error` flag or warning to the user that the script aborts on the first error. Tied to F-05.
**Actual**: Aborts with no recovery option. Good `file:line` formatting at least.
**Fix idea**: Either continue-by-default with non-zero exit code on completion, or document the strict behavior in `--script`'s help text.

### F-07 — `--help` inside the REPL leaks an internal Rust doc-comment — severity: medium
**What I did**:
```
$ printf "%s\n" "--help" "quit" | ./target/release/nba3k --save /tmp/qa-bugs-repl.db
error: A single REPL line is parsed using the same `Command` enum. We use a wrapper Parser when reading from stdin so `--save` is honored per-line if user wants to override

Usage: nba3k <COMMAND>
...
Error: A single REPL line is parsed using the same `Command` enum. We use a wrapper Parser when reading from stdin so `--save` is honored per-line if user wants to override
```
**Expected**: A user-friendly help banner, like the top-level `nba3k --help` — or at minimum no developer-facing prose.
**Actual**: The clap "about" string for the REPL wrapper is a developer-targeted code comment that gets shown to the end user as the error message and as the Cli description.
**Fix idea**: Replace the doc-comment on the REPL wrapper Parser with a normal user-facing one-liner, e.g. `"NBA 2K-style GM mode REPL — type 'help' for commands, 'quit' to exit"`.

### F-08 — Empty trade is accepted — severity: medium
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-trade.db trade propose --from BOS --to LAL
trade #1 — verdict: accept | status: accepted | round: 1 | teams: BOS/LAL
```
**Expected**: Reject with `Error: trade must include at least one player on each side` (or similar). Self-trade is correctly rejected; empty trade should be too.
**Actual**: Engine accepts an empty trade as a valid, completed transaction. Likely also corrupts CBA / asset accounting if the engine ever bookkeeps "this trade happened".
**Fix idea**: Validate `send.len() + receive.len() > 0` (and probably both sides nonempty) at the top of the propose handler.

### F-09 — Player name disambiguation is inconsistent across commands — severity: medium
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-role.db roster-set-role "James" star
Error: ambiguous match for 'James': 4 candidates (e.g. ["LeBron James", "Bronny James", "James Harden", "Sion James"]). Refine the query.

$ ./target/release/nba3k --save /tmp/qa-bugs-role.db player "James"
James Harden (PG) — CLE
age 25 | OVR 74 | POT 80
```
**Expected**: Both commands either reject ambiguous matches with the same helpful candidate list, or both pick a deterministic best match with the same rule.
**Actual**: `roster-set-role` rejects (good — destructive op); `player` silently picks James Harden (could surprise a user looking up LeBron). The ambiguous-error in `roster-set-role` is great and should be reused.
**Fix idea**: Extract the candidate-resolution helper from `roster-set-role` and reuse it in `player`. For read-only commands, a "did you mean?" list followed by the best match is also acceptable, but silent first-hit is the worst option.

### F-10 — Error messages prefixed with "error: error:" — severity: low
**What I did**:
```
$ printf "bogus\nquit\n" | ./target/release/nba3k --save /tmp/qa-bugs-repl.db
error: error: unrecognized subcommand 'bogus'
...
Error: error: unrecognized subcommand 'bogus'
```
**Expected**: One `error:` prefix.
**Actual**: clap's `error:` prefix is wrapped by anyhow's `Error:` and the inner message also begins with `error: ` — yielding `error: error:` and `Error: error:`.
**Fix idea**: When forwarding clap parse errors from REPL/script lines, strip the leading `error: ` from `e.to_string()` before wrapping in anyhow context. Or use `clap::Error::print()` directly without the anyhow wrap.

### F-11 — Read commands on a freshly-created save show output with no "no data yet" hint — severity: low
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-empty.db awards
Awards (season 2026):
  MVP        —
  DPOY       —
  ...

$ ./target/release/nba3k --save /tmp/qa-bugs-empty.db playoffs bracket
R1 bracket (season 2026):
  East ATL (1) v IND (8)
  East CHO (4) v CHI (5)
  ...
```
**Expected**: A note like `(no games simmed yet — bracket is provisional, seeded by team ID)` would help. The bracket appears legitimate but the seeds are alphabetical/by-id, not by record (every team is 0-0).
**Actual**: Output looks authoritative even though no games were played. The dashes in `awards` are subtle but reasonable; `playoffs bracket` is more misleading because it shows seed numbers.
**Fix idea**: When all teams are 0-0, prepend a "no regular-season results yet — seeding placeholder only" line.

### F-12 — Empty `trade list` prints headers with trailing whitespace — severity: low
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-deep.db trade list
  ID  STATUS    ROUND  TEAMS     
```
**Expected**: Either suppress headers when there are no rows, or print "No active trades." Either way, no trailing spaces.
**Actual**: Bare header row with trailing whitespace and nothing else. UX feels broken on first run.
**Fix idea**: Early-return with `println!("No active trades.")` when the result set is empty.

### F-13 — `roster` ID column header is too narrow for actual values — severity: low
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-god2.db roster BOS
BOS roster (16 players):
ID     NAME                          POS  AGE  OVR  POT
688883990  Payton Pritchard              PG    25   74   80
```
**Expected**: ID column header width matches the value column.
**Actual**: Header is 5 chars (`ID   `) but values are 9-10 chars, so the table is misaligned starting from column 1. (Likely also UX-tester territory; logging here for completeness because it surfaces immediately when invoking `roster`.)
**Fix idea**: Either zero-pad / right-align IDs to a fixed width, or replace huge u64-like IDs in user-facing output with a sequential index, keeping the raw ID for `--json` only.

### F-14 — `playoffs sim` does not advance save phase even when run "successfully" — severity: medium
**What I did**:
```
$ rm -f /tmp/qa-bugs-state2.db
$ ./target/release/nba3k --save /tmp/qa-bugs-state2.db new --team BOS
$ ./target/release/nba3k --save /tmp/qa-bugs-state2.db playoffs sim   # prints Champion: BOS
$ ./target/release/nba3k --save /tmp/qa-bugs-state2.db season-advance
Error: season advance requires phase Playoffs or OffSeason; current phase is PreSeason
```
**Expected**: Either `playoffs sim` is gated on Playoff phase (see F-01) and advances the save to OffSeason on success, OR it explicitly documents itself as a read-only "what-if" simulation. Right now it does neither.
**Actual**: `playoffs sim` runs, prints a champion, but `status` still reports PreSeason and `season-advance` refuses. The user is left thinking the season is over when it never started.
**Fix idea**: Pick a contract: (a) gate-and-advance, or (b) rename to `playoffs preview` and label output as hypothetical.

### F-15 — `sim-to` does not accept the obvious phase names from `status` output — severity: low
**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-bugs-neg.db status | grep season
season:   2026 (PreSeason)
$ ./target/release/nba3k --save /tmp/qa-bugs-neg.db sim-to PreSeason
Error: unknown phase 'preseason': use regular|regular-end|playoffs|trade-deadline
```
**Expected**: Accept the phase strings shown in `status` (`PreSeason`, `Regular`, `Playoffs`, `OffSeason`), case-insensitively. The error message lists hyphenated lower-case names that don't match what `status` displays.
**Actual**: User has to learn a separate vocabulary (`regular-end`, `trade-deadline`) that `status` never uses. Discoverability is poor.
**Fix idea**: Accept all phase aliases in both casings; or update `status` to print the same casing/spelling. Mention valid values in the `sim-to` `--help` text (currently empty for the `<PHASE>` arg).

## Notes on what passed
- Bad save path → clean error mentioning sqlite + path.
- Save-overwrite → refused with `refusing to overwrite existing save at <path>`.
- Bad team abbrev (`ZZZ`, empty) → `team 'ZZZ' not found in seed`. Lowercase `bos` is silently uppercased — fine.
- Bad mode → `unknown mode 'notreal': use standard|god|hardcore|sandbox`.
- Self-trade → `cannot trade with yourself`.
- Missing players → `no player 'Nobody Real' on team`.
- Role typo → `unknown role 'stra': use star|starter|sixth|role|bench|prospect`.
- `season-advance` mid-PreSeason → clean refusal with phase context.
- `sim-day -5`, `abc`, missing arg, 99999 → all handled (clap blocks negatives; 99999 caps at 181 days).
- Concurrent CLI runs against the same SQLite save → both succeeded, no corruption.
- `--god` flag and `--mode god` → both bypass CBA on a known-illegal trade (LeBron untouchable → accepted).
- `trade chain 99999`, `trade respond 99999 accept`, `trade respond 1 surrender` → clean errors.
