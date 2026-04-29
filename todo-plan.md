# todo-plan.md — M30 Trade builder redesign (codex CLI execution doc)

**Maintainer**: main agent (Claude). Codex picks tasks, flips status, asks main agent if scope shifts.

**Working dir**: `/Users/jimmymacmini/Desktop/claude-code-project/nba3k-claude`
**Build**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --workspace`
**Test**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo test --workspace`
**Release**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --release --bin nba3k`
**Run**: `./target/debug/nba3k tui` / `./target/release/nba3k tui`

**Phase tracker**: `phases/PHASES.md`

**Status legend**: `[ ]` not started · `[~]` in progress · `[x]` done · `[!]` blocked (write reason)

**Locked invariants** (do not break):
- TUI mutations always route through `crate::commands::dispatch(app, Command)` wrapped in `crate::tui::with_silenced_io(|| ...)`.
- CLI/REPL command surface stays untouched.
- Player names + team abbreviations + team full names stay English (data, not chrome).
- Tests must pass before marking task done. Baseline: 299 unit + 1 integ.
- i18n: every new UI string goes through `t(tui.lang, T::...)`. Add new T keys when needed; keep `i18n.rs` + `i18n_en.rs` + `i18n_zh.rs` in sync.

---

## Goal

Replace the current cramped 4-pane trade builder (team list / our roster / their roster / submit, image #24) with a 2-step UI:

1. **Step 1 — Team picker**: left = 30-team list, right = preview of that team's roster (read-only). Enter selects target. Esc returns to Trades menu.
2. **Step 2 — Builder**: master-detail layout. Top bar (target team / our team / 切 3 队 / 改队 / 强制成交). Two wide panels (我方送出 / 对方送出), each is a single scrollable list with section dividers (球员 first; 选秀权 deferred to M31). Bottom verdict bar showing salary totals + plain-language CBA warnings + post-submit GM dialog.

Drop the percentage-based "estimated acceptance" — replace with natural-language GM responses (Basketball-GM style). Only show CBA / cap warnings when the trade actually violates a rule, in plain language (no `CBA ✓/✗` shorthand).

---

## T32 — Step 1: Team picker screen

**Status**: `[x]` → codex: implemented team picker, roster preview, letter jump, Enter/Esc flow.

**Goal**: New first step inside the Trades > Builder sub-tab. User picks the target team before any player/asset selection happens. Two-column layout, left team list, right roster preview.

**Layout**:

```
┌─ 选目标队 ─────────────────────────────────────────────────────┐
│ ATL Atlanta Hawks  │  Atlanta Hawks (40-32, $138M)            │
│ BOS Boston Celtics │  ────────────────────────────────────    │
│ ...                │  PG  Trae Young     85 OVR  $40M 3y     │
│ DAL Dallas Mavs    │  SG  Jalen Johnson  82 OVR  $5M  4y     │
│ ...                │  SF  De'Andre Hunter 80 OVR $19M 4y     │
│ (highlight = ATL)  │  PF  Onyeka Okongwu 78 OVR  $14M 2y     │
│                    │  C   Clint Capela   74 OVR  $7M  1y     │
│                    │  ...  (top 12 by OVR)                    │
│                    │  ────────────────────────────────────    │
│                    │  Payroll: $138.2M · Cap: $154.6M          │
└────────────────────────────────────────────────────────────────┘
↑↓ 队伍 · Enter 选定 · A-Z 字母跳转 · Esc 返回交易菜单
```

- Left column ~24 cols (3-letter abbrev + truncated full name).
- Right column gets the rest. Roster preview shows top 12 by OVR with: position / name / OVR / salary / contract years.
- Letter-key jump: pressing a letter (e.g. `B`) jumps cursor to first team whose abbrev starts with that letter (BOS / BRK).
- Enter advances to Step 2 (T33), passing the chosen team.
- Esc returns to Trades menu.

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs`:
  - Refactor builder state machine into a 2-step enum: `BuilderStep::PickTeam` / `BuilderStep::Compose { target: TeamId }`.
  - New `draw_pick_team(...)` + `handle_pick_team_key(...)`. The existing `draw_builder` becomes step 2 (T33).
  - On Trades > Builder tab activation, default to PickTeam step.
- `crates/nba3k-core/src/i18n.rs` + tables — add `T::TradesPickTeamTitle`, `T::TradesRosterPreview`, `T::TradesPayrollCap`.

