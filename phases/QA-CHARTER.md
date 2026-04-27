# QA Charter — nba3k-claude

## Context

**Product**: Rust CLI (no GUI). REPL + argv subcommands. Binary at `./target/release/nba3k`. Seed already shipped at `data/seed_2025_26.sqlite`.

**Game model**: NBA 2K MyGM-style — single-team GM controls one franchise; AI runs the other 29. Single-season MVP loop is complete (preseason → 82 games → trade deadline → playoffs → draft → next season).

**M1-M6 done**. M7 polish active. Workspace tests: 174 passing.

## Your job

Test the *user-facing* surface of the game — not unit tests. We already have those. We want to know:

1. **CLI text rendering quality** — alignment, column padding, line breaks, readability of `roster`, `standings`, `chemistry`, `awards`, `playoffs`, `draft board`, `trade evaluate` text output. (Yes "前端" here means CLI text rendering. There is no web UI.)
2. **Command UX** — is `--help` clear? do error messages tell the user what to do next? are command names guessable? does the REPL feel responsive?
3. **End-to-end game flow** — can a fresh user actually play one full season + offseason without getting stuck?
4. **2K MyGM fidelity** — does role/morale/chemistry/trade negotiation behave the way a 2K MyGM player would expect? Test specific scenarios from MyGM (e.g. "Star slotted as BenchWarmer should drop morale and demand trade").

## Your output

Each tester writes a markdown file at `phases/QA-REPORT-{your-slug}.md`. Format:

```markdown
# QA Report — {slug}

## Summary
- {short pass/fail per area}

## Findings

### F-{NN} — {short title} — severity: {high|medium|low}
**What I did**:
```
$ nba3k --save x.db roster BOS
{paste output}
```
**Expected**: ...
**Actual**: ...
**Fix idea**: ...

### F-{NN+1} ...
```

Severity guide:
- **high** — game-breaker, obvious bug, broken golden path
- **medium** — confusing UX, formatting clearly off, mechanic feels wrong
- **low** — polish nit, suggested improvement

## Working agreements

- **Use the release build**: `./target/release/nba3k --save /tmp/qa-{your-slug}.db ...`. Don't rebuild — already built.
- **Each tester gets a fresh save**. Don't share `/tmp/*.db` paths.
- **Capture exact CLI output** when reporting. Don't paraphrase.
- **Don't fix bugs**. Just report. Lead drives fix waves.
- **Stop after ~12 findings or when you've exhausted your charter** — diminishing returns past that point.

## Charters (one per tester)

### `ux-tester` — CLI formatting + help text + error messages

Walk through `nba3k --help`, every subcommand's `--help`, every read command's text output (with and without `--json`). Ask:
- Is alignment broken? Column widths wrong?
- Is the `--help` text helpful or just argv noise?
- When you give garbage input (typo'd team abbrev, missing arg, bad mode), is the error helpful?
- Is the REPL prompt obvious? Can you exit cleanly?

### `flow-tester` — end-to-end gameplay

Play a full season as a fresh user. Try to break the flow. Try to skip steps. Try invalid orderings (e.g. `playoffs sim` before regular season ends). Document what works smoothly and what makes you go "huh?".

### `2k-tester` — 2K MyGM fidelity

Compare against NBA 2K25/2K26 MyGM behaviour. Specific scenarios to try:
- Demote a Star to BenchWarmer (`roster-set-role`). Check morale dropped. Try to trade them — does AI accept? Does evaluator note morale?
- Build a star-stack roster (3+ Stars on one team). Run `chemistry`. Does it penalize?
- Propose obvious-bad trade (your bench warmer for their MVP). Does it reject?
- Sim a full season. Awards: does MVP go to a high-scorer on a winning team? Does ROY only consider rookies?
- Run `season-advance`. Does a 19yo OVR-72 with high potential gain OVR? Does a 35yo regress?
- Run `chemistry BOS` for a real roster after seeding. Does the explanation make sense?

### `bug-hunter` — edge cases + invalid inputs + state corruption

Try:
- `nba3k --save /nonexistent/path/x.db status`
- `nba3k --save x.db new --team ZZZ` (invalid team)
- `nba3k --save x.db new --team BOS && nba3k --save x.db new --team LAL` (refuse overwrite?)
- `nba3k --save x.db trade propose --from BOS --to BOS ...` (self-trade)
- `nba3k --save x.db trade propose --from BOS --to LAL --send "Nobody Real" --receive "Also Nobody"` (missing players)
- Run `season-advance` mid-regular-season
- Run `playoffs sim` twice in a row
- Run `awards` on a save that hasn't simmed any games
- REPL: send invalid commands, malformed args, EOF, Ctrl-D
- Pipe garbage stdin

## Hand-off

When you're done:
1. Write your `phases/QA-REPORT-{your-slug}.md`
2. Update your task to `completed` with TaskUpdate
3. Send a one-line message to `team-lead`: `done — {N} findings in QA-REPORT-{slug}.md`
4. Go idle (don't spin)
