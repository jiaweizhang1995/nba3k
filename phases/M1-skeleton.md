# M1 — Skeleton + Persistence ✅

**Status**: Completed 2026-04-25
**Owner**: orchestrator (no team — single-thread bootstrap)

## Goal

Compile a Cargo workspace with all 7 crates, define the core domain model, stand up the SQLite persistence layer with refinery migrations, and ship a CLI that can `new` / `load` / `save` / `status` / `quit` against a save file.

## Acceptance

```bash
nba3k --save x.db new --team BOS --mode standard --season 2026 --seed 42
nba3k --save x.db status              # text output
nba3k --save x.db status --json       # JSON output
printf 'status\nsave\nquit\n' | nba3k --save x.db   # piped REPL
nba3k --save x.db --script file.txt   # script mode
```

All paths verified ✅ (2026-04-25).

## What landed

- **Workspace** (`Cargo.toml`): 7 members, edition 2021, MSRV 1.80, workspace-level dep pinning, release LTO, dev-fast.
- **`nba3k-core`** (zero I/O): `PlayerId`/`TeamId`/`SeasonId`/etc id newtypes, `Cents` (integer money), `Player`+`Ratings`+`InjuryStatus`, `Team`+`Conference`+`Division`, `Contract`+`ContractYear`+`BirdRights`, `DraftPick`+`Protection`+`DraftProspect`, `TradeOffer`+`TradeAssets`+`Verdict`+`RejectReason`+`NegotiationState`, `SeasonState`+`SeasonPhase`+`GameMode`, `BoxScore`+`PlayerLine`+`GameResult`, `GMArchetype`+`GMTraits`+`GMPersonality` with archetype-seeded defaults. 5 unit tests pass.
- **`nba3k-store`**: `rusqlite` (bundled SQLite) + `refinery` migrations. V001 schema (`meta`, `teams`, `players`, `draft_picks`, `games`, `trade_history`, `standings`, `awards`, `season_state`). `Store` API: `open`, `set_meta`/`get_meta`, `init_metadata`, `save_season_state`/`load_season_state`, `upsert_team`, `find_team_by_abbrev`, `team_abbrev`, `count_teams`/`count_players`. WAL mode + foreign keys.
- **`nba3k-cli`** (`nba3k` bin): `clap` derive with global `--save`/`--script`/`--engine`/`--god`. Subcommands: `new`/`load`/`save`/`status`/`quit` (M1) + stubs for `sim-day`/`sim-to`/`standings`/`roster`/`trade`/`draft` (M2-M5). Same `Command` enum drives argv AND REPL lines via `shlex` + `try_parse_from`. Three execution modes: subcommand argv, script file, piped stdin, interactive (rustyline). `--json` flag on `status`.
- **Placeholder crates**: `nba3k-sim` (Engine trait + TeamSnapshot/GameContext skeletons), `nba3k-trade` (module file stubs), `nba3k-season` (empty lib), `nba3k-scrape` (`nba3k-scrape` bin with arg parsing only).

## Deviations from plan

- None. Architecture matches `~/.claude/plans/bubbly-roaming-thacker.md` 1:1.

## Notes / hand-offs to M2

- `Store::upsert_team` exists but no real teams in DB — M2 scraper populates real 30-team roster.
- `Player`-related Store methods deferred to M2 (don't write speculative API; let scraper drive shape).
- `Engine` trait is in place — M2 just implements `StatisticalEngine`.
- `--engine statistical` flag is parsed but unused; M2 wires it.
- `force_god` field on `AppState` is unused (warning) — M3 will read it.
- Build command: `cargo build` from project root. PATH needs `/opt/homebrew/opt/rustup/bin` (Homebrew rustup is keg-only). Already in `~/.zshrc`.
