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

- The TUI compiles against the new CBA variants, but full pick-selection
  subcolumns in the trade-builder and a dedicated roster Picks tab remain a
  follow-up. CLI and backend behavior are implemented.
- Spotrac swap-right prose is stored verbatim as protection text; v1 does not
  model higher/lower-of swap mechanics.
