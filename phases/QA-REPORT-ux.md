# QA Report — ux-tester

## Summary

- `--help` text: **medium-fail** — top-level helpful, but every subcommand argument (`<TEAM>`, `<NAME>`, `<PLAYER>`, `<ROLE>`, `<PHASE>`, `<PATH>`, `[COUNT]`, `--from`, `--to`, `--send`, `--receive`, `--season`, `--seed`, `--runs`) ships with NO description. clap shows the bare argument name only. Users have no idea what format `<PHASE>` accepts, that `<TEAM>` must be uppercase, that `<NAME>` is a substring match, or that `--send` accepts comma-separated lists.
- CLI text rendering: **medium-fail** — `roster` ID column has zero padding (8-digit and 10-digit IDs sit in a column labelled with width 5), there's a stray double-space inside `(TW)` names, `roster` never shows role/morale even after `roster-set-role`, `trade chain` text mode dumps raw JSON instead of a human-readable chain, `chemistry` text rounds aggressively (0.440) while JSON returns the precise value (0.5199…), and `awards` text omits the All-NBA / All-Defensive teams that the JSON includes.
- Error messages: **medium-fail** — wording is inconsistent (`team 'ZZZ' not found in seed` vs `no team 'ZZZ'`; `no player matching 'X'` vs `no player matches 'X'`), `--from`/`--to` only accept exact uppercase abbrevs with no hint, the `error: error: ...` double-prefix appears in REPL mode, and an internal Rust *code comment* (about parsing strategy) is shipped to users as the help body.
- REPL: **high-fail** — there is no prompt at all, no banner, no farewell. `help` from inside the REPL prints an internal developer comment plus the Cargo `Command` enum dump. There is no command-completion, history, or hint. EOF/empty input/Ctrl-D all silently exit 0. New users will think the REPL is broken.

## Findings

### F-01 — REPL prints internal developer comment as help body — severity: high

**What I did**:
```
$ printf 'help\nquit\n' | ./target/release/nba3k --save /tmp/qa-ux.db
error: A single REPL line is parsed using the same `Command` enum. We use a wrapper Parser when reading from stdin so `--save` is honored per-line if user wants to override

Usage: nba3k <COMMAND>

Commands:
  new              Create a new save file. Writes to the path in --save
  ...
```
**Expected**: Either (a) `help` prints a friendly REPL command list, or (b) `help` is treated as `--help` and prints the standard help message.
**Actual**: clap's parse-error path triggers, with the *clap About string* set to a verbatim Rust source-code comment about how the REPL line parser works. This is a developer note that has leaked into user-facing output. The same comment shows up at the top of the help body and is then duplicated by the outer `Error:` wrapper.
**Fix idea**: In the REPL `Cli` struct (likely `crates/nba3k-cli/src/repl.rs` or wherever the wrapper Parser is defined), replace the `#[command(about = "...")]` derive's value (currently a code-design comment) with a normal one-liner like `"NBA 2K-style GM mode — REPL"`. Add a real `help` REPL builtin that prints the command list.

### F-02 — REPL has no prompt, banner, or exit message — severity: high

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db
(no output, no prompt — looks frozen)

