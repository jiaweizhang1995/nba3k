# todo-plan.md — M24 backlog (codex CLI execution doc)

**Maintainer**: main agent (Claude). Codex picks tasks, flips status, asks main agent if scope shifts.

**Working dir**: `/Users/jimmymacmini/Desktop/claude-code-project/nba3k-claude`
**Build**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --workspace`
**Test**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo test --workspace`
**Release**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --release --bin nba3k`
**Run**: `./target/debug/nba3k tui` / `./target/release/nba3k tui`

**Phase tracker**: `phases/PHASES.md` (8-menu invariant — this milestone bumps it to 9-menu, see below)

**Status legend**: `[ ]` not started · `[~]` in progress · `[x]` done · `[!]` blocked (write reason)

**Locked invariants** (do not break):
- TUI mutations always route through `crate::commands::dispatch(app, Command)` wrapped in `crate::tui::with_silenced_io(|| ...)`.
- CLI/REPL command surface stays untouched. CLI `sim-to` still accepts all old targets (`all-star`, `cup-final`, `season-end`, `playoffs`); only the TUI surface is reduced.
- Player names + team abbreviations stay English (data, not chrome).
- Tests must pass before marking task done. Baseline: 293 unit + 1 integ.
- i18n: every new UI string goes through `t(tui.lang, T::...)`. Add new T keys when needed (this milestone is allowed to grow the enum) — keep `i18n.rs` + `i18n_en.rs` + `i18n_zh.rs` in sync.

---

## Goal

User wants:

1. Sim controls move out of Calendar onto a **top banner that shows on every screen**, so user can sim from anywhere and see live changes (Roster stats update mid-screen, etc.).
2. Drop "sim to event" multi-target picker; only **Trade Deadline** remains as the event target on the banner.
3. **Calendar screen kept** as view-only (month grid + sub-tabs) — sim keys removed from it.
4. **Home dashboard rewritten** to look like the screenshot the user sent (`Seattle SuperSonics Dashboard` style):
   - Top: big team record + conference rank ("32-18 · 5th in conference").
   - Left column: full conference standings (15 teams, user team row highlighted, GB column).
   - Center column: Team Leaders (top-1 PPG / RPG / APG on user team) + League Leaders (top-1 PPG / RPG / APG league-wide).
   - Right column: Team Stats (Points / Allowed / Rebounds / Assists, each with `(Nth)` league rank) + Finances (Avg Attendance / Revenue YTD / Profit YTD / Cash / Payroll / Salary Cap).
   - Bottom strip (replaces "Recent News"): Starting Lineup table — 5 starters with PPG / RPG / APG / MIN.
   - Mandate panel + Inbox panel removed from Home.
5. **Mandate deleted** (V013 stays as a no-op orphan migration; no reads, no writes, no UI).
6. **Inbox** becomes a new menu item between Finance (#6) and Calendar (#8). Menu becomes 9 items:
   - 1 Home / 2 Roster / 3 Rotation / 4 Trades / 5 Draft / 6 Finance / 7 **Inbox** / 8 Calendar / 9 Settings.

---

## T5 — Top sim banner (global sim controls)

**Status**: `[x]`

→ codex: done — this commit — 290 unit tests passed; global Ctrl+D/W/N/T/A sim banner and invalidate_all_screens wired.

**Goal**: A persistent top banner across the entire TUI shell that shows current date / season / phase + clickable-style sim buttons. Hotkeys work in every screen including Calendar / Roster / Trades.

**Spec**:

- New top region above the body. Layout becomes:
  ```
  ┌── banner (3 lines) ─────────────────────────────────────────┐
  │ Sidebar (30 cols)  │  Content                               │
  │                    │                                        │
  ├── action bar (3) ──┴────────────────────────────────────────┤
  ```
  Body height = total - banner(3) - actionbar(3).
- Banner content (left → right):
  - Date / Season / Day-N (e.g., `Season 2026-27 · Day 41 · All-Star Break`).
  - Sim buttons row 2: `[Day]  [Week]  [Month]  [Trade Deadline]  [Season Advance]`. Localize labels via `T::SimDay` / `T::SimWeek` / `T::SimMonth` / `T::SimTradeDeadline` / `T::SimSeasonAdvance` (add new keys).
  - Each button shows its hotkey in `[H]` style: `[D] Day`, `[W] Week`, `[N] Month`, `[T] Trade Deadline`, `[A] Season Advance`. Use Ctrl-modified hotkeys to avoid collisions with existing screen keys.

- **Hotkeys (global, work in every screen except modal-active state)**:
  - `Ctrl+D` → `Command::Sim` 1 day
  - `Ctrl+W` → sim 7 days (reuse `cmd_sim_week` path)
  - `Ctrl+N` → sim 30 days (reuse `cmd_sim_month`; chose `N` because `M` collides with month picker / mode picker)
  - `Ctrl+T` → sim to trade deadline (use existing `sim-to trade-deadline` if it exists; otherwise add the target — see detail below)
  - `Ctrl+A` → season advance (existing `cmd_season_advance`)
  - All routed via `with_silenced_io(|| commands::dispatch(app, ...))`. Failures → `tui.last_msg = Some(error)`.

- **Trade-deadline target**: Check whether `Command::SimTo(SimToTarget::TradeDeadline)` already exists in `crates/nba3k-cli/src/cli.rs`. If not, add it — the schedule already encodes the deadline day in `LeagueYear::trade_deadline_day` or similar. Wire it in `commands::dispatch`. CLI / REPL acquires the new `--to trade-deadline` token.

- **Cache invalidation after every sim**: Banner sim handlers must call `tui.invalidate_caches()` (existing global cache flush) **plus** call `invalidate()` on every screen module that has caching (`screens::home::invalidate()`, `screens::roster::invalidate()`, `screens::rotation::invalidate()`, `screens::trades::invalidate()`, `screens::draft::invalidate()`, `screens::finance::invalidate()`, `screens::calendar::invalidate()`, `screens::inbox::invalidate()` once T7 lands). Add an `invalidate_all_screens()` helper in `tui/mod.rs`.

- **Banner suppressed in**: NewGame wizard, Launch screen, Settings, Saves overlay, QuitConfirm. Active-modal state (Confirm widgets) does not need to disable banner — but hotkeys do not fire while a modal is active (existing modal logic).

- **Banner key suppression on screens that own those modifier combos**: Currently no screen uses Ctrl+letter, so Ctrl-prefix keys are safe. Verify by `grep KeyModifiers::CONTROL` across screens.

**Files**:
- `crates/nba3k-cli/src/tui/mod.rs` — add banner draw + global hotkey dispatch.
- `crates/nba3k-cli/src/tui/widgets.rs` — optional `Banner` helper widget if it cleans up the layout.
- `crates/nba3k-cli/src/cli.rs` — add `SimToTarget::TradeDeadline` if missing; ensure CLI flag accepts it.
- `crates/nba3k-cli/src/commands.rs` — wire trade-deadline target if not present.
- `crates/nba3k-core/src/i18n.rs` + `i18n_en.rs` + `i18n_zh.rs` — add 5 new keys (sim buttons) + banner labels.

**Acceptance**:
- Banner visible on Home / Roster / Rotation / Trades / Draft / Finance / Inbox / Calendar / Settings.
- Banner hidden on Launch / NewGame / Saves / QuitConfirm (these screens own the full content area).
- `Ctrl+D` sims 1 day from any of the 9 menu screens; on screen visible to user, the data refreshes (e.g., team record updates on Home, player stats update on Roster).
- `Ctrl+T` advances directly to trade-deadline day.
- Original Calendar `Sim 1 Day` / `Sim Week` / `Sim Month` / `Sim to Event` / `Season Advance` keys removed; Calendar action bar reflects the cut (only `↑↓ ← → / Tab tabs / Esc back`).

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 293.
- Manual smoke: launch TUI → load save → from Roster screen press `Ctrl+W` → top banner date jumps 7 days → roster ppg column updates → Esc back to Menu, switch to Home → record reflects sim wins/losses.

---

## T6 — Home dashboard rewrite (per screenshot)

**Status**: `[x]`

→ codex: done — this commit — 290 unit tests passed; Home old inbox/news/next-game grep clean.

**Goal**: Replace current Home (mandate / next-game / inbox / recent news) with the multi-pane scoreboard layout from the screenshot.

**Layout** (in body area below banner):

```
┌──────────────────────┬──────────────────────────────────────────────────────┐
│                      │   ┌─── Header (3 lines, centered) ──────────────┐    │
│                      │   │  32-18                                       │    │
│                      │   │  5th in conference                            │   │
│                      │   └──────────────────────────────────────────────┘    │
│  Conference          ├─────────────────────┬────────────────────────────┐    │
│  Standings           │  Team Leaders        │  Team Stats               │    │
│  (15 rows)           │  • PPG: Tatum 28.4   │  Points     115.9 (3rd)   │    │
│  rank|team|GB        │  • RPG: KP   8.7     │  Allowed    110.4 (23rd)  │    │
│  user row highlighted│  • APG: White 6.1    │  Rebounds   42.9 (32nd)   │    │
│                      │                      │  Assists    24.9 (16th)   │    │
│                      │  League Leaders      │                            │    │
│                      │  • PPG: SGA 32.1 OKC │  Finances                  │    │
│                      │  • RPG: Sengun 12.4  │  Avg Attendance: 16,469    │    │
│                      │  • APG: Halib 11.0   │  Revenue (YTD): $286.12M   │    │
│                      │                      │  Profit (YTD):  $477k      │    │
│                      │                      │  Cash:          $165.58M   │    │
│                      │                      │  Payroll:       $258.49M   │    │
│                      │                      │  Salary Cap:    $266.15M   │    │
├──────────────────────┴──────────────────────┴────────────────────────────────┤
│  Starting Lineup                                                             │
│  PG White        18.4 ppg  3.2 rpg  6.1 apg  31.5 min                        │
│  SG Brown        24.7 ppg  5.5 rpg  4.0 apg  34.2 min                        │
│  SF Tatum        28.4 ppg  9.1 rpg  5.5 apg  37.0 min                        │
│  PF Porzingis    19.3 ppg  8.7 rpg  2.0 apg  29.8 min                        │
│  C  Horford       9.5 ppg  6.5 rpg  3.0 apg  28.0 min                        │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Data sources**:
- Team record + conference rank: `Store::read_standings()` filtered by `team.conference`. User team finds itself in the sorted list. GB = `(leader.wins - leader.losses - team.wins + team.losses) / 2.0`.
- Conference standings: same data, full single conference (15 teams), sorted by `wins desc, losses asc, win_pct`. Highlight user row.
- Team Leaders: top-1 by per-game PPG / RPG / APG on the user team's roster from `Store::career_stats_for_player` or per-season stats.
- League Leaders: same metrics across all 30 teams' rostered players.
- Team Stats with rank: average team PPG / opp PPG / RPG / APG across all played games this season; rank = position when 30 teams sorted by that stat.
- Finances: 6 metrics. `Avg Attendance` is hardcoded plausible default if backend has none (`16,000-19,000` random per team, deterministic by team_id seed). `Revenue (YTD)` / `Profit (YTD)` / `Cash` are derived if M12 finance system has them; otherwise plausible defaults computed from payroll (e.g., `revenue ≈ payroll × 1.3`, `profit ≈ revenue - payroll - 18M operating cost`). `Payroll` + `Salary Cap` come from existing `team_salary` + `LeagueYear`.
- Starting Lineup: `Store::read_starters(user_team)` (M21). For each filled slot, look up player + per-season averages. Empty slot → "—".

