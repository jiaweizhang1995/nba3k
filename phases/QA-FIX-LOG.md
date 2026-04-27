# QA Fix Log — wave 1-4

## QA pass

4 testers logged 55 findings to `phases/QA-REPORT-{ux,flow,2k,bugs}.md`.

## Fix waves

### Wave 1 — correctness blockers

- Empty trade rejected (`--send` / `--receive` must be nonempty).
- Phase guards: `playoffs sim` requires `Playoffs`; `draft sim`/`draft pick` require `OffSeason` (or `Playoffs` after finals).
- Idempotency: `playoffs sim` refuses if Finals already recorded for the season.
- **Trade accept actually moves players.** Added `apply_accepted_trade` walking `assets_by_team` round-robin; called from both `propose` (auto-accept path) and `respond accept`.
- `season-advance` regenerates schedule for the new season + clears prior season's schedule rows. Game IDs offset by `season × 10_000` so multi-season saves don't collide on the unique `game_id` index.
- New Store API: `clear_schedule_for_season(season)`.

### Wave 2 — REPL UX

- Replaced internal Rust doc-comment on `ReplLine` Cli wrapper with a user-facing `about` string.
- Pipe mode (`echo cmd | nba3k`) keeps going on errors instead of bailing on the first one. Returns non-zero at end if any command failed.
- Interactive REPL: added `help`/`?` builtin that prints command list. Strip duplicate `error:` prefix.

### Wave 3 — visible data

- `LeagueSnapshot::roster` now sorts by `(overall desc, id asc)` — fixes non-determinism between consecutive runs.
- `roster` table: ROLE + MORAL columns; ID column padded to widest u32; `(TW)` double-space stripped via `clean_name`.
- `player` text card: shows role, morale, NTC flag, trade kicker pct.
- `chemistry` text == JSON (root cause was non-deterministic roster iteration).
- `trade chain`: human-readable rounds (`BOS sends: X` / `LAL sends: Y`); rejection reason printed.
- Stripped `(MN)` milestone tags from `--help`.
- `sim-to` accepts PascalCase phase names (`RegularSeason`, `Playoffs`) plus `offseason`.

### Wave 4 — mechanics tuning

- `role_morale_drift`: symmetric (-0.10/rank both directions). Star→Bench = -0.40, Bench→Star = +0.40. Same-role = 0.0.
- `team_chemistry::role_distribution`: 3 stars = -0.6 (was -0.2), 4 = -1.2, 5+ = -1.8. Mismatched-star penalty 0.5 (was 0.4).
- Sixth Man filters `(TW)` two-way contracts post-hoc (promotes next eligible ballot entry).
- COY fallback: when no prev-season standings, awards best regular-season record team.
- Draft order: when current season's standings empty, falls back to prior season. Final tiebreak on `TeamId` (no more alphabetical-by-abbrev surprise).

## Results

- **174 workspace tests pass** (no regressions).
- Multi-season loop verified: new save → sim full season → playoffs sim → season-advance → 1230 unplayed games in season 2 → sim-to playoffs reaches Playoffs.
- Trade accept verified: Sam Hauser → LAL, LeBron → BOS.
- Chemistry text and JSON return identical values across consecutive runs.
- Roster shows role/morale; player card shows role/morale.

## Deferred (not blocking M7 ship)

- **F-01 (2k)**: seed has all players age 25, OVR 72-74. Real fix is upstream scrape data quality + position-aware OVR distribution. Major work — separate phase.
- **F-08 (2k)**: peer-OVR equal trade rejected as `insufficientvalue`. Evaluator calibration — needs `dev calibrate-trade` runs.
- **F-06 (2k)**: `scheme fit` always 0.000 because no real coach scheme variation in seed.
- **F-04 (2k)**: trade-demand surface for unhappy stars. New `messages` / `inbox` subcommand — feature work.
- **F-05 (flow)**: `playoffs bracket` post-sim doesn't show series scores — needs branch on persisted `series` rows.
- Per-arg `help =` descriptions still missing on most clap args (ux F-04, flow F-08). Polish pass.
- Verdict `Display` impl (still leaks `Debug` form in JSON — ux F-11).
- Inconsistent error wording across commands (ux F-09).