**Acceptance**:
- Trades > Builder opens to team picker (NOT directly to the player select view).
- Right pane shows real top-12 roster + payroll/cap of highlighted team.
- Letter jump works (`A` → ATL, `B` → BOS, etc.).
- Enter on highlighted team transitions to Step 2 with that team locked as target.
- Esc returns to Trades menu.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 299.
- Manual: Trades > Builder → see team picker → arrow → Enter → step 2 opens.

**Commit**: `M30-T32: trade builder step 1 team picker`.

---

## T33 — Step 2: Builder body (master-detail, single-list per side)

**Status**: `[x]` → codex: implemented two-step compose screen, two-panel player lists, selection controls, M31 pick placeholders.

**Goal**: Replace the cramped 4-column builder with a 2-column master-detail layout. Top bar shows context. Two wide panels for player selection. Bottom verdict bar (T34/T35).

**Layout**:

```
┌─ 交易构建 — 目标 ATL · 我方 CHI ────────────────────────────┐
│ [m 切 3 队]  [T 改队]  [F 强制成交 — 仅 god 模式]              │
├─ 我方送出 (✓3 / 18) ────────┬─ 对方接收 (✓2 / 17) ───────────┤
│ ─ 球员 ─                    │ ─ 球员 ─                       │
│ ✓ LaVine    PG 28 87 $43M 2y│ ✓ Young     PG 26 85 $40M 3y  │
│ ✓ White     SG 24 78 $12M 1y│ ✓ Johnson   SF 22 79 $5M  4y  │
│ ✓ Vučević   C  35 75 $20M 1y│   Hunter    SF 26 80 $19M 4y  │
│   Giddey    PG 23 80 $11M 1y│   Okongwu   PF 24 78 $14M 2y  │
│   ...                       │   ...                         │
│ ─ 选秀权 (M31 待加) ─        │ ─ 选秀权 (M31 待加) ─          │
├─ verdict (T34 / T35) ───────┴────────────────────────────────┤
│ 送 $75M / 收 $45M / 净 +$30M 进                                │
│ (warnings + GM dialog 在此)                                  │
└──────────────────────────────────────────────────────────────┘
```

**Per-side single-list**:
- One scrollable list with section divider rows. ↑↓ navigates **across** sections (no special key to jump). Space toggles select on focused row. Divider rows are not selectable; cursor skips them.
- Sections:
  - `─ 球员 ─` — all team players sortable by OVR desc (then id asc). All visible (scroll if > 15).
  - `─ 选秀权 (M31 待加) ─` — placeholder section, empty list, "暂未支持" italic muted text. Wired in M31. Section header still rendered so users see it's coming.
- Selected rows show ✓ prefix, unselected `  `. Selected rows highlight in `theme.highlight()`.

**Player row format** (6 columns, NO role column per user):
- Name (16 chars, `pad_display` unicode-aware truncation if longer)
- Position (2 chars, `{:<2}`)
- Age (2 chars, `{:>2}`)
- OVR (2 chars, `{:>2}`)
- Salary (7 chars, `{:>7}` — e.g. `$43.5M`)
- Contract years (3 chars, `{:>3}` — e.g. `2y` or `—` for expiring)

Total width per row: ~37 + 2 ✓ prefix = ~39 cols. Each side gets ~50% of body = 40 cols on 80-col terminal. Fits.

**Top bar buttons**:
- `m` toggle 2-team / 3-team mode (existing; preserved).
- `T` open Step 1 team picker overlay (re-pick target without losing current selections).
- `F` force-submit (god mode only — see T37; hidden when god mode off).