$ printf 'quit\n' | ./target/release/nba3k --save /tmp/qa-ux.db
$ echo $?
0
(no banner, no "goodbye", no prompt, totally silent)
```
**Expected**: a banner with version + save info, and a prompt like `nba3k> ` on every input cycle so the user knows the REPL is waiting.
**Actual**: completely silent. Indistinguishable from a hung process when launched interactively. No way for a user to know the program accepted their input until they press Enter.
**Fix idea**: print a one-line banner (`nba3k 0.1.0 — save /tmp/qa-ux.db (BOS, season 2026 PreSeason). type 'help' or 'quit'.`) on REPL entry and a `nba3k> ` prompt before each readline. Optionally print `bye` on quit. Use `rustyline` or similar for editing/history.

### F-03 — `trade chain` text mode dumps raw JSON instead of a chain view — severity: high

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db trade chain 1
trade #1 — 1 offer(s):
  round 1: {"by_team":[{"players_out":["Payton Pritchard"],"team":"BOS"},{"players_out":["LeBron James"],"team":"LAL"}],"round":1}
```
**Expected**: A human-readable diff like
```
trade #1 (rejected) — BOS / LAL — 1 round
  round 1
    BOS sends:    Payton Pritchard
    LAL sends:    LeBron James
    verdict:      rejected — "LeBron James is untouchable"
```
**Actual**: text mode emits the raw serialized JSON of the offer, in a single line. This is the `trade` flow's most diagnostic command and it is unreadable.
**Fix idea**: in the trade `chain` text formatter, walk the rounds and pretty-print the players-out per team plus the per-round verdict / counter-offer. Keep the JSON path in `--json`.

### F-04 — Every subcommand argument ships with no description — severity: medium

**What I did**:
```
$ ./target/release/nba3k roster --help
(M2) Show a team roster

Usage: nba3k roster [OPTIONS] [TEAM]

Arguments:
  [TEAM]

Options:
      --json
      ...
```
Same for `player <NAME>`, `roster-set-role <PLAYER> <ROLE>`, `chemistry <TEAM>`, `sim-day [COUNT]`, `sim-to <PHASE>`, `draft pick <PLAYER>`, `trade chain <ID>`, `trade respond <ID> <ACTION>` (only `<ACTION>` has a doc!), `trade propose --from/--to/--send/--receive`, `dev calibrate-trade --runs`, etc.
**Expected**: every positional and flag should have a one-line description from `#[arg(help = "...")]` so `--help` is self-explanatory.
**Actual**: clap renders just the bare token. New users have to guess: is `<TEAM>` a 3-letter code? a city? a numeric ID? Is `<PHASE>` PascalCase like `status` shows ("PreSeason", "RegularSeason") or kebab as `sim-to` actually accepts (`regular`, `regular-end`, `playoffs`, `trade-deadline`)?
**Fix idea**: add `help = "..."` strings to every `#[arg]` in the clap structs. Specifically call out:
- `<TEAM>`: "team abbreviation (e.g., BOS, LAL) — defaults to your save's team"
- `<NAME>` / `<PLAYER>`: "player name (case-insensitive substring match)"
- `<PHASE>`: "phase: regular | regular-end | playoffs | trade-deadline"
- `--send` / `--receive`: "comma-separated player names"
- `[COUNT]`: "number of days (default 1)"

### F-05 — `roster` table has broken column alignment when player IDs vary in width — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db roster BOS
BOS roster (16 players):
ID     NAME                          POS  AGE  OVR  POT
688883990  Payton Pritchard              PG    25   74   80
870085358  Nikola Vučević                C     25   74   80
4243287520  Luka Garza                    C     25   74   80
32103028  Max Shulga                    SG    25   73   79
```
**Expected**: the `ID` column should pad to the widest ID so `NAME` lines up, or use a sensible left-aligned width like 12.
**Actual**: header `ID    ` is 5 wide; 9-digit IDs eat into NAME (1 trailing space), 10-digit IDs eat further. `Max Shulga`'s 8-digit ID under-fills, breaking the visual grid. Also the `NAME` column is exactly 30 chars and "John Tonje  (TW)" still has a stray double-space between the name and `(TW)` — that's a name-format bug, not just padding.
**Fix idea**: compute `max(id.to_string().len())` per render and pad ID column to that width + 2. Strip duplicate whitespace from `name` when adding the `(TW)` suffix.

### F-06 — `roster` never shows role or morale even after `roster-set-role` — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db roster-set-role "Jayson Tatum" star
Jayson Tatum: role -> Star (morale 0.80)
$ ./target/release/nba3k --save /tmp/qa-ux.db roster BOS
ID     NAME                          POS  AGE  OVR  POT
...
1931031263  Jayson Tatum                  PF    25   73   79
```
**Expected**: roster table should have ROLE and MORALE columns so the user can see the effect of `roster-set-role` and the chemistry inputs. JSON output should include `role` and `morale`.
**Actual**: roster has only POS/AGE/OVR/POT in both text and JSON. The chemistry breakdown later shows "role distribution -0.100" without showing what each role actually is. The user has no in-game way to view assigned roles.
**Fix idea**: add `ROLE` (e.g., `Star/Starter/Sixth/Role/Bench/Prospect`) and `MORALE` (0.00–1.00) columns to roster text output and roster JSON.

