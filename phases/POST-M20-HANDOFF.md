# Post-M20 Handoff — M21 + M22

This document is a self-contained handoff for any AI agent picking up after M20 ships. Read this in full before starting.

---

## 0. Project context (everything you need to know)

**Project**: nba3k — Rust CLI clone of NBA 2K MyGM mode. Personal / non-commercial.
**Repo**: https://github.com/jiaweizhang1995/nba3k.git (origin/main)
**Working directory**: `/Users/jimmymacmini/Desktop/claude-code-project/nba3k-claude`
**Build**:
```bash
PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --bin nba3k
PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo test --workspace
```

### Workspace layout (8 crates)

```
crates/
  nba3k-core/        Public types: Player, Team, Coach, LeagueYear, Snapshot
  nba3k-models/      7 explainable scoring models + progression + chemistry
  nba3k-sim/         Statistical-distribution game sim + 9-feature team quality
  nba3k-trade/       Trade engine: evaluate + CBA + GM personality + multi-round negotiation
  nba3k-season/      Schedule generation + playoffs + awards + HOF
  nba3k-store/       SQLite persistence + refinery migrations (V001-V013, +V014 in M21)
  nba3k-scrape/      Bootstrap data scraper + rating calibration
  nba3k-cli/         CLI parser + REPL + TUI + command dispatcher
```

### State of the project after M20

- M1-M19 complete (legacy CLI/REPL features all shipped)
- M19 had a read-only 5-tab TUI
- M20 ships: 7-menu TUI shell + widgets + Home + Calendar + Saves overlay + new-game wizard + `--tv` preset
- 275+ tests passing
- After M20 commit: README updated, phases/PHASES.md row added

### Locked design decisions (from approved plan `~/.claude/plans/tui-tv-tui-phase-curried-pebble.md`)

These are NOT negotiable. Do not re-litigate.

1. **Cut policy**: TUI menu cuts non-7 features. CLI/REPL surface stays untouched. The 7 menu items are: Home / Roster / Rotation / Trades / Draft / Finance / Calendar.
2. **Rotation depth**: Level A only. Starting 5 picker (PG/SG/SF/PF/C × 1). No bench order, no minutes slider.
3. **Calendar style**: 7×6 month grid (already shipped in M20).
4. **Input device**: keyboard only.

### Non-mutating invariants

