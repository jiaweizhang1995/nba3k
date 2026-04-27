# M19 — TUI dashboard (ratatui)

## Status: ✅ Done (2026-04-26)

- Deps: `ratatui = "0.29"` + `crossterm = "0.28"` added to `nba3k-cli/Cargo.toml`.
- `Command::Tui` variant in `cli.rs`; dispatch + `cmd_tui` stub in `commands.rs`; `mod tui;` in `main.rs`.
- New file `crates/nba3k-cli/src/tui.rs`. Read-only — never calls mutation methods.
- 5 tabs: Status / Roster / Standings (East+West side-by-side) / Trades (open inbox + recent 20) / News (50).
- Hotkeys: `1`–`5`, `Tab` / `Shift-Tab` cycle, `↑↓` scroll, `PgUp`/`PgDn` ±10, `Home` → top, `q` / `Esc` quits.
- Caches per-tab data on first visit; payroll cached once for header.
- Empty-save path prints `no save loaded — pass --save <path> first` and exits 0 before entering alt screen.
- `< 80` cols renders `Resize terminal to ≥ 80 columns` placeholder.
- Verification: `cargo build --release -p nba3k-cli` clean (0 errors). `cargo test --workspace` 273 passed (no regression vs M18). `nba3k tui` (no save) prints message + exit 0. `nba3k --help` lists `tui`. PTY-driven visual check blocked by sandbox; left for user.

## Onboarding for the next AI

You are the sole agent on this. Project root: `/Users/jimmymacmini/Desktop/claude-code-project/nba3k-claude`. Read `phases/PHASES.md` first to understand what's been built (18 phases done, M1-M18). The codebase is a Rust workspace with 8 crates; binary is `nba3k`. The CLI is REPL + argv subcommands today; you're adding a full-screen TUI.

## Goal

Add a **read-only TUI dashboard** as a new CLI subcommand `nba3k --save x.db tui`. Game state mutations (sim-day, trade, sign, retire, etc.) stay in the existing REPL/argv interface — the TUI is a visualization layer. v1 is read-only because mutating from a TUI requires a confirmation flow that's too much for one phase. Mutation hooks are a v2 follow-on.

## Why read-only first

The original plan (`~/.claude/plans/bubbly-roaming-thacker.md`) deferred TUI because "TUI is hard to assert against in tests". We accept that constraint — smoke-build only, manual verify. Don't try to write headless TUI integration tests.

## Stack

- `ratatui` (latest stable) + `crossterm` for backend.
- Reuse existing `Store` APIs directly — DO NOT shell out to the binary.
- Single-binary: TUI lives inside `nba3k-cli`; activate via `Command::Tui` subcommand.

## Pre-locked CLI

If the stub isn't already there, add it to `crates/nba3k-cli/src/cli.rs` AND wire dispatch in `crates/nba3k-cli/src/commands.rs`. The stub should bail before you fill it in. Then fill the body.

```
Command::Tui   → cmd_tui   (calls tui::run(app))
```

## Layout

5 tabs, switch with `1`-`5` or `Tab`/`Shift+Tab`. `q` to quit.

```
╭─ nba3k 1.0.0 — BOS — season 2025-26 (Regular, day 30) — payroll $167M ───╮
│  [1]Status  [2]Roster  [3]Standings  [4]Trades  [5]News                    │
╰────────────────────────────────────────────────────────────────────────────╯
{tab content fills the rest}
╭─ q quit · Tab/Shift-Tab cycle · ↑↓ scroll ─────────────────────────────────╯
```

Tab bodies (use `Store` reads — every method already exists):

1. **Status** — `Store::load_season_state` + `count_teams/players/schedule_total/unplayed`. Render as a list.
2. **Roster** — `Store::roster_for_team(user_team)` sorted desc by overall. Show ID-padded table with NAME / POS / AGE / OVR / POT / ROLE / MORAL columns. ↑↓ scrolls.
3. **Standings** — `Store::read_standings(season)`. Two columns side-by-side (East / West) ranked by record.
4. **Trades** — `Store::list_trade_chains(season)` + `read_open_chains_targeting(season, user_team)`. Top half = your open inbox; bottom half = recent completed.
5. **News** — `Store::recent_news(50)`. Scrollable list `[kind] D### headline`.

Header bar:
- Title + team + season label + phase + day + payroll (call `Store::team_salary(user_team, current_season)` once on tab change, cache).

Footer bar:
- Static hotkey hints.

## Files you own

- `crates/nba3k-cli/Cargo.toml` — add `ratatui` and `crossterm` to `[dependencies]`. Pin versions you've tested against.
- `crates/nba3k-cli/src/tui.rs` (new) — `pub fn run(app: &mut AppState) -> Result<()>` that runs the event loop.
- `crates/nba3k-cli/src/commands.rs` — fill `cmd_tui` stub: just calls `tui::run(app)`.
- `crates/nba3k-cli/src/cli.rs` — add `Tui` variant if missing.

DO NOT touch other crates' main code.

## Implementation outline

```rust
// tui.rs
pub fn run(app: &mut AppState) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = TuiState { tab: Tab::Status, scroll: 0, /* cached data */ };
    loop {
        terminal.draw(|f| draw(f, app, &state))?;
        if let Event::Key(k) = event::read()? {
            match k.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('1') => state.tab = Tab::Status,
                KeyCode::Char('2') => state.tab = Tab::Roster,
                // ...
                KeyCode::Tab => state.tab = state.tab.next(),
                KeyCode::BackTab => state.tab = state.tab.prev(),
                KeyCode::Up => state.scroll = state.scroll.saturating_sub(1),
                KeyCode::Down => state.scroll += 1,
                _ => {}
            }
        }
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
```

Cache rosters/standings on tab switch — don't re-query on every frame. Frame is 60Hz; SQLite reads are fine but wasteful.

## Constraints

- Read-only. Never call `Store::upsert_*`, `record_*`, etc.
- Single tab at a time. No popups in v1. No mouse.
- 80×24 minimum. Degrade gracefully when narrower.
- If `--save` not set or no `season_state`, render a single-screen "no save loaded" message and exit on any key.

## Testing

- `cargo build --release -p nba3k-cli` must succeed. That's the only automated check.
- Manual smoke: `./target/release/nba3k --save /tmp/m18.db tui` — verify each tab renders, q quits cleanly, terminal is restored (no garbled state after exit).

## Out of scope

- Mutations from the TUI (no signing FAs, no proposing trades, no setting roles).
- Mouse support.
- Color theming.
- Help overlay.
- Mid-frame log streaming.

If you implement well past these constraints, fine — but don't sacrifice the "ship today" timeline. v1.0 release is gated on this phase + a README pass.

## Reference

- Existing read APIs you'll consume:
  - `crates/nba3k-store/src/store.rs` — every read method (look for `read_*` / `list_*` / `count_*`).
  - `crates/nba3k-cli/src/commands.rs` — patterns for resolving user_team via `meta.user_team` or `state.user_team`.
- Existing CLI surface for context: `./target/release/nba3k --help`. The TUI tabs mirror those subcommands: status/roster/standings/trade list/news.

When done:
1. `cargo test --workspace` still green (no regressions).
2. Manual smoke confirms 5 tabs work + `q` cleanly exits.
3. Update `phases/PHASES.md` row M19 to ✅.
4. Tell the user the TUI ships; call out that mutations stay in REPL/argv.
