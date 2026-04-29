# PROGRESS.md — where the project is right now

> Generated at the M35 / `2d7a3cd` mark, refreshed after roster-cap bugfix
> (`7973832`). Keep this file in sync with `phases/PHASES.md` (the
> per-milestone log) on every milestone close.

## Current state

| | |
|---|---|
| Current binary | `nba3k` (CLI + REPL + TUI surfaces) |
| Workspace tests | **329 passed + 2 ignored** across 72 suites |
| Latest milestone | **M35 — Start From Today snapshot semantics** + post-M35 roster bugfix |
| Latest commit | `7973832 fix(roster): phase-aware roster bounds + season-start gate; drop --season from new-game` |
| Schema high-water mark | **V017** (next migration uses V018) |
| Default `new` behavior | **Live ESPN snapshot** (post-M34); pass `--offline` for the seed-anchored fresh-October path. The wizard no longer asks for a starting season — the bundled seed is anchored to 2025-26. |

## What works end-to-end

- Full season simulation with playoff bracket, awards, draft, season
  advancement, and progression pass.
- Trade engine with CBA validation, GM personalities, multi-round
  negotiation, 3-team trades.
- Live "Start From Today" — current standings, rosters, injuries, and
  season-to-date player stats imported from ESPN's public JSON API.
- TUI with 8-menu shell: Home / Roster / Rotation / Trades / Draft /
  Finance / Calendar / Settings.
- Bilingual TUI (English + 中文) via `t(lang, T::...)` lookup.
- Determinism: same seed → same season, asserted by the integ test.

## Milestone history (high-level)

See `phases/PHASES.md` for the full table with verification commands
and per-milestone docs.

| # | What | Status |
|---|------|--------|
| M1–M22 | Foundation through trade builder M22 | ✅ all done |
| M23–M30 | i18n, polish, trade builder redesign | ✅ all done |
| **M31** | Calendar decoupling + ESPN fetch layer | ✅ |
| **M32–M33** | `--from-today` importer + TUI wizard + season-advance | ✅ |
| **M34** | Live ESPN start is the default | ✅ |
| **M35** | Snapshot semantics (match NBA 2K behavior) | ✅ |

## Recent commits (most recent first)

```
7973832 fix(roster): phase-aware roster bounds + season-start gate; drop --season from new-game
3072745 fix(makefile): escape backticks in `make help` output
9822609 docs: normalize project docs into docs/ + Makefile
82aa1f7 chore: workspace rustfmt + fix clippy logic-bug in cmd_sim_pause
6abff80 fix(tui): align center-position rows in trade builder
2d7a3cd M35: snapshot semantics for --from-today (match NBA 2K behavior)
01db555 M34: live ESPN start is the default
ec48a9a M32-M33: --from-today live ESPN importer + TUI wizard + season-advance
566ee00 M31: calendar decoupling + ESPN fetch layer
83b36a0 M30: trade builder redesign (T32-T44)
c806bcf docs: rewrite todo-plan.md for M30 trade builder redesign
6c7d9f1 M27-M29: post-release polish (T20-T31)
71e832d chore: prune obsolete per-phase docs
05d4720 M26-T16: trades action picker via Enter
```

Use `git log --oneline -20` for a longer view.

## Known polish items (pulled from M33 / M35 phase docs)

These are not blockers — they are real but small. Pick from this list
when looking for a small high-value follow-up.

- **`cmd_records --scope season --stat ppg` falls back to box-score
  aggregate**, which is empty after a `--from-today` import. Rewire to
  consult `player_season_stats` when game logs are sparse.
- **Cup table backfill** for the current real-life season's
  group-stage / KO results. Today the importer leaves `cup_match`
  empty.
- **Per-player box-score backfill for completed games** — would need a
  different ESPN endpoint and a much larger fetch budget. Today, past
  games are deliberately not imported (M35 decision).
- **TUI loading indicator** for `--from-today` cold-cache import. The
  wizard freezes ~30-45 s on first run without progress feedback.
- **Parallelize the 30 sequential roster fetches** with
  `std::thread::scope` (10-worker pool) to drop import wall time
  from ~45 s → ~5 s. ESPN tolerates parallelism.
- **Localized labels in `from_today.rs`** (currently English-only
  console output via `cmd_new`'s success message).
- **Legacy clippy nits**: a handful of `unused_imports` / `dead_code` /
  `dropping_copy_types` warnings in `crates/nba3k-trade/` and
  `crates/nba3k-cli/src/{state,commands}.rs`. `make lint-strict` flags
  them; `make lint` accepts them while the cleanup backlog clears.

## What's NOT in this repo

These are deliberate omissions. Don't propose adding them without
talking to the human first.

- Per-possession sim engine — current sim is statistical (M2 era);
  per-possession is v2 territory.
- Restricted free agency / qualifying offers / Bird rights /
  sign-and-trade / trade exceptions / contract buyouts.
- Coaching scheme trees, assistant coaches, training camp bonuses.
- Boss firing the GM for poor results.
- Online play, multiplayer, server sync.
- Branding / mascots / arena / city economics.

## Working agreements (TL;DR)

- Each phase ends with a Bash-verifiable artifact recorded in
  PHASES.md.
- Schema is migration-first: never edit a committed `.sql`.
- TUI mutations route through `commands::dispatch`. No parallel
  mutation paths.
- All new TUI strings go through `t(lang, T::...)`.
- Bash `grep` is rewritten by the rtk hook in this user's environment.
  Use the Grep tool directly.
- Player names + team abbreviations stay English even in 中文 mode —
  they are data, not chrome.

## Where to record new work

- Active milestone: pick the next free `M{N}`, create
  `phases/M{N}-<slug>.md` from the existing template, add the row to
  `phases/PHASES.md`.
- Discoveries / surprises: write them into the relevant phase doc, not
  into a free-floating scratch file.
- Long-term backlog: this PROGRESS.md "Known polish items" section.
- Domain research: `RESEARCH.md` and `RESEARCH-NBA2K.md` are the
  reference — append, don't rewrite.