- All state changes go through `commands::dispatch(app, Command) -> Result<()>` at `crates/nba3k-cli/src/commands.rs:40`. Do not bypass.
- Wrap any `dispatch(...)` call from TUI in `crate::tui::with_silenced_io(|| ...)` to suppress stdout/stderr from corrupting alt-screen.
- TUI files live under `crates/nba3k-cli/src/tui/` after M20 (mod.rs + widgets.rs + screens/*).
- Widget API contract: `FormWidget { fn render(&self, f, area, theme); fn handle_key(&mut self, key) -> WidgetEvent }` with `WidgetEvent = None | Submitted | Cancelled | Selected(usize) | Toggled(usize)`. Theme exposes `.text() / .highlight() / .accent_style() / .muted_style() / .block(title)`. ActionBar takes `&[(&str, &str)]` and supports `.with_status(&str)`.

### Agent team conventions (mandatory)

- One team per phase, named `nba3k-m{N}` (e.g. `nba3k-m21`).
- Workers own non-overlapping file paths. Coordinate via `SendMessage`.
- Orchestrator (lead session) creates team via `TeamCreate`, breaks work into tasks via `TaskCreate`, blocks Wave 1 by Wave 0 via `TaskUpdate addBlockedBy`, spawns workers via Agent tool with `team_name` + `name` + `run_in_background: true`.
- Workers use `subagent_type: general-purpose` so they can write code.
- Workers never modify another worker's files. If a Wave-0 contract gap blocks Wave 1, message Wave-0 worker to extend the contract.
- Each worker mark task `completed` via `TaskUpdate` only when build + tests pass + smoke verified.
- Orchestrator runs final acceptance review (own task) + makes the commit.
- Phase doc per phase: `phases/M{N}-{slug}.md` written by orchestrator before spawning workers.
- Per-phase Bash-verifiable smoke test required before completion.

### Memory layer (durable cross-session context)

`/Users/jimmymacmini/.claude/projects/-Users-jimmymacmini-Desktop-claude-code-project-nba3k-claude/memory/`

Read `MEMORY.md` first (always-loaded index). Most relevant file: `project_nba3k.md` — long-form project context, phase log, commands, locked decisions. Append a new section there after each completed milestone.

### Approved plan

`~/.claude/plans/tui-tv-tui-phase-curried-pebble.md` — high-level 3-phase plan (M20 + M21 + M22). M21 + M22 sections of that plan are the authoritative scope for this handoff.

---

## 1. M21 — Roster + Rotation (Level A)

**Goal**: Replace M20's stub Roster/Rotation screens with real interactive screens. Add Rotation Level A as a new feature with backend support (DB migration + sim engine hook).

**Estimated effort**: 1.5 days, 3 workers, 2 waves.

### Wave 0 (foundation, single worker)

#### Worker `nba3k-m21-rotation-backend`

Builds the Rotation Level A backend before any UI work.

**Owns:**
- `crates/nba3k-store/migrations/V014__rotation.sql` (new)
- `crates/nba3k-core/src/rotation.rs` (new) + `nba3k-core/src/lib.rs` re-export
- `crates/nba3k-store/src/store.rs` — add `read_starters(team_id) -> Result<Starters>` and `upsert_starter(team_id, pos, player_id)` and `clear_starter(team_id, pos)`
- `crates/nba3k-cli/src/commands.rs:build_snapshot` — single-function edit at line ~1078 to use user-set starters when complete

**Migration spec** (`V014__rotation.sql`):
```sql
CREATE TABLE team_starters (
    team_id   INTEGER NOT NULL,
    pos       TEXT    NOT NULL CHECK(pos IN ('PG','SG','SF','PF','C')),
    player_id INTEGER NOT NULL,
    PRIMARY KEY (team_id, pos),
    FOREIGN KEY (team_id) REFERENCES teams(id),
    FOREIGN KEY (player_id) REFERENCES players(id)
);
CREATE INDEX idx_team_starters_player ON team_starters(player_id);
```

**Rotation struct spec** (`crates/nba3k-core/src/rotation.rs`):
```rust
#[derive(Debug, Clone, Default)]
pub struct Starters {
    pub pg: Option<PlayerId>,
    pub sg: Option<PlayerId>,
    pub sf: Option<PlayerId>,
    pub pf: Option<PlayerId>,
    pub c:  Option<PlayerId>,
}

impl Starters {
    pub fn is_complete(&self) -> bool { /* all 5 set */ }
    pub fn slot(&self, pos: Position) -> Option<PlayerId> { ... }
    pub fn set_slot(&mut self, pos: Position, player: Option<PlayerId>) { ... }
}
```

**Sim integration** (`commands.rs:build_snapshot`):
- Read existing function (around line 1078). It currently builds a position-aware rotation by picking top-OVR players per position with adjacency fallback.
- New behavior: for each team, call `store.read_starters(team_id)`. If complete (all 5 slots set AND all 5 players are still on the roster AND not retired), use those 5 as the positional starters. Bench picks remain auto (next-best-by-position). Minutes split unchanged.
- If any slot empty OR player no longer on roster, fall through to existing auto-builder. No partial overrides.
- AI teams (29 of 30) rarely have starters set, so they go through auto path. Only user team's 5 typically set.

**Verification gates**:
- `cargo test --workspace` ≥ 275 + 3 new unit tests in `nba3k-store/tests/rotation.rs` (set / read / clear / non-existent team)
- Migration runs cleanly on a fresh save
- Migration is idempotent (existing saves don't break — V014 is purely additive, no data backfill needed)
- Smoke: open existing /tmp/m20.db save (M20-built), confirm V014 applies, no rows in team_starters, sim still works (auto path used).

#### Worker `nba3k-m21-roster-screen` (parallel with backend, no shared files)

**Owns:**
- `crates/nba3k-cli/src/tui/screens/roster.rs` — replace M20 stub with real screen
- (optional) `crates/nba3k-cli/src/commands.rs` — add small data-fetch helpers if needed (e.g., `pub fn read_roster_with_stats(app, team_id) -> Vec<RosterRow>`). Mark each `// M21 Worker B: data-fetch wrapper for TUI`.

