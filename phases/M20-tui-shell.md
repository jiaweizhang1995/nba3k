# M20 — TUI Shell + 7-Menu + Home + Calendar + Saves

**Status:** ✅ Done 2026-04-27. Single-day completion via 3-worker team `nba3k-m20`.

## Outcome

- Worker A (`nba3k-m20-shell`): split `tui.rs` 1123 LoC monolith → `tui/mod.rs` + `tui/widgets.rs` + `tui/screens/{stub,home,saves,new_game,calendar,legacy}.rs`. Built menu shell, theme (Default + TV), action bar, FormWidget contract (TextInput/NumberInput/Picker/MultiSelect/Confirm/ActionBar), `--tv` and `--legacy` flags on `Tui` subcommand. Wave-0 contract revisited: added `Option<SaveCtx>` + mirrored fields on `TuiApp` + `refresh_save_ctx`/`switch_save`/`has_save` public API so the no-save entry → wizard flow works.
- Worker B (`nba3k-m20-home`): Home (4-panel dashboard: mandate / upcoming game / GM inbox / recent news), Saves overlay (full-screen modal: list/load/delete/export with confirm + path-input modals), New-game wizard (6-step: path → team → mode → season → seed → confirm). Per-screen state in `thread_local! RefCell<…>` to avoid widening `TuiApp`. All mutations through `dispatch` wrapped in `with_silenced_io`.
- Worker C (`nba3k-m20-calendar`): Calendar with 7×6 month grid + 6 sub-tabs (Schedule/Standings/Playoffs/Awards/All-Star/Cup). Sim controls Space/W/M/A/Enter on event days. Pause-on-event modal during `sim-week`/`sim-month`.

## Verification

- `cargo build --bin nba3k --release`: clean (0 errors, only pre-existing drop(snap) warning in commands.rs).
- `cargo test --workspace`: 275 passed, 1 ignored.
- TUI launch tested via Python pty.fork by both Worker A and Worker B (40×140 xterm-256color).
- CLI cuts hold: `compare`, `hof`, `records --scope season --stat ppg` all still work post-M20.
- TUI menu cuts hold: no path from menu to compare/records/hof/coach standalone (some accessible via Calendar sub-tabs).
- `nba3k tui --legacy` still launches the M19 5-tab dashboard.
- `nba3k tui` with no `--save` no longer bails — fires new-game wizard.



**Goal:** Make TUI playable on TV. Establish module/widget foundation, render 7-item left menu matching user mockup, build Home + Calendar screens, add Saves overlay + New-Game wizard, expose `--tv` preset.

**Locked decisions** (from approved plan `~/.claude/plans/tui-tv-tui-phase-curried-pebble.md`):
1. CLI/REPL surface untouched. Cuts only at TUI menu level.
2. Rotation Level A (M21 scope, not this phase).
3. Calendar = 7×5 month grid.
4. Keyboard input only.

## Mockup target

```
┌─────────────────┐
│ Season 2026-27  │
├─────────────────┤
│ MENU            │
│ > Home          │
│   Roster        │  ← stub in M20, real in M21
│   Rotation      │  ← stub in M20, real in M21
│   Trades        │  ← stub in M20, real in M22
│   Draft         │  ← stub in M20, real in M22
│   Finance       │  ← stub in M20, real in M22
│   Calendar      │  ← REAL in M20
└─────────────────┘
```

## Workers (team `nba3k-m20`)

| Worker | Owns | Wave |
|------|------|------|
| `nba3k-m20-shell`    | `tui/mod.rs`, `tui/widgets.rs`, `tui/screens/menu.rs`, theme, action bar, `--tv` flag in `cli.rs`, screen stubs for Roster/Rotation/Trades/Draft/Finance | Wave 0 |
| `nba3k-m20-home`     | `tui/screens/home.rs`, `tui/screens/saves.rs`, `tui/screens/new_game.rs` | Wave 1 |
| `nba3k-m20-calendar` | `tui/screens/calendar.rs` + sub-tabs (Schedule/Standings/Playoffs/Awards/AllStar/Cup) | Wave 1 |

Wave 0 establishes module skeleton + widget API. Wave 1 spawns after Wave 0 confirms `cargo build` clean.

## Acceptance gates (orchestrator verifies)

1. `cargo build --workspace` clean (no new warnings beyond existing).
2. `cargo test --workspace` ≥ 275 passing (current baseline).
3. `nba3k --save fresh.db tui` works on save with valid data: 7 menu items render, `↑↓` navigates, `Enter` enters Home/Calendar, `Esc` returns.
4. `nba3k tui` (no `--save`) launches new-game wizard.
5. `Ctrl+S` opens saves overlay with list/delete.
6. Calendar grid: 7×5 cells, current week highlighted, user-team game cells show opponent abbrev, event days highlighted.
7. Calendar sim controls: `Space`, `W`, `M`, `Enter` on event row, `A` for season-advance all work.
8. `--tv` flag bumps padding + applies high-contrast theme.
9. `q` from menu prompts quit confirm; from inner screen returns to menu.

## Out of scope (deferred to M21/M22)

- Roster screen content (just stub "Coming in M21")
- Rotation backend + UI
- Trades / Draft / Finance interactive screens
- Mouse support
- Help overlay (`?` key) — punted to M22 polish

## Files NOT touched

- `commands.rs` (reuse `dispatch` as-is)
- `commands.rs:build_snapshot` (M21 only)
- `cli.rs` Command enum (only adds `--tv` to existing `tui` subcommand)
- All other crates (nba3k-core, nba3k-models, nba3k-sim, nba3k-trade, nba3k-season, nba3k-store, nba3k-scrape) — zero changes