**Multi-side navigation**:
- `Tab` / `Shift-Tab` switches focus between 我方送出 (left) and 对方接收 (right) panels.
- `i` cycles incoming team in 3-team mode (existing; preserved).

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` — replace existing `draw_builder` and key handlers with the new 2-pane layout. Drop the team-picker column + the right-side "提交" status panel (status moves to bottom verdict bar in T34/T35).
- `crates/nba3k-core/src/i18n.rs` + tables — add:
  - `T::TradesBuilderTitle`, `T::TradesBuilderTopBar`
  - `T::TradesSendList`, `T::TradesReceiveList`
  - `T::TradesSectionPlayers`, `T::TradesSectionPicks`, `T::TradesPicksDeferred` ("M31 待加 — 暂未支持" / "Coming in M31").

**Esc behavior** (per user):
- Esc once → return to Step 1 team picker (T32) with current selections preserved (so user re-picks team but doesn't lose progress).
- Esc again from Step 1 → return to Trades menu / sub-tab strip.

**Acceptance**:
- After T32 selects ATL, builder opens with target locked to ATL.
- Each side scrollable, single-list with section dividers.
- Player rows show 6 columns aligned.
- Tab cycles focus left ↔ right pane (with focus border per existing T11 logic).
- m / T / F top-bar keys work; F only visible when god mode on (T37).
- Esc once → back to Step 1; Esc twice → back to Trades menu.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 299.

**Commit**: `M30-T33: trade builder step 2 master-detail`.

---

## T34 — Verdict bar: salary totals + plain-language CBA warnings

**Status**: `[x]` → codex: implemented salary totals and plain-language warning bar backed by trade CBA validation.

**Goal**: Bottom strip of builder shows current selection state + warnings. Replace the cryptic `CBA: ✓/✗` with full natural-language warnings (Basketball-GM style — image #25).

**Bar contents** (always visible at bottom of step 2):

```
─ 提交预览 ────────────────────────────────────────────────────
送 $75M (3 球员)  /  收 $45M (2 球员)  /  净 +$30M 进
[警告条 — 见下]
[Enter 提交 · c 清除选择 · Esc 回选队]
```

**Salary line**: always 3 numbers — sent, received, delta. Update on every Space toggle.

**Warning conditions** — show in red panel only when violated. Plain language:

- **CHI 在工资帽以上**:
  ```
  ⚠ 你已超过工资帽. 进薪 ≤ 送薪 × 125%.
  当前 进/送 = {actual}%, 超出 {diff}%.
  需削减进薪约 ${diff}M.
  ```
- **CHI 在硬帽线以上**:
  ```
  ⚠ 你已触及第一/第二档奢侈线 (硬帽). 该交易会让你超出.
  净进薪 ${X}M 超出剩余空间 ${Y}M.
  ```
- **球员 NTC (no-trade clause)**:
  ```
  ⚠ {Player} 持有不可交易条款, 无法被送出.
  ```
- **球员有 trade kicker**:
  ```
  ℹ {Player} 有交易激励金, 该交易会触发 ${kicker}M 加薪 (按余年比例).
  ```
- **roster size 违规** (低于 13 或高于 18 含 two-way):
  ```
  ⚠ 交易后阵容人数 {N}, 不在 13-18 范围.
  ```

通过时不显示 — 让 verdict bar 干净.

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` — `render_verdict_bar(...)`. Reads from `nba3k_trade::cba::validate(...)` and translates each `CbaError` variant into the human paragraph.
- `crates/nba3k-core/src/i18n.rs` + tables — one localized template per warning type:
  - `T::TradesWarnSalaryMatch` — params: actual %, diff $M
  - `T::TradesWarnHardCap` — params: net $M, remaining $M
  - `T::TradesWarnNTC` — param: player name (untranslated)
  - `T::TradesNoteTradeKicker` — param: kicker $M
  - `T::TradesWarnRosterSize` — param: count
  - `T::TradesVerdictSent`, `T::TradesVerdictReceived`, `T::TradesVerdictDelta`
  - `T::TradesVerdictPrompt` — "选 ↑↓ Space, 满意后按 Enter 提交"

**Drop**:
- Old `T::TradesInsufficientValue` fixed phrase — replaced with engine-driven dialog in T35.

**Acceptance**:
- Add a player to send → salary line updates instantly.
- Force a salary mismatch → red warning explains the % rule and how much to fix.
- Add a player with NTC → warning shows their name.
- Trades that pass CBA show no warnings (clean bar).

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 299.

**Commit**: `M30-T34: trade verdict salary + plain-language CBA warnings`.

---

## T35 — GM dialog post-submit (natural language, no percentage)

**Status**: `[x]` → codex: implemented post-submit GM dialog and removed acceptance probability from trade UI surfaces.

**Goal**: After user presses Enter to propose, instead of an "estimated acceptance: 42%" output, show a Basketball-GM-style quoted dialog from the AI GM. No percentage anywhere.

**Mapping** (post-submit only — pre-submit shows nothing about acceptance):