**Files**:
- `crates/nba3k-cli/src/tui/screens/home.rs` — full rewrite.
- `crates/nba3k-cli/src/commands.rs` — add small read-side helpers if needed (e.g., `pub fn read_league_leader(app, metric: Metric) -> Option<(Player, f32)>`). Mark each helper with `// M24 Home`.
- `crates/nba3k-store/src/store.rs` — add `read_team_avg_stats(team_id, season) -> TeamAvgStats { ppg, oppg, rpg, apg }` if not already present. Add `read_league_leaders(season, metric) -> Option<(PlayerId, f32)>`.
- `crates/nba3k-core/src/i18n.rs` + tables — add `T::HomeRecord`, `T::HomeConferenceRank`, `T::HomeConferenceStandings`, `T::HomeTeamLeaders`, `T::HomeLeagueLeaders`, `T::HomeTeamStats`, `T::HomeFinances`, `T::HomeStartingLineup`, `T::FinanceAvgAttendance`, `T::FinanceRevenueYTD`, `T::FinanceProfitYTD`, `T::FinanceCash`, plus the existing `T::FinancePayroll` / `T::FinanceCap` reused.
- Drop `T::HomeOwnerMandate`, `T::HomeNextGame`, `T::HomeGmInbox`, `T::HomeRecentNews`, `T::HomeNoMandate`, `T::HomeNoGoals`, `T::HomeNoUpcomingGames`, `T::HomeNoAlerts`, `T::HomeNoNews` (no longer rendered, OK to remove from enum + tables; keep build green).

**Acceptance**:
- New Home renders all 4 panes (header / standings / leaders+stats / finances / starting-lineup-bottom).
- Conference standings highlights user row in `theme.highlight()`.
- After `Ctrl+D` (T5) sims a day, all numbers refresh on next render — record, leaders, ranks.
- Empty starter slots show `—`.
- League leaders row shows the league-wide top-1 for each metric (could be a non-user player, marked with team abbrev e.g. `SGA OKC 32.1`).

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 293.
- Manual smoke: new game BOS → Home → see "0-0 · 1st in conference" before any sim → Ctrl+D 5 times → record updates, conference standings reorder, lineup ppg moves off 0.

---

## T7 — Inbox menu (between Finance and Calendar)

**Status**: `[x]`

→ codex: done — ba565e9 — 290 unit tests passed; sidebar menu 9 items with Inbox at #7.

**Goal**: New 7th menu item showing GM messages / trade demands / news. Existing Home Inbox panel data moves here.

**Menu order (9 items)**: Home / Roster / Rotation / Trades / Draft / Finance / **Inbox** / Calendar / Settings. Number-key range becomes `1`-`9`.

**Spec**:
- New `crates/nba3k-cli/src/tui/screens/inbox.rs` following M21+M22 screen pattern (`render` + `handle_key` + thread-local cache + `invalidate()`).
- Tabs: `[Messages] [Trade Demands] [News]`.
  - Messages: `Store::list_messages(user_team)` — same source as `cmd_messages`. Each row: date / subject / preview.
  - Trade Demands: `Store::list_trade_demands(user_team)` if exists, else read from `messages` filtered by `kind == TradeDemand`.
  - News: `Store::list_news(limit=50)` (M13 V008).
- Selected row → modal with full message body. Esc closes modal.
- Action bar: `↑↓ Move · Tab Tabs · Enter Detail · Esc Back`.

**Files**:
- New: `crates/nba3k-cli/src/tui/screens/inbox.rs`.
- Edit: `crates/nba3k-cli/src/tui/screens/mod.rs` — `pub mod inbox;`.
- Edit: `crates/nba3k-cli/src/tui/mod.rs`:
  - `enum MenuItem` — insert `Inbox` between `Finance` and `Calendar`.
  - `MenuItem::ALL: [_; 9]` — 9 items in correct order.
  - `MenuItem::label` / `screen` arms.
  - `enum Screen` — add `Inbox`.
  - `draw_content` arm + `inner_screen_key` arm for Inbox.
  - Hotkey range `'1'..='9'` (was `'1'..='8'`).
- Edit: `crates/nba3k-core/src/i18n.rs` + tables — add `T::MenuInbox`, `T::InboxTitle`, `T::InboxMessages`, `T::InboxTradeDemands`, `T::InboxNews`, `T::InboxNoMessages`, `T::InboxNoDemands`, `T::InboxNoNews`. Reuse existing `T::CommonDetail` / `T::CommonBack` / `T::CommonTabs`.

**Acceptance**:
- Sidebar menu shows 9 items. Number-key shortcut `7` jumps to Inbox.
- Inbox 3 tabs render with real data from store.
- Sim from banner (T5) eventually generates new messages → Inbox row count grows.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 293.
- Manual: sim a month → check Inbox shows trade demand / news rows.

---

## T8 — Delete mandate

**Status**: `[x]`

→ codex: done — 01f62cf — 290 unit tests passed; mandate grep 0 non-migration hits.

