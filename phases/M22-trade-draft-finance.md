# M22 — Trades + Draft + Finance + polish

**Status:** Done 2026-04-27. 3 parallel screen workers plus main-agent integration/review.

## Goal

Replace the final M20 TUI stubs with playable screens for Trades, Draft, and Finance, then finish the 7-menu TV-mode surface with contextual help and documentation.

## Worker ownership

| Worker | Owns | Notes |
|--------|------|-------|
| `nba3k-m22-trades-screen` | `crates/nba3k-cli/src/tui/screens/trades.rs` | Inbox, proposal chains, 2-team builder, rumors |
| `nba3k-m22-draft-screen` | `crates/nba3k-cli/src/tui/screens/draft.rs` | Prospect board, draft order, scout/pick/auto actions |
| `nba3k-m22-finance-screen` | `crates/nba3k-cli/src/tui/screens/finance.rs` | Cap summary, payroll bar, contracts, extensions |
| main agent | TUI shell wiring, help overlay, docs, final review | No screen-file ownership |

## Outcome

- `Trades`: 4 tabs for incoming offers, user proposal chains, a 2-team builder, and rumors. Open offers support accept/reject/counter through dispatch; builder submits `TradeAction::Propose`; 3-team builder is intentionally disabled in-screen.
- `Draft`: Board / Order tabs, top-60 prospect board with scouting fog, local draft-order view, scout/pick/auto actions gated to Playoffs/OffSeason.
- `Finance`: cap/tax/apron/minimum summary, payroll gauge, sortable contracts table, and extension modal.
- Shell polish: all three screens wired into the 7-menu shell, menu copy updated, global `?` context help overlay added.
- Docs: README TUI table updated and phase tracker row added.

## Implementation contract

- TUI keeps the 7-menu cut policy: Home, Roster, Rotation, Trades, Draft, Finance, Calendar only.
- CLI/REPL command surface stays intact.
- All TUI mutations go through `commands::dispatch(app, Command)` wrapped in `with_silenced_io`.
- Screens follow the M21 cache/modal/action-bar pattern and expose `render`, `handle_key`, and `invalidate`.
- Any roster-changing action invalidates Home, Roster, Rotation, and relevant M22 screen caches.

## Acceptance gates

1. `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --workspace`
2. `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo test --workspace`
3. TUI keyboard smoke:
   - New game BOS / Standard / 2026 / seed 42.
   - Home shows mandate.
   - Rotation sets 5 starters.
   - Finance shows payroll/cap/apron summary.
   - Calendar sims forward.
   - Trades builder can submit a proposal or shows a clear engine rejection.
   - Draft screen shows board/order and blocks pick outside active phase.
   - Roster FA tab still signs/cuts after M22 screens invalidate caches.
4. README TUI section finalized.

## Verification

- `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo check --workspace` passed during integration.
- `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --workspace` passed.
- `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo test --workspace` passed.
- PTY smoke opened and exited Trades + help overlay, Draft board inactive state, and Finance cap/contracts screen without panic or terminal restore issues.

## Out of scope

- New DB migrations or new crates.
- Full gamepad support.
- Full 3-team drag-and-drop trade builder if it would destabilize the 2-team flow.
- Adding non-7-menu TUI entry points for compare, records, HOF, coach, or dev tools.