### F-07 — `chemistry` text and JSON disagree on the score, and reasons are reordered — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db chemistry BOS
chemistry BOS: 0.440
  baseline               +0.700
  positional balance     -0.160
  role distribution      -0.100
  scheme fit             +0.000
  morale                 +0.000

$ ./target/release/nba3k --save /tmp/qa-ux.db chemistry BOS --json
{
  "score": 0.5199999999999998,
  "reasons": [
    {"label": "baseline", "delta": 0.7},
    {"label": "role distribution", "delta": -0.10000000000000003},
    {"label": "positional balance", "delta": -0.08000000000000002},
    {"label": "scheme fit", "delta": 0.0},
    {"label": "morale", "delta": 0.0}
  ]
}
```
**Expected**: text and JSON should agree. score(text) == score(json). reason orderings should match. `0.700 + -0.080 + -0.100 = 0.520` matches the JSON, so the text rendering is wrong — either the score is computed twice or it sums different deltas.
**Actual**: text shows `chemistry BOS: 0.440` but the actual score is 0.520. Text shows positional balance `-0.160` but JSON has `-0.080`. There are two bugs here: (a) the displayed score is wrong, and (b) at least one reason delta is doubled in text. The reason list also reorders between the two outputs.
**Fix idea**: have the text formatter consume the same `ChemistryReport` struct that serializes to JSON, so they cannot diverge. This looks like the text path is recomputing rather than reading the precomputed values.

### F-08 — `awards` text output omits All-NBA and All-Defensive teams that JSON returns — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db awards
Awards (season 2026):
  MVP        —
  DPOY       —
  ROY        —
  Sixth Man  —
  MIP        —
  COY        —

$ ./target/release/nba3k --save /tmp/qa-ux.db awards --json
{
  "all_defensive": [[], []],
  "all_nba": [[], [], []],
  "coy": null, "dpoy": null, ...
}
```
**Expected**: text mode should at least mention the All-NBA 1st/2nd/3rd teams and All-Defensive 1st/2nd teams, even if empty; or document that text mode is a subset and hint at `--json` for full data.
**Actual**: text and JSON have different schemas. `season-summary` text is even shorter — it drops COY too.
**Fix idea**: render All-NBA and All-Defensive teams in text output as
```
  All-NBA 1st: [Player, Player, ...]
  All-NBA 2nd: ...
```

### F-09 — Error message wording is inconsistent across commands — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/x.db new --team ZZZ
Error: team 'ZZZ' not found in seed

$ ./target/release/nba3k --save /tmp/qa-ux.db roster ZZZ
Error: no team 'ZZZ'

$ ./target/release/nba3k --save /tmp/qa-ux.db chemistry ZZZ
Error: no team 'ZZZ'

$ ./target/release/nba3k --save /tmp/qa-ux.db player NotReal
Error: no player matching 'NotReal'

