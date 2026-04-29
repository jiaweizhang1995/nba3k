# M36 — Draft-Pick Trading System

## Goal

Make draft picks first-class trade assets: seed pick ownership into saves,
import live future-pick obligations from Spotrac, enforce pick-specific CBA
rules, use pick ownership in draft order, and expose picks through the CLI.

## Implemented

- V018 adds `resolved`, `protection_text`, and `protection_history` to
  `draft_picks`, plus a unique `(season, original_team, round)` key.
- Offline `new --offline` seeds a seven-year vanilla horizon
  (`7 * 2 * 30 = 420` rows).
- Live `new` seeds vanilla rows, overlays Spotrac `/nba/draft/future`, and
  falls back to vanilla with `tracing::warn!` on Spotrac failure.
- Optional `data/pick_swaps_overrides.toml` applies manual row overrides after
  either source.
- `picks [--team BOS] [--season YYYY]` lists unresolved picks.
- `trade propose` accepts `--send-picks` and `--receive-picks` tokens in
  `YEAR-R1-ORIGINAL` format.
- Accepted trades transfer pick ownership through `nba3k-store`.
- Draft order/draft sim/season advance use `DraftSlot.current_owner`; resolved
  picks are marked, and the trailing pick horizon is topped up on
  `season-advance`.
- CBA validation blocks picks beyond seven years and Stepien violations.
- TUI surfaces (commit `2f5660a`):
  - Trade builder: Picks subcolumn beside Players in both halves; combined
    cursor walks players first then picks; `Space` toggles selection; chosen
    picks flow through `TradeAssets.picks_out` to CBA validation.
  - Roster screen: new Picks sub-tab (`1`/`2` to switch), columns
    YEAR / RND / VIA / PROTECTION.
  - Draft Order screen: VIA column + uses `draft_picks.current_owner` so
    traded slots render as `Pick #5 — DET via NYK`.
  - Stepien / 7-year CBA error messages no longer leak `TeamId(_)` debug
    formatting.
- Star rating (commit `4131354`): trade builder pick rows render `★★★★☆` from
  a heuristic over round + years-out + structured `Protection` enum + light
  prose scan ("more/least favorable"). Spotrac-flagged "Not Tradable" /
  "FROZEN PICK" picks render as `🔒 frozen`.
- God-mode unlock (commit `bd8deb5`): in god mode the frozen-prose check is
  skipped and every pick gets normal stars, mirroring how god mode bypasses
  CBA validation.

## Verification Log

- `cargo check --workspace` ✅
- `cargo test -p nba3k-store --test draft_picks` ✅
- `cargo test -p nba3k-scrape --test spotrac_parser` ✅
- `cargo test -p nba3k-trade --test cba_misc` ✅
- Offline schema sanity:
  `SELECT COUNT(*), COUNT(DISTINCT season), SUM(original_team != current_owner) FROM draft_picks`
  → `420|7|0` ✅
- Live Spotrac sanity on 2026-04-29:
  `SELECT COUNT(*), COUNT(DISTINCT season), SUM(original_team != current_owner) FROM draft_picks`
  → `420|7|137` ✅
- CLI pick trade smoke:
  `trade propose --from BOS --to LAL --send 'Jrue Holiday' --receive 'Austin Reaves' --send-picks 2027-R1-BOS`
  accepted and transferred the pick owner to LAL ✅
- Stepien smoke:
  `--send-picks 2027-R1-BOS,2028-R1-BOS` rejected with a Stepien violation ✅

## Deviations

- Codex's foundation commit (`9c96ca4`) deferred all TUI surfaces. Those were
  picked up and shipped in `2f5660a` / `4131354` / `bd8deb5` (this main-agent
  session). M36 is now end-to-end complete.
- Spotrac swap-right prose is stored verbatim as protection text; v1 does not
  model higher/lower-of swap mechanics.
- Star rating ignores live standings on purpose so the rating degrades cleanly
  to offline 0-0 saves. A future pass could fold in Spotrac-derived "team
  strength" once we have a reliable signal beyond W-L.