**Goal**: Remove all mandate generation, reads, writes, and UI surface. Leave V013 migration in place (so old saves don't break) but no code path touches the table.

**Files** (delete or stub):
- `crates/nba3k-store/src/store.rs` — keep `read_mandate` / `write_mandate` methods if they exist to avoid breaking old saves on read; mark `#[allow(dead_code)]`. Remove all callers.
- `crates/nba3k-cli/src/commands.rs` — drop mandate generation from `cmd_season_advance` / `cmd_new`. Drop `cmd_mandate` if it's a CLI subcommand (decide: kill the subcommand entirely or have it print "mandate system removed"). Either is fine; killing it is cleaner — CLI invariant is "command surface stays intact" but mandate was an addition, so kill is approved.
- `crates/nba3k-cli/src/cli.rs` — drop `Command::Mandate` variant if present.
- Anywhere else mandate is referenced (`grep -rn 'mandate\|Mandate' crates/`).
- Old `T::Home*Mandate*` keys are dropped in T6.

**Acceptance**:
- `grep -rn -i mandate crates/ | grep -v migrations | grep -v test_` returns ≤ 1 hit (the V013 migration file is allowed; comments referring to "mandate" elsewhere should be cleaned up).
- Tests still pass.
- `cmd_season_advance` no longer rolls a mandate.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 293 (drop any mandate-specific tests; subtract from baseline if needed and report new count).

---

## T9 — Calendar screen view-only

**Status**: `[x]`

→ codex: done — this commit — 290 unit tests passed; Calendar local sim/action-bar grep clean.

**Goal**: Remove sim keys from Calendar; everything else stays.

**Files**:
- `crates/nba3k-cli/src/tui/screens/calendar.rs`:
  - Remove `Sim 1 Day` / `Sim Week` / `Sim Month` / `Sim to Event` / `Season Advance` keybindings + action bar entries.
  - Keep month grid, sub-tabs (Schedule / Standings / Playoffs / Awards / All-Star / Cup), navigation keys (`↑↓←→` / `Tab` / `Esc`).
- `crates/nba3k-core/src/i18n.rs` — keep existing calendar keys; remove unused sim-related ones if exclusively used by Calendar (`T::CalendarSimDay`, `T::CalendarSimWeek`, `T::CalendarSimMonth`, `T::CalendarSimToEvent`, `T::CalendarSeasonAdvance`). T5 reuses analogous new keys (`T::SimDay`, etc.) for the banner — don't double-localize.

**Acceptance**:
- Calendar action bar shows navigation keys only; no sim keys.
- Sim still works while Calendar is the visible screen — via banner hotkeys (`Ctrl+D` etc., from T5). Calendar refreshes to show updated date.

---

## Coordination protocol

- Codex: pick a task, flip `[ ]` → `[~]`, leave note `→ codex: <one-line plan>`. When done, flip `[x]` and leave `→ codex: done — <commit hash> — <test count> passed; <key grep stat>`.
- Wave 1: T8 (mandate delete) is independent, do first or last — does not touch UI.
- Wave 2: T7 (Inbox menu) before T6 (Home rewrite) so Home doesn't try to re-create an inbox panel that's about to move.
- Wave 3: T5 (banner) parallel-safe with T6 / T7 — but they'll merge-conflict on `tui/mod.rs`. Do T7 → T5 → T6 sequentially, or use one worker for all three.
- Wave 4: T9 (Calendar trim) last — it depends on T5's banner being in place so sim doesn't get accidentally orphaned.
- Commit per task: `M24-T<N>: <one-line summary>`. Co-authored line preserved.
- Blocked: write `→ codex: blocked — <reason>` and stop. Main agent revises.
- Add new T enum keys freely this milestone (i18n.rs + i18n_en + i18n_zh in lockstep). Player names and team abbreviations stay English.

## Resolved decisions (2026-04-28)

- T5 sim location: top banner; hotkeys are Ctrl-prefixed (`Ctrl+D/W/N/T/A`).
- T5 sim event target: only Trade Deadline (`Ctrl+T`). Other targets remain in CLI but are dropped from the TUI banner.
- T6 conference standings: single conference, 15 rows.
- T6 starting lineup placement: bottom strip, replaces "Recent News".
- T6 leaders: top-1 only (PPG / RPG / APG) for both Team and League panels.
- T7 inbox menu position: 7th, between Finance (#6) and Calendar (#8). Total menu = 9.
- T8 mandate: delete code paths; V013 migration stays as orphan.
- T9 calendar: view-only, kept on the menu.

---

# M25 — UX polish backlog (post-M24 user feedback)

User-reported items 2026-04-28 after running release build. Same protocol as above (status legend / commit format / new i18n keys allowed when tables stay in sync).

## T10 — Default starting lineup + rotation row alignment

**Status**: `[x]`

→ codex: done — this commit — 291 unit tests passed; new saves/default loads populate five starters and Rotation rows align.

**Goal a**: New saves should start with a populated default starting 5 (top-OVR by adjacency-aware position fit, same algorithm as the existing auto-builder in `build_snapshot`). User then edits. No more all-empty rotation screen on first open.

**Goal b**: Rotation screen layout bug — when a slot is empty, the row text `> C  [empty — auto-pick]  press Enter to choose` does NOT align with filled rows like `PF  Miles Bridges (84 OVR)  press Enter to change, c to clear`. Position label width inconsistent (single-letter `C` vs two-letter `PG`/`SG`/`SF`/`PF`), and the hint column slides left.

**Files**:
- `crates/nba3k-cli/src/commands.rs` — `cmd_new(...)` (creates fresh save). After season_state + roster persist, call new helper `populate_default_starters(store, user_team) -> Result<()>` that:
  1. Reads user team roster.
  2. Picks 5 starters using the existing `build_snapshot` adjacency-aware top-OVR algorithm (extract that picker into a shared function in `nba3k-core::rotation` or `nba3k-cli::commands`).
  3. Writes 5 rows into `team_starters` via `Store::upsert_starter`.
- Same helper invoked at `Store::open` time on saves where `team_starters` is empty for the user team — guards against pre-existing saves missing defaults. Cheap idempotent check.
- `crates/nba3k-cli/src/tui/screens/rotation.rs` — fix row formatter:
  - Pad position label to 2 columns: `format!("{:<2}", pos.label())` so `C ` matches `PG`.
  - Fixed-width name column (e.g. 24 cols), then fixed-width hint suffix (`>= 32 cols`).
  - Selected-row prefix `> ` already 2 cols, ensure unselected is `  ` 2 cols (no shift).
  - Empty-slot text should still respect the same column widths so `[empty — auto-pick]` aligns with `Miles Bridges (84 OVR)`.

**Acceptance**:
- New save → Menu → Rotation → 5 slots already filled with sensible top-OVR starters.
- All 5 rows align column-for-column whether filled or empty.
- User can `c` to clear a slot, picker re-opens, fill restored.
- Existing saves (if any) get auto-populated defaults the first time they're loaded post-fix.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ baseline (currently 289). Add 1 test in `nba3k-store/tests/rotation.rs` confirming `populate_default_starters` writes 5 rows for a team with ≥5 players.
- Manual smoke screenshot vs current image #10 shows aligned rows.

---

## T11 — Focus border indicator (sidebar vs content active region)

**Status**: `[x]`

→ codex: done — this commit — 293 unit tests passed; shell focus zone drives yellow sidebar/content outer borders.

**Goal**: User can't tell whether they're focused on the sidebar menu or the content pane. Add a yellow accent border around the **active** zone (whichever side currently consumes input). Mirror the existing yellow-border pattern used by the Calendar selected-day cell (image #12).

**Spec**:

- New enum on `TuiApp`: `pub focus: FocusZone { Sidebar, Content }`.
- Rules:
  - `Screen::Menu` → focus = Sidebar.
  - Any inner screen with `tui.preview_mode == true` → focus = Sidebar (user is hovering menu items, content is preview).
  - Any inner screen with `tui.preview_mode == false` → focus = Content (Enter/Tab focused).
  - Launch / NewGame / Saves / QuitConfirm / Settings → focus = Content (full-area screens).
- Renderer: introduce `Theme::focus_block(title, active: bool) -> Block` helper. When `active`, border style = `theme.accent_style()` (yellow). When inactive, border style = `theme.muted_style()` (default gray).
- Apply at the **outer container** of each region:
  - Sidebar (season banner + menu, currently uses `theme.block(...)`) wraps in `theme.focus_block(title, focus == FocusZone::Sidebar)`.
  - Content area: each screen's outermost block uses `theme.focus_block(title, focus == FocusZone::Content)`.
- Internal sub-panels (Home's standings / leaders / finances / etc.) keep their default block style — only the outer frame switches.

**Files**:
- `crates/nba3k-cli/src/tui/widgets.rs` — `Theme::focus_block` helper.
- `crates/nba3k-cli/src/tui/mod.rs` — derive `tui.focus` from current state in `draw()`. Pass into `draw_sidebar` and `draw_content`. Each screen render fn signature optionally accepts `focused: bool` (read from `tui.focus` at the call site).
- Each `screens/*.rs` outer block call switches to `theme.focus_block(title, focused)`. Default to `true` for full-area screens.

**Acceptance**:
- On Menu / preview mode → sidebar has yellow border, content gray.
- On focused inner screen (post Enter/Tab) → content has yellow border, sidebar gray.
- Esc from inner screen → sidebar border becomes yellow again (preview_mode resumed).
- Calendar's existing selected-day yellow cell still works (T11 doesn't regress that).

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 289.
- Manual screenshot smoke: launch save → arrow nav (yellow on sidebar) → Enter (yellow flips to content) → Esc (yellow flips back).

---

## T12 — Finance text contrast + cap-line overflow

**Status**: `[x]`

→ codex: done — this commit — 290 unit tests passed; Finance Gauge label shortened and cap text wraps.

**Goal a**: Finance screen has a band where light cream text sits on dark blue background — barely readable (image #11). Looks like the highlight style is being applied to the cap summary band where it shouldn't be, or the highlight bg + fg combo is too close in luminance.

**Goal b**: The cap implication line `$171.11M / $207.82M 硬帽线 (` cuts off at the open paren — content overflows the panel width. Need either wrap, truncate-with-ellipsis, or shorter copy.

**Spec**:

- Inspect `crates/nba3k-cli/src/tui/screens/finance.rs` for any `.style(theme.highlight())` on summary lines that aren't selectable rows. If the cap summary is using highlight, switch to `theme.accent_style()` (yellow on default bg) or `theme.text()` for normal contrast.
- For the overflow: wrap the cap line in `Paragraph::new(...).wrap(Wrap { trim: false })` so it spans 2 lines if width is tight. Or split the metric into two lines deliberately — line 1 `$171.11M / $207.82M` and line 2 `硬帽线 (剩余 $36M)` etc.
- Theme audit: verify `Theme::DEFAULT` and `Theme::TV` highlight combos are legible. If `bg=DarkGray, fg=Yellow` produces the screenshot's near-illegible state, swap to `bg=Black, fg=Yellow` or `bg=Yellow, fg=Black` for genuine contrast — but this affects every screen using highlight. Prefer the per-screen fix unless the global combo is genuinely broken.
- Inbox / Roster / Trades selected-row highlight should still look clearly distinct after any global theme tweak.

**Files**:
- `crates/nba3k-cli/src/tui/screens/finance.rs` — switch styles + add wrap to cap summary line.
- `crates/nba3k-cli/src/tui/widgets.rs` only if global highlight needs adjustment.

**Acceptance**:
- Finance cap summary is readable at default + TV themes.
- The 硬帽线 / 软帽线 / 第一档 / 第二档 lines fit fully within the panel; if truncation is unavoidable, the truncation is intentional and ends with `…`.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 289.
- Manual screenshot smoke vs image #11.

---

## T13 — Home header shows team identity (abbrev + full name)

**Status**: `[x]`

→ codex: done — this commit — 290 unit tests passed; Home header now shows abbrev + full team name.

**Goal**: Home header currently shows `2-13 / 15th 分区` but no team identity. User can't tell which franchise they're managing without checking the sidebar banner. Add team abbrev + full name above the record line.

**Spec**:

Header layout becomes 3 centered lines:

```
 CHO Charlotte Hornets

       2-13
   15th in conference
```

(Or 2 lines if vertical room is tight: line 1 `CHO Charlotte Hornets`, line 2 `2-13 · 15th in conference`. Codex chooses based on header height — current header is 3 lines tall, so prefer the 3-line version with a blank gap line.)

Localization:
- Team abbrev: always raw uppercase from `Team::abbrev` (data, never localized).
- Team full name: `Team::name` from store. English in EN locale; ZH locale also shows the English `Team::name` field — locked invariant says "team abbreviations stay English (data, not chrome)" — extend that to team full names too. NO translation table for team names.
- "in conference" / "分区" suffix is chrome → localized via existing `T::HomeConferenceRank`.

**Files**:
- `crates/nba3k-cli/src/tui/screens/home.rs` — `draw_header` rebuild with new 3-line layout pulling `tui.user_abbrev` + a new field `user_team_name` mirrored from `SaveCtx`.
- `crates/nba3k-cli/src/tui/mod.rs` — `SaveCtx::load` reads `team_full_name` via `Store::team_name(team_id)` (add accessor if missing in `crates/nba3k-store/src/store.rs`).
- `crates/nba3k-store/src/store.rs` — `team_name(team_id) -> Result<Option<String>>` if not already exposed.

**Acceptance**:
- Home header shows e.g. `CHO Charlotte Hornets` line, then `2-13`, then `15th in conference`.
- Switching language → only the suffix `in conference / 分区` flips; team name stays English.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 289.
- Manual screenshot smoke vs image #13: identifying franchise is obvious at a glance.

---

## Coordination protocol (M25)

- Codex picks T10–T13 in any order; they touch mostly disjoint files. T11 (focus border) is the one that ripples — every screen's outer block changes — so leave it for last to avoid merge conflicts with T10/T12/T13.
- Commit format: `M25-T<N>: <one-line summary>`.
- Same i18n discipline: tables in lockstep when keys are added.
- Status notes in this file: codex flips `[ ]` → `[~]` → `[x]` and leaves a `→ codex: ...` line per task.

---

## T14 — Rotation player picker column alignment (unicode + position pad)

**Status**: `[x]`

→ codex: done — this commit — 296 unit tests passed; Rotation picker and slot rows use unicode display-width padding.

**Goal**: In the rotation slot picker modal (image #15), the OVR column drifts off by 1 column between rows. Two root causes; fix both.

**Root cause**:

1. `crates/nba3k-cli/src/tui/screens/rotation.rs:469-471` formatter:
   ```rust
   let picker: Picker<PlayerOption> = Picker::new(title, bucket, |o| {
       format!("{:<24}  {}  {} OVR", o.name, o.primary, o.overall)
   });
   ```
   `{}` for `o.primary` (the position string) doesn't pad. `C` → 1 char, `PF` → 2 chars. The OVR that follows shifts by 1 column between `C` rows and `PF` rows.

2. `{:<24}` uses Rust's byte-count width, not unicode display width. Names containing multi-byte chars (`Jusuf Nurkić`, `Moussa Diabaté`, `Tidjane Salaün`) get fewer trailing spaces than ASCII names of the same visual length, so the position column starts left-shifted on those rows.

**Fix**:

- Add `unicode-width = "0.1"` (or whatever the latest 0.1.x line is — check crates.io) to `crates/nba3k-cli/Cargo.toml` `[dependencies]`.
- Write a small helper in `rotation.rs` (or `tui/widgets.rs` if you prefer to share):
  ```rust
  use unicode_width::UnicodeWidthStr;

  fn pad_display(s: &str, target: usize) -> String {
      let w = UnicodeWidthStr::width(s);
      if w >= target {
          s.to_string()
      } else {
          let mut out = String::from(s);
          out.extend(std::iter::repeat(' ').take(target - w));
          out
      }
  }
  ```
- Change formatter to:
  ```rust
  format!("{}  {}  {} OVR",
      pad_display(&o.name, 22),
      pad_display(&o.primary, 2),
      o.overall)
  ```
  Position pad target = 2 (so `C` becomes `C `, matches `PF`). Name pad target = 22 (slightly tighter so 30-col modal still fits).
- Same fix should apply to **any other rotation row** that uses byte-pad on player names. The earlier T10 fix at lines 157/160/172/175 used `{:<28}` byte-pad on name body. Convert those four sites to `pad_display(...)` for the same reason — names with diacritics shift the hint column.

**Acceptance**:
- Picker modal: OVR digits start at the exact same column for every row regardless of position (`C` vs `PF`) or name characters (ASCII vs diacritics).
- Slot row in main rotation screen: hint column (`press Enter to change, c to clear`) starts at the same column whether the slot is empty, has Tatum, has Nurkić, or has any other name length.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ baseline (currently 292).
- Manual screenshot: rotation picker modal shows OVR perfectly aligned column.

**Commit**: `M25-T14: align rotation picker OVR column (unicode-width)`.

---

# M26 — Keyboard model unification (Enter / Tab / Space / arrows)

User goal: most TUI interactions should be possible with arrows + Enter + Tab + Space. Letter shortcuts allowed where they're the only practical option, but anywhere a "primary action" exists on a selected row, Enter should fire it. User-confirmed answers below; deviations require re-confirmation.

## T15 — Saves screen: Enter = Load

**Status**: `[x]`

→ codex: done — this commit — 296 unit tests passed; Enter and l/L share the saves load handler.

**Goal**: On Saves overlay, pressing `Enter` on the highlighted row loads that save. Currently requires `l` / `L`.

**Files**:
- `crates/nba3k-cli/src/tui/screens/saves.rs` — add `KeyCode::Enter` arm matching the existing `KeyCode::Char('l') | KeyCode::Char('L')` handler. Both keys keep working (Enter primary, `l` legacy).
- Action-bar hint update: replace `L Load` with `Enter Load` (keep `n New / d Delete / e Export` letters as secondary actions in their own action-bar slots).

**Acceptance**:
- Highlight a save → Enter → save loads, `Screen::Menu` appears with that save's context.
- `l` / `L` still work.
- New / Delete / Export still on their letter keys.

**Verification**:
- `cargo build --workspace` clean. `cargo test --workspace` ≥ 295.
- Manual smoke: open saves overlay → arrow → Enter → save loads.

**Commit**: `M26-T15: saves Enter loads selection`.

---

## T16 — Trades Inbox / My Proposals: Enter opens action picker

**Status**: `[x]`

→ codex: done — this commit — 296 unit tests passed; Enter opens localized response picker and a/r/c fast paths remain.

**Goal**: Three-action rows (Accept / Reject / Counter) currently bound to `a` / `r` / `c`. Replace with: Enter on selected offer pops a small action picker modal (3 rows: 接受 / 拒绝 / 还价), arrow + Enter fires the chosen action. `a/r/c` letters stay as a fast-path.

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs`:
  - Add `Modal::OfferAction { offer_id, picker: Picker<&'static T> }` variant (or use a simple list of localized strings + an enum tag).
  - `KeyCode::Enter` in Inbox tab + My Proposals tab opens the modal targeting the highlighted offer / chain. Keep existing `a/r/c` handlers.
  - Modal `Enter` → fire the same `respond_current_inbox(...)` / `respond_current_chain(...)` path.
  - Modal `Esc` closes without firing.
- `crates/nba3k-core/src/i18n.rs` + tables — reuse `T::TradesAccept`, `T::TradesReject`, `T::TradesCounter`. Add `T::TradesActionPickerTitle` (`"Respond to offer"` / `"响应报价"`).
- Action-bar text: replace `a/r/c` chips with `Enter Respond  a/r/c Quick`.

**Acceptance**:
- Highlight an inbox offer → Enter → 3-row picker → arrow → Enter on `Accept` / `Reject` / `Counter` → engine responds, message lands in last_msg.
- `a/r/c` shortcuts still fire directly without the picker.
- Same flow for My Proposals tab (when AI's turn).

**Verification**:
- `cargo build --workspace` clean. `cargo test --workspace` ≥ 295.

**Commit**: `M26-T16: trades action picker via Enter`.

---

## T17 — Roster sort: drop letters, Tab cycles, show current sort label

**Status**: `[x]`

→ codex: done — this commit — 296 unit tests passed; o/p/a/s sort shortcut grep clean and localized sort labels synced.

**Goal**: Replace `o` / `p` / `a` sort hotkeys with a single Tab-cycled sort selector. Show current sort label in Chinese (or localized — read `tui.lang`) at the top of the table.

**Sort cycle**: 总评 (OVR) → 位置 (Position) → 年龄 (Age) → back to 总评. Tab forwards, Shift-Tab backwards.

**Tab collision check**: Roster currently uses Tab to switch between `My Roster` ↔ `Free Agents` tabs. T18 below moves Free Agents to Trades, so Roster will no longer have sub-tabs after T18 — Tab is free. **T17 depends on T18 landing first**, OR T17 picks a different sort-cycle key (e.g. `s` "Sort" cycler).

**Decision**: do T18 → T17 in that order so Tab is free.

**Files**:
- `crates/nba3k-cli/src/tui/screens/roster.rs`:
  - Remove `KeyCode::Char('o' | 'p' | 'a')` arms.
  - Add Tab / BackTab arms that cycle a `RosterSort` enum (Overall → Position → Age → Overall).
  - Rebuild header line to include `format!("排序: {}", t(tui.lang, T::RosterSortOverall))` (or analogous) — display label updates on each cycle.
  - Action-bar: replace `o OVR / p Pos / a Age` chips with `Tab 排序 ({当前列名})`.
- `crates/nba3k-core/src/i18n.rs` + tables — add `T::RosterSortLabel` ("排序" / "Sort"), `T::RosterSortOverall` ("总评" / "Overall"), `T::RosterSortPosition` ("位置" / "Position"), `T::RosterSortAge` ("年龄" / "Age").

**Acceptance**:
- Roster screen no longer responds to `o` / `p` / `a`.
- Tab cycles sort: Overall → Position → Age → Overall, header label updates.
- Shift-Tab cycles backwards.
- Sort persists per-screen-session (thread-local cache); resets to Overall on save reload.

**Verification**:
- `cargo build --workspace` clean. `cargo test --workspace` ≥ 295.

**Commit**: `M26-T17: roster Tab-cycle sort with label`.

---

## T18 — Move Free Agents from Roster to Trades

**Status**: `[x]`

→ codex: done — this commit — 296 unit tests passed; Roster FA grep clean and Trades has 5-tab FA sign flow.

**Goal**: Drop the `Free Agents` tab from Roster screen. Add a new `Free Agents` sub-tab to Trades screen as the 5th tab (after Rumors).

**Files**:
- `crates/nba3k-cli/src/tui/screens/roster.rs`:
  - Remove the `Tab` keypress handling that switched to FA.
  - Remove FA-specific render path (`render_fa_tab` or similar).
  - Remove `s` Sign hotkey (it lived only in FA tab).
  - Roster becomes single-view "My Roster" only.
- `crates/nba3k-cli/src/tui/screens/trades.rs`:
  - Existing tab enum: `Inbox / MyProposals / Builder / Rumors`. Add `FreeAgents` as 5th.
  - FA tab render: list FAs (`Store::list_free_agents()` or whatever was used in roster), sortable by OVR, action `s` to sign (`Command::FaSign`). Keep `s` letter for sign — the per-row action is single-purpose so a letter is OK.
  - Tab nav already cycles via Tab — no extra wiring beyond the new tab variant.
  - Action-bar updates per active tab (FA: `↑↓ Move · s Sign · Tab Tabs · Esc Back`).
- `crates/nba3k-core/src/i18n.rs` + tables — `T::TradesFreeAgents` already exists? If not, add. Reuse `T::RosterFreeAgents` / `T::RosterSign` if present.
- Drop unused `T::RosterFreeAgents` if it becomes orphan after roster cleanup (only if grep confirms zero remaining users).

**Acceptance**:
- Roster screen shows only "My Roster" content; no Free Agents tab.
- Trades screen has 5 tabs; Tab cycles all 5.
- FA sign action ends up on the user team's roster (verifies via switch back to Roster screen + Tab cycles → no FA tab needed).
- Existing FA backend (M10 V006 free_agents table) untouched; only the UI surface moved.

**Verification**:
- `cargo build --workspace` clean. `cargo test --workspace` ≥ 295.
- Manual smoke: open Trades → Tab to Free Agents → arrow on top FA → s → roster grows by 1.

**Commit**: `M26-T18: move free agents from roster to trades`.

---

## T19 — Calendar: drop `1`-`6` sub-tab jump

**Status**: `[x]`

→ codex: done — this commit — 296 unit tests passed; Calendar 1-6 jump grep clean.

**Goal**: Remove the `KeyCode::Char(c @ '1'..='6')` direct sub-tab jumps. Tab / Shift-Tab already cycle the 6 sub-tabs (Schedule / Standings / Playoffs / Awards / All-Star / Cup).

**Files**:
- `crates/nba3k-cli/src/tui/screens/calendar.rs`:
  - Delete the `1..=6` match arm.
  - Action-bar: drop `1-6 Tabs` hint chip.
- Help overlay (`tui/mod.rs:870+` Calendar entry) — drop the `1-6 Jump tab` line.

**Acceptance**:
- Pressing `1` through `6` in Calendar does nothing (or, if those keys collide with menu shortcuts, the menu shortcut wins — but Calendar is an inner screen, so menu shortcut wouldn't fire mid-screen anyway; plain `1-6` are no-ops).
- Tab / Shift-Tab still cycle.

**Verification**:
- `cargo build --workspace` clean. `cargo test --workspace` ≥ 295.

**Commit**: `M26-T19: calendar drop 1-6 sub-tab jumps`.

---

## Coordination protocol (M26)

- Wave order: **T18 first** (frees Tab in Roster), then T17 (uses freed Tab), then T15 / T16 / T19 in any order (independent files).
- Commit format: `M26-T<N>: <one-line summary>`.
- Status notes: codex flips `[ ]` → `[~]` → `[x]` and leaves a `→ codex: ...` line per task.
- Every new T enum key syncs across `i18n.rs` + `i18n_en.rs` + `i18n_zh.rs`.

## Resolved decisions (2026-04-28)

- T15 Saves: Enter = Load (letter `l` retained as legacy alt).
- T16 Trades 3-action rows: Enter opens action picker; `a/r/c` retained.
- T17 Roster sort: drop `o/p/a` letters, Tab cycles, current sort label rendered at table header in user's language.
- T18 Free Agents: move out of Roster screen → Trades screen as 5th sub-tab (after Rumors).
- T19 Calendar: drop `1-6` direct-jump letters; Tab cycle remains.
- Roster row verbs (`t/e/x/R`): unchanged (Enter still opens Detail; letters fire action either on row or inside Detail).
- Roster main `Enter` behavior: unchanged (Detail modal).
- Finance `e` Extend: unchanged (no Enter handler added).
- Draft `s` Scout: unchanged.
- Rotation `c/C` clear: unchanged.
- Trades Builder `m/i/p`: unchanged (modifier keys, kept).

---

# M27 — Roster sort + Trades cleanup + FA seed + Home finance fit

User feedback after M26 release smoke. 4 fixes.

## T20 — Roster Tab cycle: OVR / PTS / Position only

**Status**: `[x]`

**Goal**: Change the Roster sort cycle from `OVR → Position → Age` (T17) to `OVR → PTS → Position`. Drop Age sort entirely. Default load = OVR. Each mode is a single-key sort — no multi-key tiebreak beyond a deterministic id fallback.

**Files**:
- `crates/nba3k-cli/src/tui/screens/roster.rs`:
  - Update `enum SortKey` variants: `Ovr`, `Pts`, `Position` (drop `Age`).
  - Tab cycle order: `Ovr → Pts → Position → Ovr`.
  - Sort impl per key:
    - `Ovr`: `(overall desc, id asc)` for stability.
    - `Pts`: `(season ppg desc, id asc)`. Use existing per-season totals helper (same source as Home league/team leaders).
    - `Position`: `(position asc, overall desc, id asc)`.
  - Header label: `format!("{} ({})", t(lang, T::RosterSortLabel), sort_label(lang, current_sort))`.
- `crates/nba3k-core/src/i18n.rs` + tables:
  - Drop `T::RosterSortAge` (no longer rendered).
  - Add `T::RosterSortPoints` ("得分" / "Points").
  - Keep `T::RosterSortOverall`, `T::RosterSortPosition`, `T::RosterSortLabel`.

**Acceptance**:
- Open Roster → header shows `排序: 总评`. Tab → `排序: 得分` (or `Points`). Tab → `排序: 位置`. Tab → back to `排序: 总评`.
- No path to "Age" sort.
- Within each mode, identical-key rows fall back to player id (no flicker between renders).

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 295.
- Manual: open Roster, sim a week, confirm PTS sort orders by current ppg.

**Commit**: `M27-T20: roster sort cycle ovr/pts/position`.

---

## T21 — Trades: drop Rumors sub-tab

**Status**: `[x]`

**Goal**: Remove the `Rumors` tab from the Trades screen. Final tab order: `Inbox / My Proposals / Builder / Free Agents` (4 tabs). Backend `cmd_rumors` CLI command stays — only the TUI surface drops.

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs`:
  - Drop `SubTab::Rumors` variant.
  - Drop `draw_rumors` + `handle_rumors_key` + Rumors-specific cache fields.
  - Tab cycle now wraps over 4 tabs.
- `crates/nba3k-core/src/i18n.rs` + tables — keep `T::TradesRumors` (CLI may still use). If grep confirms zero TUI users, can leave the key as future-use.

**Acceptance**:
- Trades screen shows 4 tabs labeled (in EN/ZH) `Inbox / My Proposals / Builder / Free Agents`.
- Tab/Shift-Tab cycles 4 tabs.
- CLI `nba3k --save x.db rumors` still works.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 295.

**Commit**: `M27-T21: trades drop rumors tab`.

---

## T22 — Seed a real Free Agent pool on new game

**Status**: `[x]`

**Goal**: Currently `list_free_agents()` returns 0 rows on every new save because the scraper only inserts rostered players (`team_id` set, `is_free_agent = 0`) and draft prospects (`team_id NULL`, `is_free_agent = 0`). Real NBA has plausible FAs (post-buyout vets, unsigned veterans). Add a curated FA seed pool and bootstrap it on `cmd_new`.

**Approach**:
- New static file `data/free_agents_2025_26.toml` with ~25 entries. Each entry:
  ```toml
  [[fa]]
  name = "Russell Westbrook"
  primary_position = "PG"
  secondary_position = "SG"
  age = 37
  overall = 75
  potential = 75
  ```
  Curate from real 2025-26 unsigned veterans (Westbrook, Markkanen-tier? actually mid-Feb status — pick plausible ones). 25 entries spread across positions.
- New helper `crates/nba3k-cli/src/commands.rs::seed_free_agents(store) -> Result<u32>`:
  1. Read TOML at compile-time via `include_str!` (no fs read at runtime).
  2. For each entry, check if a player with that exact name already exists. Skip if so.
  3. Otherwise `INSERT INTO players (name, primary_position, secondary_position, age, overall, potential, ratings_json, contract_json, team_id, is_free_agent, ...) VALUES (..., NULL, 1, ...)` with synthetic ratings (OVR-tier defaults same as the scraper's M3 contract injection) and zero contract.
  4. Return inserted count.
- Call `seed_free_agents` from `cmd_new` (after team rosters seeded) AND from a one-time idempotent backfill helper invoked at TUI launch (`tui::run` start) so existing saves get the pool too. Backfill skips when `count_free_agents() > 0`.

**Files**:
- New: `data/free_agents_2025_26.toml`.
- Edit: `crates/nba3k-cli/src/commands.rs` — `seed_free_agents` helper + call from `cmd_new`. Use `serde` + `toml` (already workspace deps).
- Edit: `crates/nba3k-cli/src/tui/mod.rs` — at `run()` top, after store opens but before TUI loop, call `seed_free_agents` if `count_free_agents() == 0`. Wrap in `with_silenced_io`.
- Edit: `Cargo.toml` of nba3k-cli if `toml` / `serde_derive` not yet imported (they should be — used elsewhere).

**Out of scope**:
- Real-time FA market dynamics (cap-aware bids, AI signing pressure) — already exists (M12 ai_fa_pass).
- Scraper integration (don't touch nba3k-scrape).

**Acceptance**:
- Fresh `nba3k new --team BOS --save x.db` → `nba3k --save x.db fa list` shows ≥20 free agents.
- Existing save opened in TUI → Trades → Free Agents tab → shows ≥20 FAs after first launch.
- Re-running TUI on same save does NOT duplicate (idempotent on `count > 0`).
- Sign a FA → roster grows by 1, FA pool shrinks by 1.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 295. Add 1 unit test in `nba3k-cli/tests` (or inline) confirming `seed_free_agents` inserts the right count + is idempotent.

**Commit**: `M27-T22: seed free agent pool on new game`.

---

## T23 — Home Finance panel: label/value alignment + value fit

**Status**: `[x]`

**Goal**: Image #18 shows the 财务 panel with values truncated (`17,5` / `$212` / `$30.` / `$154.0` / `$163.` / `$162.`). Two root causes:
1. Label `{:<16}` byte-pad doesn't account for CJK display width — labels visually mis-align.
2. Values like `$212.34M` are 8-9 columns; panel is too narrow to render label + 16 bytes pad + 9-col number, so the number gets clipped at the right border.

**Fix**:
- Replace the manual `Line::from(vec![Span(...label), Span(value)])` approach with a 2-column `Table`:
  ```rust
  let rows = s.finances.iter().map(|(label, value)| {
      Row::new([
          Cell::from(t(tui.lang, *label)),
          Cell::from(value.clone()).style(theme.accent_style()),
      ])
  });
  let table = Table::new(rows, [Constraint::Percentage(60), Constraint::Percentage(40)])
      .block(theme.block(t(tui.lang, T::HomeFinances)));
  ```
- Right-align the value column so trailing zeros/period don't fall off panel: pre-format values via `format!("{:>10}", value)` or use ratatui's `Cell::from(...).alignment(Alignment::Right)` if API supports it (it doesn't for `Cell` directly — wrap in `Text`/`Line` with `.alignment(Alignment::Right)`).
- For `format_number` (attendance) — produce shorter form: `17_500` → `17.5K` or just `17,500` already fits; check. Image shows `17,5` which is the 5-digit `17,500` getting truncated, so wider value column is enough.
- Drop `{:<16}` byte-pad on label entirely — `Table` handles label column width.

**Files**:
- `crates/nba3k-cli/src/tui/screens/home.rs` — rewrite `draw_finances`. Same data source, new layout.
- Optional: shrink the right-column width budget upstream so finances panel gets ≥ 24 cols of width. Currently right column is `Constraint::Percentage(50)` of body, then split 45/55 between team-stats and finances. If terminal is 80 cols, right column = 25 cols; finances = 13-14 cols → too tight. Bump to `Constraint::Length(28)` or rebalance: team-stats 40% + finances 60% so finances gets ~15 cols.

**Acceptance**:
- All 6 finance lines render fully:
  ```
  平均上座         16,469
  本季收入         $212.34M
  本季利润           $0.48M
  现金            $165.58M
  工资单          $258.49M
  工资帽          $154.65M
  ```
- No truncation at default 80-col terminal.
- Labels CJK-aligned (`pad_display` not needed if `Table` columns auto-fit).
- TV theme + EN locale also fits.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 295.
- Manual screenshot at 80 cols + 120 cols + TV preset, all 6 lines fully readable.

**Commit**: `M27-T23: home finance panel fit + alignment`.

---

## Coordination protocol (M27)

- T20–T23 are independent files; do in any order.
- Commit format: `M27-T<N>: <one-line summary>`.
- Same i18n discipline.

## Resolved decisions (2026-04-28)

- T20 sort cycle: OVR (default) → PTS → Position. Age dropped.
- T21 Trades: 4 tabs (Inbox / My Proposals / Builder / Free Agents). CLI rumors cmd stays.
- T22 FA seed: curated TOML at `data/free_agents_2025_26.toml`, bootstrap on cmd_new + idempotent backfill on TUI open. Skip scraper changes.
- T23 Finance: rewrite as 2-col Table; rebalance right column width if needed.

---

# M27.5 — Post-release polish (T24-T27)

User-tested release build, found 4 more issues. Same protocol as previous M-段.

## T24 — TextInput: ←/→/Home/End/Delete cursor nav

**Status**: `[x]`

**Goal**: NewGame wizard `Save path` field currently only supports Backspace. User can't move cursor mid-string to edit. Add full cursor navigation.

**Files**:
- `crates/nba3k-cli/src/tui/widgets.rs` — `TextInput::handle_key`:
  - `KeyCode::Left` → `if self.cursor > 0 { self.cursor -= 1 }`.
  - `KeyCode::Right` → `if self.cursor < self.value.chars().count() { self.cursor += 1 }`.
  - `KeyCode::Home` → `self.cursor = 0`.
  - `KeyCode::End` → `self.cursor = self.value.chars().count()`.
  - `KeyCode::Delete` → remove char AT cursor position (don't decrement cursor).
- `TextInput::render` — current renders `value` then `█`. Change to render: chars before cursor, then `█` (cursor block) overlaid on / inserted at cursor position, then chars after. Simplest: build the line in two spans:
  ```rust
  let chars: Vec<char> = self.value.chars().collect();
  let (left, right) = chars.split_at(self.cursor);
  let line = Line::from(vec![
      Span::styled(format!(" {} ", self.label), theme.accent_style()),
      Span::styled(left.iter().collect::<String>(), theme.text()),
      Span::styled("█", theme.text()),
      Span::styled(right.iter().collect::<String>(), theme.text()),
  ]);
  ```

**Acceptance**:
- NewGame wizard Save path: type some text → press Left → cursor moves left → type → char inserted at cursor → Delete → char at cursor removed → Home/End jump to ends.
- Backspace continues to remove char before cursor.
- Visual cursor (█) tracks the actual `self.cursor` position.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 296.
- Add 1 unit test for TextInput cursor nav (insert / delete / home / end).

**Commit**: `M27-T24: textinput cursor navigation`.

---

## T25 — Quit confirm: Enter = quit, Esc = stay; drop y/n

**Status**: `[x]`

**Goal**: Currently quit confirm modal accepts `y/n`. Replace with: Enter quits, Esc cancels. Remove `y/n` letter shortcuts entirely.

**Files**:
- `crates/nba3k-cli/src/tui/widgets.rs` — `Confirm::handle_key`:
  - Drop `KeyCode::Char('y' | 'Y')` and `KeyCode::Char('n' | 'N')` arms.
  - `KeyCode::Enter` → `WidgetEvent::Submitted`.
  - `KeyCode::Esc` → `WidgetEvent::Cancelled`.
- `crates/nba3k-cli/src/tui/mod.rs` — modal action bar copy: change `y Yes / n No` chips to `Enter Confirm / Esc Cancel`. Localize via existing `T::CommonConfirm` / `T::CommonCancel`.

**Acceptance**:
- Press `q` from any inner screen → modal "Quit nba3k?" → Enter quits, Esc returns to previous screen.
- `y` / `n` keys do nothing in the quit modal.
- All other Confirm widgets in the app (Cut, ClearAllStarters, etc.) inherit the same behavior.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 296.
- Search confirms `KeyCode::Char('y')` and `KeyCode::Char('n')` removed from `Confirm::handle_key`.

**Commit**: `M27-T25: confirm modal enter/esc only`.

---

## T26 — First frame after load shows Home dashboard

**Status**: `[x]`

**Goal**: Currently after Launch → Continue / Load Game / NewGame finishes, user lands on `Screen::Menu` with `> 1. Home` highlighted but right pane shows the static blurb "Press Enter to open." User has to press Enter (or arrow once) to see the Home dashboard. We want the Home dashboard rendered on the very first frame.

**Files**:
- `crates/nba3k-cli/src/tui/mod.rs`:
  - `TuiApp::switch_save` (or wherever a save is loaded into context) — after `set_save_ctx(...)`, set `self.current = Screen::Home` and `self.preview_mode = true`.
  - `screens/launch.rs` Continue / NewGame flows that route to Menu — after the dispatch / load completes, set `tui.current = Screen::Home; tui.preview_mode = true;` (and `menu_selected = 0` to keep sidebar in sync).
- Sidebar drawing already shows the menu when `has_save()`; that part stays.
- The `derived_focus` logic from T11 still keeps the yellow border on the sidebar when `preview_mode == true`, which is desired (user is hovering Home item, not yet committed).

**Acceptance**:
- New game: wizard → confirm → first frame shows Home dashboard (record / standings / leaders / finances / lineup) with sidebar yellow-bordered (preview).
- Load existing save from Launch Continue: same.
- Arrowing down from sidebar still live-previews other screens (T3 behavior).
- Pressing Enter on `> 1. Home` is now a no-op (current already matches selection); pressing Enter on another menu item still focuses that screen.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 296.
- Manual: launch → new game → first frame is Home, NOT the blurb.

**Commit**: `M27-T26: first frame renders home dashboard`.

---

## T27 — Finance screen: apron labels, Tab sort, drop NOTES

**Status**: `[x]`

**Goal**: Image #21 shows 4 issues:
- Two rows both labeled `硬帽线` (back-end is `apron_1` + `apron_2`). Rename to `第一档奢侈线` / `第二档奢侈线`.
- Sort hotkeys `t/y/n` are opaque; convert to Tab-cycle sort with localized header label (same pattern as Roster T17/T20).
- `NOTES` column is unused, drop it.
- Keep `under` / `over` in English (user explicitly said EN).

**Files**:
- `crates/nba3k-cli/src/tui/screens/finance.rs`:
  - Rebuild cap-summary lines:
    - `工资帽` + cap (existing `T::FinanceCap`).
    - `奢侈税` + tax line (existing `T::FinanceTax` if present, otherwise add).
    - `第一档奢侈线` + apron_1.
    - `第二档奢侈线` + apron_2.
    - Each line keeps `... $XX.XXM under` (English `under` / `over`).
  - Drop `KeyCode::Char('t' | 'y' | 'n')` sort arms; add `Tab` / `BackTab` cycling a `FinanceSort` enum: `Total → Years → Total` (only 2 modes per user's ask, drop name sort or keep as 3rd? user said "总额, 年限" → 2 modes).
  - Header line for contracts table renders `format!("{} {}: {}", t(lang, T::CommonSort), '—', t(lang, sort_label))` or similar, mirroring Roster's `排序: 总额` style.
  - Drop the `NOTES` column from contracts table — `Row::new` and column constraints both shrink.
- `crates/nba3k-core/src/i18n.rs` + tables:
  - Drop `T::FinanceApron` (was reused for both apron_1 and apron_2).
  - Add `T::FinanceApron1` ("第一档奢侈线" / "First Apron") and `T::FinanceApron2` ("第二档奢侈线" / "Second Apron").
  - Add `T::FinanceSortTotal` ("总额" / "Total") and `T::FinanceSortYears` ("年限" / "Years").
  - `T::FinanceTax` if not already present ("奢侈税" / "Luxury Tax").

**Acceptance**:
- Cap summary lists 4 distinct lines: 工资帽 / 奢侈税 / 第一档奢侈线 / 第二档奢侈线.
- `under` / `over` stay English on every line regardless of locale.
- Pressing Tab cycles sort: Total → Years → Total. Header shows current.
- Pressing `t/y/n` does nothing (former hotkeys removed).
- NOTES column gone from contracts table.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 296.
- Manual screenshot vs image #21: 4 distinct apron labels, Tab cycles sort, no NOTES column.

**Commit**: `M27-T27: finance apron labels + tab sort + drop notes`.

---

# M28 — Sim animation (frame-by-frame day playback)

User wants Ctrl+W / Ctrl+N / Ctrl+T / Ctrl+A to play out day-by-day on screen instead of jumping to end-state. Sees 0-1 → 1-1 → 2-1 record progression, standings reorder, etc. Single Ctrl+D stays instant (1 day = no animation). Esc / q interrupts and jumps to end.

## T28 — Animated multi-day sim

**Status**: `[x]`

**Goal**: Replace the current "single-shot dispatch" for Ctrl+W (week), Ctrl+N (month), Ctrl+T (trade deadline), Ctrl+A (season advance) with a frame-loop variant: dispatch one day at a time, redraw the visible screen, sleep ~50ms, repeat. Esc / q during the loop cuts straight to end-state.

**Spec**:

- New helper `crate::tui::run_animated_sim(app, tui, terminal, target: SimTarget) -> Result<()>`:
  - `SimTarget::Days(n)` for week (7) / month (30).
  - `SimTarget::TradeDeadline` — compute target day from `season_phases::trade_deadline_day(...)`.
  - `SimTarget::SeasonAdvance` — loop until `state.phase == OffSeason` then run season-advance (advance has its own multi-step sub-flow; for animation, only animate the day-by-day until end of regular season, then run season-advance instantly afterwards).
- Loop body:
  ```rust
  for _ in 0..target_days {
      if event::poll(Duration::from_millis(0))? {
          if let Event::Key(k) = event::read()? {
              if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                  break;  // user interrupted
              }
          }
      }
      with_silenced_io(|| commands::dispatch(app, Command::SimDay { count: Some(1) }))?;
      tui.refresh_season_state(app)?;
      invalidate_all_screens(tui);
      terminal.draw(|f| draw(f, app, tui))?;
      std::thread::sleep(Duration::from_millis(50));
  }
  // If interrupted, dispatch the remainder as a single fast call:
  let remaining = target_days - completed;
  if remaining > 0 {
      with_silenced_io(|| commands::dispatch(app, Command::SimDay { count: Some(remaining) }))?;
      tui.refresh_season_state(app)?;
      invalidate_all_screens(tui);
  }
  ```
- Animation gate: Ctrl+D stays instant (no loop); Ctrl+W / Ctrl+N / Ctrl+T / Ctrl+A go through `run_animated_sim`.
- Frame rate: 50ms per day → month animates in 1.5s. Tunable constant `ANIMATION_DELAY_MS = 50`.
- Status line during animation: top banner `Sim 进行中... [Esc 跳过]` / `Simulating... [Esc skip]`. Locale-aware via `T::SimAnimating` + `T::SimSkip`.
- The `terminal.draw` inside the loop must not double-buffer poorly — ratatui handles diff rendering, expected to be cheap.

**Files**:
- `crates/nba3k-cli/src/tui/mod.rs`:
  - Refactor `run_global_sim` for Day to stay direct, others to call `run_animated_sim`.
  - New `run_animated_sim` helper.
  - Pass `&mut Terminal<...>` from the event loop down — currently `event_loop` owns the terminal but `handle_key` doesn't. Plumb terminal handle through `handle_key` → `handle_global_sim_key` → `run_animated_sim`. Or: collect a "pending sim" intent on key handle, return to event loop, let event loop drive the animation in its own context where terminal is in scope.
  - **Recommended approach**: introduce `pub enum PendingSim { None, Animated(SimTarget) }` field on `TuiApp`. Key handler sets it. Event loop, after handle_key returns, checks `tui.pending_sim`; if set, runs the loop with terminal in hand, clears flag.
- `crates/nba3k-core/src/i18n.rs` + tables — `T::SimAnimating`, `T::SimSkip`.

**Acceptance**:
- Ctrl+W from any screen → top banner shows "Simulating..." → record / standings / leaders update visibly day-by-day for ~350ms total → settles to end-state.
- Pressing Esc / q during animation → instant fast-forward to end-state.
- Ctrl+D still instant (single day, no animation).
- Ctrl+N animates 30 days (~1.5s).
- Ctrl+T animates from current day to trade deadline (variable count).
- Ctrl+A animates regular-season tail then runs season-advance instantly.

**Edge cases**:
- Already past target day → no-op (existing CLI behavior).
- Phase transition mid-animation (e.g., Regular → Playoffs) — let it happen, just keep redrawing.
- Banner key handler should ignore further Ctrl+letter presses while animation runs (prevent re-entrance).

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 296.
- Manual: trigger Ctrl+W on Home → see record tick over from 0-0 to 7-0 day by day.
- Manual: trigger Ctrl+N → mid-flight Esc → instant settle.

**Commit**: `M28-T28: animated multi-day sim`.

## Resolved decisions (2026-04-28)

- M27.5: T24 cursor nav, T25 Enter/Esc only, T26 home first-frame, T27 finance polish.
- T27 sort modes: Total → Years (2 modes only). NOTES column dropped.
- T27 apron names: 第一档奢侈线 / 第二档奢侈线.
- T27 `under` / `over` stays English.
- M28: animation per Ctrl+W/N/T/A, ~50ms/day, Esc/q skip; Ctrl+D stays instant.

---

# M29 — New game polish (T29-T31)

User feedback after release smoke. 3 small fixes.

## T29 — New-game wizard: drop Seed step

**Status**: `[x]`

→ codex: done — this worktree — removed Seed from the TUI wizard, confirm now uses a fresh random `u64` seed, CLI `--seed` remains unchanged; workspace build/test passed.

**Goal**: NewGame wizard currently has 6 steps: SavePath / Team / Mode / Season / Seed / Confirm. Drop the Seed step. Internal default uses a random seed per save so user doesn't see the option.

**Files**:
- `crates/nba3k-cli/src/tui/screens/new_game.rs`:
  - Drop `Step::Seed` from the `Step` enum.
  - Drop `seed: NumberInput` field from `WizardState`.
  - Drop the Seed render arm + step navigation + step indicator entry.
  - On `Confirm` dispatch, populate `NewArgs.seed` with `rand::random::<u64>()` so each new game has a fresh seed.
  - Update step indicator copy: "5/6 — Seed" gone; renumber `Confirm` to `5/5`.
  - Module doc-comment first line: "Six steps (...)" → "Five steps (...)".
- `crates/nba3k-core/src/i18n.rs` + tables — `T::NewGameSeed` becomes orphan; safe to leave or drop.
- CLI `new` subcommand keeps `--seed` flag (CLI invariant). Only the TUI wizard surface drops it.

**Acceptance**:
- Wizard flow: SavePath → Team → Mode → Season → Confirm.
- Each new game gets a fresh seed (running the wizard twice with same team produces different sim results from day 1).
- CLI `nba3k new --team BOS --save x.db --seed 42` still accepts `--seed`.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 298.

**Commit**: `M29-T29: drop seed step from new-game wizard`.

---

## T30 — Auto-assign team roles on new game

**Status**: `[x]`

→ codex: done — this worktree — `cmd_new` now assigns initial roles for all teams after starter population; added idempotent role distribution unit test; workspace build/test passed.

**Goal**: Currently all players show `role = RolePlayer` (角色). After new game, auto-distribute roles per OVR rank within each team so Roster screen shows a meaningful mix.

**Role thresholds** (per team, applied right after `cmd_new` finishes seeding):

| Rank / Condition | Role |
|---|---|
| OVR ≥ 88 | Star (球星) |
| Top 1-5 by OVR among non-Stars | Starter (首发) |
| Rank 6-7 | SixthMan (第六人) |
| Rank 8-12 | RolePlayer (角色) |
| Rank 13+ | BenchWarmer (替补) |
| Age ≤ 21 AND OVR < 75 | Prospect (新秀) — overrides BenchWarmer for young low-OVR players |

Tiebreak within OVR ties: id ascending.

**Files**:
- `crates/nba3k-cli/src/commands.rs`:
  - New helper `pub(crate) fn assign_initial_roles(store: &nba3k_store::Store, team_id: TeamId) -> Result<u32>`. Reads roster, sorts desc by OVR + id asc, applies the table above, writes role via existing role-write API. Returns count updated.
  - Call from `cmd_new` for **all 30 teams**, right after `populate_default_starters` block.
- `crates/nba3k-store/src/store.rs` — confirm a role-write API exists; add if missing.

**Idempotency**: helper safe to re-run. Backfill on TUI launch NOT required — only `cmd_new` runs it. Existing saves keep their old roles.

**Acceptance**:
- Fresh new game BOS → Menu → Roster → mix shows Tatum=Star, Brown=Starter, Porziņģis=Starter, etc., not all "角色".
- Default 5 starters' roles all show Star or Starter.
- Sim a week → roles preserved (don't auto-rewrite mid-season).
- 30 teams all have mixed roles.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 298.
- 1 unit test in commands.rs asserting role distribution for a synthetic team with known OVR rank.

**Commit**: `M29-T30: auto-assign team roles on new game`.

---

## T31 — Sim animation slower (50ms → 100ms per day)

**Status**: `[x]`

→ codex: done — this worktree — `ANIMATION_DELAY_MS` bumped from 50ms to 100ms; workspace build/test passed.

**Goal**: Current `ANIMATION_DELAY_MS = 50` is too fast to read. Bump to 100ms.

**Files**:
- `crates/nba3k-cli/src/tui/mod.rs` — `const ANIMATION_DELAY_MS: u64 = 100;` (was 50).

**Acceptance**:
- Ctrl+W animates 7 days in ~700ms.
- Ctrl+N animates 30 days in ~3s.
- Esc/q still cuts to instant.

**Commit**: `M29-T31: sim animation 100ms per day`.

---

## Coordination protocol (M29)

- T29 / T30 / T31 independent, any order.
- Commit format: `M29-T<N>: <one-line summary>`.

## Resolved decisions (2026-04-28)

- T29 seed: drop wizard step; CLI `--seed` flag stays.
- T30 role table: Star ≥88 / top1-5 Starter / top6-7 SixthMan / top8-12 RolePlayer / top13+ BenchWarmer / age≤21+OVR<75 Prospect override.
- T31 anim: 100ms/day.