$ ./target/release/nba3k --save /tmp/qa-ux.db roster-set-role "Nobody Real" star
Error: no player matches 'Nobody Real'
```
**Expected**: same kind of error → same wording. ideally `unknown team 'ZZZ' (try one of: ATL, BOS, ...)` so the user can recover.
**Actual**: 3 different phrasings for the same "team not found" situation, 2 different phrasings for "player not found". None of them list the valid options.
**Fix idea**: extract one `pub fn unknown_team(s)` and `pub fn unknown_player(s)` helper that produces a consistent, actionable error including a few candidate suggestions (cheapest is `did you mean` via Levenshtein on the abbrev list).

### F-10 — `trade propose` accepts empty `--send` and silently auto-completes a trade — severity: high

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db trade propose --from BOS --to LAL --send "" --receive "LeBron James"
trade #1 — verdict: reject(other("lebron james is untouchable")) | status: rejected | round: 1 | teams: BOS/LAL

$ ./target/release/nba3k --save /tmp/qa-ux.db trade propose --from BOS --to LAL
trade #2 — verdict: accept | status: accepted | round: 1 | teams: BOS/LAL
```
**Expected**: `--send` empty (or both `--send` and `--receive` missing) should error: `Error: --send must list at least one player`. A trade with zero outgoing players is not a trade.
**Actual**: empty `--send` is accepted and the trade is proposed; with both flags missing the trade is *immediately accepted* (BOS gets … nothing, in exchange for nothing, but the engine logs it as accepted). That is both a UX failure (the error message is missing) and a data integrity failure (an empty trade hits the negotiation chain).
**Fix idea**: in the trade `propose` handler, validate that `--send` and `--receive` each parse to a non-empty `Vec<PlayerName>`. Return `Error: --send requires at least one player` / `Error: --receive requires at least one player` before reaching the evaluator.

### F-11 — Error / verdict casing is inconsistent and verdict leaks Rust enum debug format — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db trade propose --from BOS --to LAL --send "" --receive "LeBron James"
trade #1 — verdict: reject(other("lebron james is untouchable")) | status: rejected | round: 1 | teams: BOS/LAL

