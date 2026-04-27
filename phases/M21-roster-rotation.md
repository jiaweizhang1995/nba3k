# M21 — Roster + Rotation (Level A)

**Status:** ✅ Done 2026-04-27. 3-worker team `nba3k-m21`, 2 waves.

## Outcome

- Worker `nba3k-m21-backend`: V014 migration `team_starters(team_id, pos, player_id)` with CHECK + PK, idx on player_id. `Starters` struct in `nba3k-core/src/rotation.rs` re-exported from `nba3k_core`. Store API (`read_starters`/`upsert_starter`/`clear_starter`/`clear_all_starters`) with `StoreError::InvalidInput` for bad pos. `build_snapshot` hook (`user_starters_or_auto` + `apply_user_starters`) at commands.rs:1310 — falls back silently when starters incomplete or any player off-roster. 6 round-trip tests.
- Worker `nba3k-m21-roster`: Real Roster screen replacing M20 stub (~1600 LoC). 2 sub-tabs (My Roster + Free Agents). Sort keys o/p/a/s. Row actions t/e/x/R/Enter. 4-panel Player Detail modal. thread_local cache + invalidate(). All mutations through `dispatch` wrapped in `with_silenced_io`.
- Worker `nba3k-m21-rotation`: Real Rotation Level-A screen (~530 LoC). 5 slot rows (PG/SG/SF/PF/C). Picker filtered by adjacency + OVR-desc sorted. `c` clear slot, `C` clear all. thread_local cache + invalidate(). Direct Store API writes (no Command dispatch needed). Also wired both Roster + Rotation routes in `tui/mod.rs` (388/389 dispatch + 605/606 render).

## Verification

- `cargo build --release --bin nba3k`: clean
- `cargo test --workspace`: **281 passed**, 1 ignored (67 suites) — +6 over M20 baseline
- V014 applies to fresh save: `team_starters` schema confirmed via sqlite3
- CLI cuts hold: `compare`, `hof`, `records --scope season --stat ppg` all work
- Worker pty smokes covered: roster sort + cut/sign + extend 2-step + role; rotation slot fill + clear flow

## Known limitations (Level A scope)

- No bench order / no minutes slider / no closing lineup (deferred to potential Level B/C)
- Rotation only affects user team's snapshot; AI teams stay 100% auto
- "Stale starter" (traded/retired) renders dimmed `(off roster)`; sim engine swallows silently


**Goal:** Replace M20 stubs for Roster + Rotation. Add Rotation Level A as new feature with backend support (DB migration + sim engine hook).

**Locked decisions** (from `~/.claude/plans/tui-tv-tui-phase-curried-pebble.md` and `phases/POST-M20-HANDOFF.md`):
1. Rotation = Level A only (5 starting positions, no bench/minutes/closing)
2. CLI/REPL untouched — TUI menu only
3. ratatui 0.29 + crossterm 0.28, no new deps

## Workers (team `nba3k-m21`)

| Worker | Owns | Wave |
|--------|------|------|
| `nba3k-m21-backend` | V014 migration + `nba3k-core::Starters` + Store API + `build_snapshot` hook | Wave 0 |
| `nba3k-m21-roster` | `tui/screens/roster.rs` (replaces M20 stub) | Wave 0 (parallel, different files) |
| `nba3k-m21-rotation` | `tui/screens/rotation.rs` (replaces M20 stub) | Wave 1 (blocked by backend) |

Backend + Roster screen run parallel in Wave 0 (no shared files). Rotation screen waits for backend's `Starters` API.

## Acceptance gates (orchestrator)

1. `cargo build --workspace` clean (no NEW warnings)
2. `cargo test --workspace` ≥ 278 (275 baseline + ≥3 rotation backend tests)
3. V014 migration applies cleanly to existing M20 saves
4. Smoke (TUI keyboard-only, on fresh save):
   - Menu → Roster → see roster sorted by OVR
   - Train action fires dispatch
   - FA tab → sign action works
   - Menu → Rotation → 5 empty slots
   - Pick a player for each → sim 1 game → box score reflects assignment
   - Clear all → sim 1 game → auto path resumes
5. CLI cuts hold: `compare`, `hof`, `records` still work
6. README TUI section updated

## Out of scope

- Bench order, minutes slider, closing lineup (Level B/C deferred)
- Wave 0 TUI shell file modifications (`tui/mod.rs`, `tui/widgets.rs`)
- Other crates beyond `nba3k-core` + `nba3k-store` + `nba3k-cli`
