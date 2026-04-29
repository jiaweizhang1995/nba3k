# M33 — TUI wizard step + season-advance follow-through + docs

**Status**: ✅ Done
**Started**: 2026-04-29
**Completed**: 2026-04-29

## Goal

Surface `--from-today` to the TUI wizard, make `season-advance` write a fresh per-year `season_calendar` row so the parameterized schedule doesn't fall back to const defaults forever, and document the new feature with its known gaps.

## Sub-tasks

| # | Item | Status |
|---|------|--------|
| T9 | TUI new-game wizard `StartMode` step (Fresh / Today) | ✅ |
| T10 | `season-advance` writes next year's `season_calendar` row | ✅ |
| T11 | README "Start from Today" section + this phase doc | ✅ |

## Decisions

- **Wizard ordering**: SavePath → Team → **Start mode** → Mode → Season → Confirm. The Start step is rendered as a vertical 2-option toggle (↑/↓) above an i18n hint paragraph. No filter, no Picker — keeps the keystroke surface small.
- **i18n keys** (added in lockstep across `i18n.rs` / `i18n_en.rs` / `i18n_zh.rs`): `NewGameStartTitle`, `NewGameStartFresh`, `NewGameStartToday`, `NewGameStartTodayHint`.
- **`next_calendar_from_previous` heuristic**: shift +365 days, snap forward to the next Tuesday, then derive `end = +174 days` and `trade_deadline = +107 days` from that anchor. All-star / cup day offsets carry forward unchanged. Approximate but stable; the user (or a future M34 calendar-pulled-from-NBA endpoint) can override per-year.
- **No regression to legacy path**: pure-additive change. `cmd_new` (no flag) and `season-advance` both keep their previous outputs byte-identical except that `season-advance` now also writes one row to `season_calendar`.

## Files touched

- `crates/nba3k-cli/src/tui/screens/new_game.rs` (`Step::Start`, `StartMode`, picker, render, key handler, retreat logic, NewArgs.from_today wired)
- `crates/nba3k-core/src/i18n.rs` + `i18n_en.rs` + `i18n_zh.rs` (4 keys × 2 langs)
- `crates/nba3k-cli/src/commands.rs` (`next_calendar_from_previous` helper + season-advance hook + unit test)
- `crates/nba3k-cli/tests/season_advance_calendar.rs` (`#[ignore]` integ scaffold)
- `README.md` (Start-from-Today section + known gaps)
- `phases/PHASES.md` row M33

## Verification

```bash
cargo test --workspace                                     # 320 passed + 2 ignored
cargo run --bin nba3k -- --save /tmp/legacy.db new --team BOS   # legacy path unchanged
cargo run --bin nba3k -- --save /tmp/today.db new --team BOS --from-today
sqlite3 /tmp/today.db "SELECT season_year, start_date, end_date, trade_deadline FROM season_calendar;"
# → 2026|2025-10-21|2026-04-12|2026-02-05
./target/release/nba3k tui                                 # wizard shows new "Start mode" step
```

After running `season-advance` once on the live save, `season_calendar` will have an additional 2027 row with start_date snapped to a Tuesday roughly 365 days after the prior start.

## Polish items deferred to a future milestone

- `cmd_records --scope season` rewire to consult `player_season_stats` when box-score aggregate is empty (relevant for `--from-today` saves).
- Cup table backfill for current real-life season's group-stage / KO results.
- Per-player box-score backfill for completed games (would need a different ESPN endpoint and a much larger fetch budget).
- Localized labels for any user-visible string in `from_today.rs` (currently the importer prints English-only console output via `cmd_new`'s success message).
- TUI loading indicator for `--from-today` import (currently blocks the wizard for ≈30 s on a cold-cache run with no progress bar).