$ ./target/release/nba3k --save /tmp/qa-ux.db trade list --json
{
  "id": 1,
  "verdict": "Reject(Other(\"LeBron James is untouchable\"))",
  ...
}
```
**Expected**: human-readable verdict like `verdict: reject — "LeBron James is untouchable"` in text, and `{"verdict": "reject", "reason": "LeBron James is untouchable"}` in JSON.
**Actual**: text mode lowercases the entire enum debug print (so the proper noun "LeBron James" becomes "lebron james"), and JSON ships the raw `Debug` form `Reject(Other(\"...\"))` — that's a Rust enum literal, not an API surface.
**Fix idea**: implement `Display` for the verdict enum that takes a (reason: Option<String>) and serialize it as a tagged JSON object `{verdict, reason}`. Don't lowercase player names.

### F-12 — REPL emits `error: error:` double-prefix and `Error: error:` triple-prefix on bad input — severity: low

**What I did**:
```
$ printf 'asdfgh\nquit\n' | ./target/release/nba3k --save /tmp/qa-ux.db
error: error: unrecognized subcommand 'asdfgh'

Usage: nba3k <COMMAND>

For more information, try '--help'.

Error: error: unrecognized subcommand 'asdfgh'

Usage: nba3k <COMMAND>

For more information, try '--help'.
```
**Expected**: one error line — `unknown command 'asdfgh' — type 'help' to see commands` — and the REPL keeps running.
**Actual**: clap's `error:` prefix is printed once by clap and a second time wrapped by `anyhow` / the outer error handler, then the entire help message is printed twice (once by clap, once by the wrapper). Three commands with bad input fills the screen.
**Fix idea**: in the REPL loop, downgrade clap parse errors to a single one-line `unknown command 'X' — type 'help'` and continue, instead of letting the error propagate out the wrapper.

### F-13 — clap parse errors return mixed exit codes (`0` vs `2`) on otherwise identical bad input — severity: low

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db sim-day abc
error: invalid value 'abc' for '[COUNT]': invalid digit found in string
For more information, try '--help'.
$ echo $?
0     # ← but it's an error!

$ ./target/release/nba3k --save /tmp/qa-ux.db awards --season abc
error: invalid value 'abc' for '--season <SEASON>': invalid digit found in string
For more information, try '--help'.
$ echo $?
2

$ ./target/release/nba3k --save /tmp/qa-ux.db sim-to NotARealPhase
Error: unknown phase 'notarealphase': use regular|regular-end|playoffs|trade-deadline
$ echo $?
1
```
**Expected**: any error exits non-zero. Failing parse → `2`, runtime error → `1`, success → `0`.
**Actual**: positional-arg parse failures exit `0` (a positional clap error is being swallowed somewhere), so scripts can't tell that `sim-day abc` failed. `--season abc` correctly exits 2 because clap handles flag errors itself.
**Fix idea**: ensure the dispatcher always returns the clap `Error::exit_code()` (typically 2) for parse failures rather than printing-and-returning-Ok. Probably a missing `?` on the result of `parse_from(...)` somewhere in the REPL/CLI dispatch path.

### F-14 — `load` subcommand requires `<PATH>` even though `--save` is the global way to specify the file — severity: low

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db load
error: the following required arguments were not provided:
  <PATH>
Usage: nba3k load <PATH>

$ ./target/release/nba3k load --help
(no help text shown for what PATH should be)
```
**Expected**: either `load` re-uses `--save` (matching the rest of the CLI), or the help body explains that `<PATH>` overrides `--save`.
**Actual**: `load` is the only subcommand that takes a positional save path, and the conflict between `--save` and the positional is not documented. The help text reads "Load an existing save (no-op if --save points at one)" which makes the positional even more confusing.
**Fix idea**: drop the positional `<PATH>` and have `load` require `--save`. The user already opens the save by passing `--save`; `load` should be either a no-op echo of the save, or removed.

### F-15 — `status` reports phase as PascalCase (`PreSeason`) but `sim-to` accepts only kebab/lowercase — severity: low

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-ux.db status
season:   2026 (PreSeason)
...

$ ./target/release/nba3k --save /tmp/qa-ux.db sim-to PreSeason
Error: unknown phase 'preseason': use regular|regular-end|playoffs|trade-deadline

$ ./target/release/nba3k --save /tmp/qa-ux.db sim-to RegularSeason
Error: unknown phase 'regularseason': use regular|regular-end|playoffs|trade-deadline
```
**Expected**: the phase identifiers shown by `status` should be the same identifiers `sim-to` accepts. Either status shows `regular-season` or `sim-to` accepts `RegularSeason`.
**Actual**: a user doing the obvious thing — copy the phase name out of `status` into `sim-to` — fails. The error message lists 4 phases (`regular | regular-end | playoffs | trade-deadline`) but the `PreSeason` shown by status isn't even in that list.
**Fix idea**: either accept both forms (PascalCase + kebab), or have `status` print the kebab form. Document both names in `sim-to --help`.

### F-16 — Top-level help mixes (M2)/(M3)/(M5)/(M6) milestone tags into user-facing descriptions — severity: low

**What I did**:
```
$ ./target/release/nba3k --help
Commands:
  sim-day          (M2) Sim a number of days
  sim-to           (M2) Sim until a phase
  standings        (M2) League standings
  roster           (M2) Show a team roster
  roster-set-role  (M5) Assign a role tag to a player. Roles: star, starter, sixth, role, bench, prospect
  trade            (M3) Trade subcommands
  draft            (M5) Draft subcommands
  chemistry        (M5) Show team chemistry breakdown
  ...
```
**Expected**: end users don't care about milestone numbers. These are dev-internal labels.
**Actual**: `(M2)`, `(M3)`, `(M5)`, `(M6)` are baked into the public `--help` description for almost every command. Looks unfinished.
**Fix idea**: strip the `(MN)` prefix from each `#[command(about = ...)]`. Keep the milestone tag in commit messages / docs, not in help text.
