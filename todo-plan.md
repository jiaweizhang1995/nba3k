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

**Status**: `[ ]`

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

→ codex: done — this commit — 290 unit tests passed; mandate grep 0 non-migration hits.

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

**Status**: `[ ]`

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

**Status**: `[ ]`

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

**Status**: `[ ]`

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