| Engine outcome | GM dialog template |
|---|---|
| Accept | `{TEAM} GM: "成交, 合作愉快."` / `Nice dealing with you.` |
| Counter (high quality) | `{TEAM} GM: "差不多, 但我这边觉得你还得加 {player}."` (use engine's actual counter chain) |
| Counter (low quality) | `{TEAM} GM: "你给的太轻了, 至少得加上 {player} + {player2} 我才考虑."` |
| Reject (insufficient value) | `{TEAM} GM: "差远了, 别浪费时间."` / `Close, but not quite good enough.` |
| Reject (CBA violation) | `{TEAM} GM: "想法不错, 但工资帽这关过不去."` |
| Reject (untouchable / NTC) | `{TEAM} GM: "{player} 不在交易考虑范围内."` |
| Reject (badFaith / OutOfRoundCap) | `{TEAM} GM: "我们暂时不想再谈这笔."` |

**Where rendered**:
- Inside the verdict bar (T34). Pre-submit: only salary + CBA warnings. Post-submit: GM dialog appended below as a quoted blockquote (theme.accent_style for the quote, theme.text for the GM name prefix).
- Dialog stays visible until next selection change (which re-clears it back to "preview" mode). Counter-offer also auto-loads the AI's counter into the right pane so user sees what the AI proposed.

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` — bind to existing `commands::dispatch(Command::Trade(Propose...))` path, capture the verdict, format the GM dialog. Pull engine outcome via existing `negotiate::step` / `evaluate::evaluate` return types.
- `crates/nba3k-core/src/i18n.rs` + tables — one key per template above:
  - `T::TradesGmAccept` (param: team abbrev)
  - `T::TradesGmCounterMild` (params: team, counter player names)
  - `T::TradesGmCounterDemand`
  - `T::TradesGmRejectInsufficient`
  - `T::TradesGmRejectCba`
  - `T::TradesGmRejectUntouchable` (param: player name)
  - `T::TradesGmRejectBadFaith`

**Drop**:
- Any `format!("{} %", probability * 100.0)` style strings.

**Acceptance**:
- Submit a fair trade → GM line `ATL GM: "成交, 合作愉快."` AND backend trade applies (players move).
- Submit a lopsided trade → GM line `ATL GM: "差远了, 别浪费时间."`.
- Submit a CBA-violating trade → engine validation kicks in BEFORE the GM speaks; verdict bar shows the CBA warning (T34) and GM line `想法不错, 但工资帽这关过不去.`
- Counter chain → AI's counter players auto-populate the right pane; user can iterate Space + Enter again.
- Nowhere does any percentage show.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 299.
- Manual: 4 distinct trade submissions matching 4 different outcome categories.

**Commit**: `M30-T35: trade gm dialog natural language`.

---

## T36 — Settings: god mode toggle (persisted)

**Status**: `[x]` → codex: implemented persisted Settings god-mode toggle.

**Goal**: Add a god-mode toggle to the Settings screen, persisted across sessions like the language setting.

**Spec**:
- New picker row in Settings screen below the existing language picker:
  ```
  god 模式  [关] / [开]
  ```
  Or two rows. Tab/↑↓ navigates between rows; Space or Enter toggles the value of the focused row.
- Persistence: `nba3k_store::Store::write_setting("god_mode", "on" | "off")`. Read at TUI launch (same path as language). Fallback to file-based config when no save.
- New `tui.god_mode: bool` field on `TuiApp`.
- When toggled mid-session, set `app.force_god` (existing flag from M3) so trade engine bypasses CBA + always accepts. Persist to settings.

**Files**:
- `crates/nba3k-cli/src/tui/screens/settings.rs` — extend the picker UI with a second row (or two-section layout).
- `crates/nba3k-cli/src/tui/mod.rs` — `TuiApp` adds `pub god_mode: bool`. `run()` reads from store/config. Toggle handler updates `app.force_god`.
- `crates/nba3k-cli/src/config.rs` — add `read_god_mode()` / `write_god_mode(bool)` paralleling existing `read_lang` / `write_lang`.
- `crates/nba3k-store/src/store.rs` — `read_setting("god_mode")` already exists; just a new key.
- `crates/nba3k-core/src/i18n.rs` + tables — `T::SettingsGodMode`, `T::SettingsOn`, `T::SettingsOff`.

**Acceptance**:
- Settings screen → second row "god 模式" → toggle → instant effect.
- Quit + relaunch → toggle state preserved.
- When god mode on, trade engine accepts everything (existing M3 behavior).

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 299.

**Commit**: `M30-T36: settings god mode toggle`.

---

## T37 — Force Trade button in builder (gated to god mode)

**Status**: `[x]` → codex: implemented god-mode-gated force trade chip and force propose path.

**Goal**: When god mode is ON, the builder's top bar shows an additional `[F] 强制成交` chip. Pressing F submits the trade with engine override, bypassing all checks.

**Spec**:
- Top-bar render in T33 — if `tui.god_mode == true`, append `[F] 强制成交` (or `[F] Force Trade`) chip. Otherwise hide the chip entirely.
- `F` keypress (only when god mode):
  - Equivalent to Enter but with `force = true` flag passed through dispatch.
  - Calls `commands::dispatch(Command::Trade(Propose { ..., force: true }))`. Extend `TradeAction::Propose` with optional `force: bool` field (defaults false). Negotiate / evaluate already short-circuits when force is true.
- After force submit, GM dialog still shows but message becomes:
  ```
  {TEAM} GM (被迫): "好吧, 这交易我们接受."
  ```

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` — top-bar render + F handler.
- `crates/nba3k-cli/src/cli.rs` — extend `TradeAction::Propose` with `force: bool` if not present.
- `crates/nba3k-cli/src/commands.rs` — propagate `force` to engine.
- `crates/nba3k-core/src/i18n.rs` + tables — `T::TradesForceTradeChip`, `T::TradesGodAcceptDialog`.

**Acceptance**:
- god mode off → no F chip; F key does nothing in builder.
- god mode on → F chip visible; F key force-submits any trade regardless of CBA / value.
- Force-submit always results in Accept verdict + players move.

**Verification**:
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 299.

**Commit**: `M30-T37: force trade button in builder`.

---

## T38 — (Deferred to M31) Draft picks in trade

**Status**: `[!]` deferred — record only.

**Goal**: Allow draft picks (V005 `draft_picks` table) to be sent/received in a trade. Currently the trade engine has scaffolding for picks (M3 `assets_by_team` includes pick IDs) but the TUI builder doesn't expose them.

**Why deferred**:
- Need to verify `evaluate::evaluate` + `cba::validate` + `apply_accepted_trade` all handle picks end-to-end. Some paths may be stubbed.
- TUI work is straightforward (T33 already reserves a `─ 选秀权 ─` section per side); just needs the data + key wiring.
- Splitting M30 keeps it shippable now.

**M31 scope** (for later):
- Audit pick-handling in `nba3k-trade` library.
- Wire `─ 选秀权 ─` section in T33 layout: read user/target team's owned future picks from `Store::picks_for_team(team_id)`, render rows like `2027 1st  (CHI 自有)` or `2028 2nd  (经 NYK)`.
- Update salary/value totals to factor pick value.
- Add `T::TradesPickFirstRound`, `T::TradesPickSecondRound`, `T::TradesPickProtected`, etc.
- New tests covering pick swaps end-to-end.

**Reminder for main agent**: when M30 ships, open M31 with this scope. Don't lose the section-divider stub in T33.

---

## Coordination protocol

- Wave order suggested:
  1. **T36** Settings god mode toggle (foundation; doesn't touch builder).
  2. **T32** Step 1 team picker.
  3. **T33** Step 2 builder body (largest piece; depends on T32 having selected a target).
  4. **T34** Verdict bar warnings.
  5. **T35** GM dialog.
  6. **T37** Force Trade button (depends on T36 + T33).
- Codex flips `[ ]` → `[~]` → `[x]` per task with `→ codex: ...` notes.
- Commit format: `M30-T<N>: <one-line summary>`.
- Blocked: `[!]` + reason.
- New T enum keys synced across all 3 i18n tables.
- Player names + team abbrevs + team full names stay English (data, not chrome).

## Resolved decisions (2026-04-29)

- Step 1 picker: left team list / right roster preview.
- Step 2 builder: master-detail; top bar / two side panels / bottom verdict bar.
- Player row 6 columns: name(16) pos(2) age(2) OVR(2) salary(7) years(3). NO role column.
- Picks: separate section divider in each side panel; M30 shows section header with "M31 待加" placeholder, M31 wires the data + UI.
- Esc behavior: Esc once → Step 1 (re-pick team); Esc twice → Trades menu.
- CBA term: replaced with plain-language warnings; never write `CBA` acronym in UI.
- Acceptance: NO percentage anywhere; natural-language GM dialog post-submit only.
- god mode: Settings toggle persisted across saves; Force Trade button in builder shown only when god on.

---

# M30 follow-up — T39 / T40 / T41

User-tested release (2026-04-29) — 3 polish items.

## T39 — Step 1 roster preview: unicode-safe columns + `y` suffix on years

**Status**: `[x]` → codex: fixed roster preview unicode-safe columns and year suffix alignment; tests passed.

**Goal**: image #28 — `Luka Dončić` / `Moses Brown` / `Daniel Gafford` rows misalign in the OVR / salary / years columns due to byte-pad on names with diacritics + no right-pad on salary + missing `y` suffix on years.

**Fix**:
- Use `pad_display(&player.name, 18)` (T14 helper) instead of byte-pad.
- Salary: `format!("{:>7}", money)` so `$5.3M` aligns with `$27.1M` (7-col right-aligned).
- Years: `format!("{:>3}", format!("{}y", years))` so `4y` / `3y` align as 2-char + space.
- Position label: `format!("{:<2}", pos)` so `C` matches `PF` (already done elsewhere).

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` — wherever the Step 1 roster preview row is built (likely `draw_roster_preview` or inline in `draw_pick_team`).

**Acceptance**: image #28 columns line up; OVR / $ / 年限 right-aligned; names with `ć` / `ü` don't shift the row.

**Commit**: `M30-T39: step 1 roster preview column alignment`.

---

## T40 — Step 2 player rows: `y` suffix on years column

**Status**: `[x]` → codex: fixed Step 2 player row unicode-safe name padding and year suffix alignment; tests passed.

**Goal**: image #29 — trailing `2` / `3` / `5` after salary look like random numbers because the format dropped the `y` suffix the spec called for.

**Fix**:
- Both panels (`我方送出` + `对方接收`): change `format!("{:>3}", years)` to `format!("{:>3}", format!("{}y", years))`.
- Same for "expiring contract" rows: show `"—"` (em dash) instead of `0y`.
- While we're here, double-check `pad_display` is used on the name column in Step 2 too (same diacritic concern as Step 1).

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` — Step 2 row formatter.

**Acceptance**:
- LaVine row reads `Zach LaVine    SF 31 83  $13.9M  —` (no contract year on expiring) or `Zach LaVine    SF 31 83  $13.9M  1y` (1 year left).
- All years render with `y` suffix; no bare numbers.

**Commit**: `M30-T40: step 2 player years suffix and unicode pad`.

---

## T41 — Verdict bar: ✓ when CBA passes + specific GM reject per CbaError

**Status**: `[x]` → codex: added cap-pass marker and CBA-variant-specific GM reject lines; tests passed.

**Goal**: image #30 shows two issues.

(a) Trade was rejected for **roster size** (19 > 18), but GM said `想法不错, 但工资帽这关过不去` — that's the salary-cap message, not the roster-size reason. The `T::TradesGmRejectCba` template is too coarse — currently catches ALL CBA errors with the same line.

(b) When the salary match / cap rules DO pass, there's no positive signal. User wants a green ✓ next to the salary line so they know they're safe from a cap perspective (even if other warnings exist).

**Fix (a)** — split GM reject template per `CbaError` variant:

| CbaError variant | New i18n key | Template |
|---|---|---|
| `SalaryMatch` (over-cap, fails 125% rule) | `T::TradesGmRejectSalaryMatch` | `{TEAM} GM: "工资帽这关过不去, 进薪超出 125% 限制."` |
| `HardCap` (touches first/second apron) | `T::TradesGmRejectHardCap` | `{TEAM} GM: "我们触线了, 这交易没法做."` |
| `RosterSize` (13-18 violation) | `T::TradesGmRejectRoster` | `{TEAM} GM: "想法不错, 但交易后阵容人数不合规."` |
| `NoTradeClause` | reuse `T::TradesGmRejectUntouchable` | (already player-named) |
| `Other` / unmapped | keep `T::TradesGmRejectCba` as catch-all fallback | `{TEAM} GM: "规则上有问题, 这笔做不成."` |

In `crates/nba3k-cli/src/tui/screens/trades.rs::format_gm_dialog` (or wherever the mapping lives), match on the actual `CbaError` variant returned from `validate(...)` rather than treating "any CBA error" as one bucket.

**Fix (b)** — when CBA `validate(...)` returns Ok (no salary mismatch / hard cap / NTC issues), render a green `✓` glyph after the salary delta line:

```
送 $28.5M  /  收 $29.9M  /  净 +$1.5M  ✓ 工资帽通过
```

The ✓ uses `Style::default().fg(Color::Green)` (or `theme.accent_style()` if green isn't the theme accent — pick green explicitly so it reads as "safe").

If CBA fails, NO ✓; the warning panel below explains.

Roster-size and other non-financial warnings still appear in the warning panel as before, even when ✓ is shown for cap.

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` — `render_verdict_bar` adds the green ✓ branch + refactor `format_gm_dialog` to be CbaError-aware.
- `crates/nba3k-core/src/i18n.rs` + `i18n_en.rs` + `i18n_zh.rs` — add `T::TradesGmRejectSalaryMatch`, `T::TradesGmRejectHardCap`, `T::TradesGmRejectRoster`, `T::TradesVerdictCapPass` ("工资帽通过" / "Cap rules OK").

**Acceptance**:
- Same trade as image #30 (roster 19, $1.5M net) → GM line says `想法不错, 但交易后阵容人数不合规.` (NOT "工资帽").
- Salary-match violation (over-cap, 125% fail) → GM line says `工资帽这关过不去, 进薪超出 125% 限制.`.
- Trade where caps pass but roster size violates → green `✓ 工资帽通过` on salary line + roster-size warning below + roster-specific GM line.
- Clean trade (passes everything) → green `✓ 工资帽通过` on salary line + no warnings + GM accept line after submit.

**Commit**: `M30-T41: verdict cap-pass marker + cba-aware gm reject lines`.

---

## Wave order

T39 / T40 are pure formatting fixes, can do in any order, parallel-safe.
T41 touches engine-mapping logic + new i18n keys — do after T39/T40 to avoid merge conflicts in trades.rs.

## Resolved decisions (2026-04-29 follow-up)

- T39 alignment via `pad_display` + right-aligned salary (7 cols) + years with `y` suffix (3 cols).
- T40 same fixes apply to Step 2 row formatter; both side panels.
- T41 GM reject message must match the actual `CbaError` variant; green `✓` shows when cap rules pass even if other warnings exist.

---

## T42 — Roster-size warning specifies which team violates

**Status**: `[x]` → codex: roster-size warnings now name the offending team abbrev and render one line per violating team.

**Goal**: image #33 — `⚠ 交易后阵容人数不在 13-18 范围. 当前 21.` doesn't tell user which team has the bad count. The `CbaError::RosterSize` variant from `validate(...)` already carries the offending team id (or both if both violate); the i18n template just throws away the team info.

**Fix**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` warning rendering for roster-size: read team abbrev (`Store::team_abbrev`) of the offending team, plug into the template. If both teams violate (rare but possible), render two lines.
- `crates/nba3k-core/src/i18n.rs` + tables — change `T::TradesWarnRosterSize` template to take 2 params (team_abbrev, count):
  - ZH: `⚠ {team} 交易后阵容人数 {count}, 不在 13-18 范围.`
  - EN: `⚠ {team} would have {count} players post-trade — outside the 13-18 range.`

**Acceptance**:
- Trade in image #33 → warning reads `⚠ HOU 交易后阵容人数 21, 不在 13-18 范围.` (or whichever team is the violator).
- If both teams overflow / underflow → 2 warning lines, one per team.

**Commit**: `M30-T42: roster size warning names offending team`.

---

## T43 — Swap panels: target team LEFT, my team RIGHT

**Status**: `[x]` → codex: Step 2 now shows target roster on the left, my roster on the right, with target-first focus order.

**Goal**: User wants the TARGET team's roster on the LEFT panel, MY team on the RIGHT. Currently it's reversed (我方送出 left, 对方接收 right). Reasoning per user: read left-to-right as "from THEM → into MY team".

**Spec**:
- Step 2 layout becomes:
  ```
  ┌─ 对方送出 (HOU) ────────┬─ 我方送出 (CHI) ────────┐
  │ ─ 球员 ─                │ ─ 球员 ─                │
  │ ✓ Şengün C 24 87 $29.9M │ ✓ LaVine SF 31 83 $13.9M│
  │   ...                   │   ...                   │
  ```
- Top bar copy stays factual — `目标 HOU · 我方 CHI` order remains semantic; no swap there.
- Verdict bar copy: keep `送 $X / 收 $Y / 净 ±Z`. "送" still refers to MY team's outgoing salary, "收" still my team's incoming. Numbers don't change, only the panel order swaps.
- Tab/Shift-Tab focus order: now goes target panel first (left) then my panel (right) — natural left-to-right.
- All keys (Space select / m mode / etc) preserved on both panels.

**Files**:
- `crates/nba3k-cli/src/tui/screens/trades.rs` — swap the two `Layout::default().split(...)` calls in Step 2 body draw, swap the focus enum order, swap which side the user-team list / target-team list go into.
- `crates/nba3k-core/src/i18n.rs` + tables — re-examine `T::TradesSendList` / `T::TradesReceiveList` panel headers:
  - LEFT panel header was "我方送出"; change to "对方送出 (target sends to us)" — or use existing `T::TradesReceiveList` ("对方接收") inverted to "对方送出" / "{TEAM} sends".
  - RIGHT panel header was "对方接收"; change to "我方送出" / "我方 sends".
  - Easiest: rename the two existing keys to reflect new positions, OR keep keys neutral and swap usage at call site. Pick the call-site swap so i18n stays simple.

**Acceptance**:
- Open Step 2 with target=HOU, mine=CHI → LEFT panel has HOU roster, RIGHT panel has CHI roster.
- Tab moves focus left → right (target → mine).
- Selections still work; verdict salary lines still calculate "送" from my team.

**Commit**: `M30-T43: swap trade panels target left mine right`.

---

## Wave order (M30 follow-up part 2)

T42 / T43 independent. Codex picks any order.

## Resolved decisions (2026-04-29 part 2)

- T42 roster-size warning includes offending team abbrev.
- T43 panels swapped: target LEFT, mine RIGHT. Top bar copy unchanged. Verdict salary semantics unchanged.

---

## T44 — Scraper dedup + per-team cap (root-cause fix for >18 rosters)

**Status**: `[x]` → codex: implemented scraper-side primary-team dedup, top-15 per-team cap, tighter assertions, duplicate-name guard, and unit tests.

**Problem**: Each team in the seed has 20-22 players because BBRef's per-team page lists EVERY player who appeared on the team that season — including mid-season-traded-out players, 10-day signees, and waived players. The scraper concatenates all 30 team pages → duplicates (e.g. Mitchell on both DAL and CLE) + bloated rosters → CBA's 13-18 post-trade roster bound is impossible to satisfy.

**Background context** (from M19.1 memory note):
> Bumped scrape-assertion bounds (max_players 600→720, max_per_team 20→30) since prior-season pulls duplicate rows for traded players.

That widening was a workaround. T44 fixes the data instead.

**Approach** (one-pass, no extra HTTP calls):

After Stage 1 in `crates/nba3k-scrape/src/main.rs` collects `all_players` (30 team pages concatenated), insert a dedup + cap pass BEFORE rating + insert. Two filters:

1. **Per-team cap** — within each team, keep top 15 by `minutes_per_game` (then `games` desc as tiebreak). Drops 10-day signees and rotation fringe.
2. **Cross-team dedup** — group all remaining players by normalized name. If a player name appears on 2+ teams, keep only the entry with highest `minutes_per_game * games` (= total minutes that season — that's their "primary team"). The other entries get dropped entirely.

After both filters: each team has ≤ 15 players; each player has exactly one team.

Tighten `data/free_agents_2025_26.toml` curated list (M27 T22) to fill the remaining gap of 30-50 unsigned vets.

**Files**:
- `crates/nba3k-scrape/src/main.rs`:
  - After `all_players` is built (line ~170), call new `dedup_and_cap(&mut all_players, &mut team_player_offsets) -> ()`.
  - Helper iterates teams, sorts each team's slice by `mpg desc, games desc`, truncates to top 15 (configurable const `MAX_PER_TEAM = 15`).
  - Then groups remaining by normalized name (reuse `names_match` logic from `bbref.rs`); for each multi-team duplicate, retains only the row with max `mpg * games`. Other rows dropped from `all_players`. Recompute `team_player_offsets` to point to the new contiguous slices.
- `crates/nba3k-scrape/src/assertions.rs`:
  - Tighten bounds: `max_players: 720 → 480`, `max_per_team: 30 → 18`. Restores the original "real NBA" sanity check.
- `crates/nba3k-scrape/src/sources/bbref.rs`:
  - Optional: pre-filter inside `parse_team_page` so per-team page already returns ≤ 18 (avoids carrying 22 then trimming). Probably cleaner to do at main.rs level — keeps bbref.rs raw.

**Edge cases**:
- A player with 0 mpg (didn't play) — drop entirely. Don't include in seed.
- Tied tiebreak (rare) — sort by name asc as final tiebreak for determinism.
- Synthetic roster fallback (offline) — `synthetic_roster` already returns 15. No-op for it.

**Acceptance**:
- After `cargo run -p nba3k-scrape` regenerates `data/seed_2025_26.sqlite`:
  - `sqlite3 data/seed_2025_26.sqlite "SELECT team_id, COUNT(*) FROM players WHERE team_id IS NOT NULL GROUP BY team_id;"` shows each team with 13-15 players.
  - `sqlite3 data/seed_2025_26.sqlite "SELECT name, COUNT(*) FROM players GROUP BY name HAVING COUNT(*) > 1;"` returns 0 rows.
- Fresh `nba3k new --team BOS --save x.db` → roster size = 14-15.
- Trade builder: post-trade roster size stays in 13-18 for any reasonable swap.
- `cargo build --workspace` clean.
- `cargo test --workspace` ≥ 299 (the dedup logic gets unit tests in main.rs or new module).

**Not in scope**:
- Migrating existing saves — user explicitly chose pure A. Existing saves keep their bloat; user starts a new save for the fix.
- Touching trade engine CBA rules — they were always correct; data was wrong.

**Commit**: `M30-T44: scraper dedup + per-team cap to 15`.

---

## Resolved decisions (2026-04-29 part 3)

- T44 fixes roster bloat at SCRAPER level: per-team top-15 by mpg + cross-team dedup keeping max-minutes team. Bounds tightened to 480/18.
- User starts new save after re-scraping; existing save not migrated (per user choice).