**Roster screen spec**:
- Table layout: # / Name / Pos / Age / OVR / PPG / RPG / APG / Role / Cap%
- Sort keys: `o` OVR, `p` position, `a` age, `r` role (cycle on press)
- Row keys (selected row):
  - `Enter` — open Player Detail modal
  - `t` — Train (focus picker modal: shoot/inside/def/reb/ath/handle) → `Command::Training`
  - `e` — Extend modal (NumberInput salary $M + NumberInput years) → `Command::Extend`
  - `x` — Cut (Confirm modal) → `Command::FaCut`
  - `R` — Set role (Picker: star/starter/sixth/role/bench/prospect) → `Command::RosterSetRole`
- Tabs at top: `[My Roster] [Free Agents]`
  - FA tab: list FAs sortable by OVR. `s` to sign → `Command::FaSign` with confirm if cap-tight
- Detail modal:
  - Stats panel (current season averages + last 5 games)
  - Career panel (year-by-year)
  - Contract panel (year-by-year salary breakdown)
  - Chemistry panel (this player's contribution to team chem)
  - Action bar: `t`=train, `e`=extend, `x`=cut, `R`=role, `Esc`=back

#### Worker `nba3k-m21-rotation-screen` (Wave 1, blocked by Wave-0 rotation backend)

**Owns:**
- `crates/nba3k-cli/src/tui/screens/rotation.rs` — replace M20 stub

**Rotation screen spec**:
- Single-screen layout: 5 horizontal slot rows (PG / SG / SF / PF / C). Each row shows: position label + slot occupant (player name + OVR) or `[empty — auto-pick]`.
- Selected row highlighted via theme.
- Keys:
  - `↑ / ↓` navigate slots
  - `Enter` open player picker (filtered to user-team players whose `position == slot OR position adjacent to slot`; adjacency: PG↔SG, SG↔SF, SF↔PF, PF↔C — matches existing `build_snapshot` logic)
  - `c` clear selected slot back to auto
  - `C` clear all 5 slots
  - `Esc` back to menu
- Save is implicit on each slot change: TUI calls `store.upsert_starter` or `store.clear_starter` immediately, then refreshes display.
- Bottom action bar shows `↑↓ Navigate  Enter Pick Player  c Clear Slot  C Clear All  Esc Back`.
- After any change, render hint text "Rotation will apply on next sim. Bench + minutes remain auto."

### M21 acceptance gates (orchestrator)

- `cargo build --workspace` clean (no NEW warnings)
- `cargo test --workspace` ≥ 278 (275 + 3 rotation backend tests)
- `nba3k-scrape` re-run NOT required (V014 is additive)
- Smoke (TUI keyboard-only, on a fresh save):
  1. Menu → Roster → see roster sorted by OVR
  2. Select Tatum → `t` → Train shoot → confirm dispatch fires
  3. `Esc` → Roster list still there
  4. Select bench player → `x` → Confirm → roster shrinks by 1
  5. Tab to Free Agents → top FA → `s` → roster grows back
  6. Menu → Rotation → 5 empty slots
  7. `Enter` on PG → picker shows White, Pritchard, Holiday → select White
  8. Repeat for SG/SF/PF/C
  9. Menu → Calendar → sim 1 game → check box score: White logs PG minutes
  10. Menu → Rotation → `C` → clear all → sim 1 more game → auto path resumes
- README.md TUI section updated to mention Roster + Rotation
- Single commit "M21: Roster + Rotation Level A"

### M21 forbidden

- Do NOT add bench order, minutes slider, closing lineup. Level A only.
- Do NOT modify Wave 0 TUI shell files (`tui/mod.rs`, `tui/widgets.rs`).
- Do NOT touch other crates beyond what's listed.

---

## 2. M22 — Trades + Draft + Finance + cuts + polish

**Goal**: Replace M20 stubs for Trades / Draft / Finance with real interactive screens. Apply menu cuts. Final TV polish.

**Estimated effort**: 2 days, 4 workers, 2 waves (3 screen workers parallel + 1 polish worker after).

### Wave 0 (parallel screens)

#### Worker `nba3k-m22-trades-screen`

**Owns:**
- `crates/nba3k-cli/src/tui/screens/trades.rs`

**Spec**:
- 4 sub-tabs at top: `[Inbox] [My Proposals] [Builder] [Rumors]`
- Tab 1 Inbox (incoming AI offers):
  - Reuse `cmd_offers` data path
  - Row keys: `a` Accept (`Command::Trade(Respond { id, action: "accept" })`), `r` Reject, `c` Counter (sub-modal)
- Tab 2 My Proposals:
  - Reuse `cmd_trade_list` / `cmd_trade_chain` data
  - `Enter` show full chain
  - `a` / `r` / `c` respond if AI's turn
- Tab 3 Builder:
  - Mode toggle: 2-team or 3-team
  - 2-team: from team picker (defaults user) → to team picker → multi-select players each side → submit
  - On submit: `with_silenced_io(|| dispatch(app, Command::Trade(Propose { ... })))`
  - Verdict modal shows accept/reject reason + counter chain inline
- Tab 4 Rumors: list `cmd_rumors` data, no actions

#### Worker `nba3k-m22-draft-screen`

**Owns:**
- `crates/nba3k-cli/src/tui/screens/draft.rs`

**Spec**:
- Active only when `Phase::OffSeason` or end-of-Playoffs trigger (else show "Draft not active. Sim to end of season.")
- Layout: top-60 prospect board table — Name / Pos / Age / OVR (or `???` if not scouted) / scouted-ratings if scouted / projected pick
- Tab toggle: `[Board] [Order]`
- Keys:
  - `s` on selected prospect → `Command::Scout` (one of 5/season; counter shown)
  - `Enter` on user's pick turn → `Command::Draft(Pick)` confirm modal
  - `A` Auto-pick rest of round
- Show user team's next pick prominently with countdown ("Your pick: #14")

#### Worker `nba3k-m22-finance-screen`

**Owns:**
- `crates/nba3k-cli/src/tui/screens/finance.rs`

**Spec**:
- Single screen (no sub-tabs)
- Top: 4-line cap summary — Salary Cap / Luxury Tax / First Apron / Second Apron / Governors Line — with payroll bar visualization
- Body: roster contracts list — Player / Pos / Age / Y1 / Y2 / Y3 / Y4 / Total / Notes (kicker / NTC / option)
- Sort: by total $, by years remaining, by name
- Selected row keys:
  - `e` Extend (jumps to same modal as Roster screen, shared widget) → `Command::Extend`
- Bottom: tax/apron implications text ("X over second apron — hard cap implications: ...")

### Wave 1 (single polish worker, after Wave 0 all done)

#### Worker `nba3k-m22-polish`

**Owns:**
- `crates/nba3k-cli/src/tui/mod.rs` — small touches:
  - Help overlay `?` key — context-aware key list per screen
  - Optional mouse click (crossterm `EnableMouseCapture` + click-to-select on table rows)
  - Menu cuts (already in M20 — verify nothing leaked back)
  - Final E2E test
- `phases/M22-trade-draft-finance.md` — write the phase doc as the work happens
- `README.md` — update TUI section: list 7 menu items, mention --tv, --legacy, mouse, help overlay

**Spec**:
- Help overlay: full-screen modal triggered by `?`. Renders a 2-column key reference for the current screen. Pull data from each screen's `key_help() -> Vec<(&str, &str)>` method (added by Wave-0 workers).
- Mouse: optional, only if not invasive. Click-to-select on tables; click on tab to switch tab; click on action-bar key chips to invoke.
- Verify all M20 stub cuts hold: from TUI menu, no path to compare/records/hof/coach/all-star/cup standalone. (Some are accessible via Calendar sub-tabs — that's fine.)
- E2E smoke: complete one full season cycle from Menu only — see "End-to-end verification" below.

### M22 acceptance gates (orchestrator)

- `cargo build --workspace` clean
- `cargo test --workspace` ≥ 278 passing
- E2E (TUI keyboard-only):
  1. New game → BOS / Standard / 2026 / seed 42
  2. Menu → Home → see mandate
  3. Menu → Rotation → set 5 starters
  4. Menu → Finance → check payroll under cap
  5. Menu → Calendar → sim to All-Star
  6. Menu → Trades → Builder → propose Brown→KAT to MIN, accept counter
  7. Menu → Calendar → sim to season-end
  8. Menu → Draft → pick Cooper Flagg
  9. Menu → Roster → FA tab → sign top FA
  10. Menu → Calendar → season-advance → see Season 2027-28 in sidebar
- All 10 steps complete with no CLI/REPL drop-out
- README.md TUI section finalized
- Single commit "M22: Trades + Draft + Finance + polish — M20-22 ships"

### M22 forbidden

- Do NOT add features beyond the 7 menu items
- Do NOT modify Wave 0 worker files from M20/M21 except `tui/mod.rs` (only Wave 1 polish worker may touch it)
- Do NOT touch other crates

---

## 3. After M22

After M22 ships, the user can play one full season cycle from TUI on TV. Ship to GitHub:

```bash
git push origin main
```

Then update memory:

1. Append a section to `/Users/jimmymacmini/.claude/projects/.../memory/project_nba3k.md` titled "M20-M22 done (DATE)" describing what shipped.
2. Update `MEMORY.md` index if the entry name changed.

Future phases (NOT M21/M22 scope, just for awareness):
- M23+ candidates from `~/.claude/plans/tui-tv-tui-phase-curried-pebble.md` "Open assumptions": gamepad input, possession-by-possession sim, watch-game UI, RFA/Bird-rights flow.
- These were explicitly punted from M20-M22 and are NOT what this handoff covers.

---

## 4. Quick reference for the receiving AI

**To start M21:**

1. Read this doc end-to-end.
2. Read `~/.claude/plans/tui-tv-tui-phase-curried-pebble.md` (approved plan).
3. Read memory: `~/.claude/projects/-Users-jimmymacmini-Desktop-claude-code-project-nba3k-claude/memory/MEMORY.md` then `project_nba3k.md`.
4. Read `phases/PHASES.md` to confirm M20 completed.
5. Read `phases/M20-tui-shell.md` to understand the TUI shell + widget contract.
6. Read `crates/nba3k-cli/src/tui/mod.rs` and `crates/nba3k-cli/src/tui/widgets.rs` (you'll plug screens into them).
7. Read `crates/nba3k-cli/src/commands.rs:dispatch` (line 40) — single mutation entry point.
8. Skim `crates/nba3k-cli/src/cli.rs` for the `Command` enum + sub-action enums (`TradeAction`, `DraftAction`, `SavesAction`, etc.).
9. Write `phases/M21-roster-rotation.md` (orchestrator's planning doc).
10. `TeamCreate team_name=nba3k-m21`.
11. `TaskCreate` for each worker. Block Wave 1 by Wave 0.
12. Spawn workers via Agent tool with `team_name`, `name`, `run_in_background: true`.
13. Wait for completion notifications. Verify each worker's output.
14. Final acceptance review + commit.

**To start M22**: same flow, after M21 ships.

**Key gotchas**:
- `cargo test` requires PATH to include `/opt/homebrew/opt/rustup/bin`.
- Saves directory `data/seed_2025_26.sqlite` is `.gitignore`d — do not commit. Re-generate via `cargo run -p nba3k-scrape`.
- `.claude/` and `playtest/` are gitignored — agent worktrees go there, do not stage.
- SQLite saves auto-create `*.db-shm` and `*.db-wal` — clean up via `rm save.db save.db-shm save.db-wal`.
- Never use `git commit --amend` after a hook failure — create a NEW commit instead.
- TUI alt-screen mode means `eprintln!` from `dispatch` corrupts the screen — always wrap `dispatch` in `with_silenced_io`.

**Conventions**:
- Commit messages: short subject, body explains the why, ends with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` line.
- Phase docs live under `phases/M{N}-{slug}.md`.
- Phase tracker `phases/PHASES.md` gets a new row per completed phase.
- Memory file paths use absolute paths.

Good luck. Don't break the CLI.
