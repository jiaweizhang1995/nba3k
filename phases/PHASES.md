# nba3k-claude — Phase Tracker

Project status board. Each phase has a dedicated `phases/M{N}-{slug}.md` doc with goals, sub-tasks, agent assignments, and verification artifacts.

| #  | Phase                              | Status      | Started     | Completed   | Bash verification |
|----|------------------------------------|-------------|-------------|-------------|-------------------|
| M1 | [Skeleton + persistence](M1-skeleton.md)         | ✅ Done     | 2026-04-25  | 2026-04-25  | `nba3k new --team BOS --save x.db && nba3k --save x.db status --json` ✅ |
| M2 | [Seed data + sim engine](M2-seed-sim.md)         | ✅ Done     | 2026-04-25  | 2026-04-25  | Full season sim → standings sum = 1230 ✅ (2.8s wall time) |
| M3 | [Trade engine v1 (headline)](M3-trade.md) | ✅ Done | 2026-04-25 | 2026-04-25 | Engine + CBA + negotiation + CLI integration ✅; calibration is polish |
| M4 | [Realism Engine](M4-realism.md)    | ✅ Done     | 2026-04-25  | 2026-04-25  | Luka untradeable ✅, star stat realism (Luka 33/6/15, Jokic 31/12/6) ✅, M4-polish calibration ✅ |
| M5 | [Realism v2 (2K-borrow)](M5-realism-v2.md) | ✅ Done     | 2026-04-25  | 2026-04-26  | 21-attr schema ✓, chemistry ✓, awards ✓, playoffs ✓, season-summary ✓ — progression CLI hook deferred to M6 |
| M6 | Draft + offseason                  | ✅ Done    | 2026-04-26  | 2026-04-26  | `draft board/order/sim/pick`, `season-advance` (progression + auto-draft), schedule re-gen deferred to M7 |
| M7 | Polish + AI initiation + integ test| ✅ Done    | 2026-04-26  | 2026-04-26  | Schedule regen ✓, scripted season1 e2e (4s) ✓, AI deadline trade volume ✓, weighted top-14 lottery ✓, verdict prose ✓ |
| M8 | Realistic seed + age curves        | ✅ Done    | 2026-04-26  | 2026-04-26  | bbref `name_display` fix + birth_date age derivation. Ages 20-42, OVR 47-93. SGA 93, Jokić 92, LeBron 42/78, Flagg 20/83 |
| M9 | Trade evaluator + scheme + inbox   | ✅ Done    | 2026-04-26  | 2026-04-26  | Accept threshold loosened (0.55→0.50), reject threshold (0.20→0.12), per-team schemes by abbrev hash, `messages` GM inbox surfaces trade demands |
| M10| 3-team trades + dynasty + FA + training | ✅ Done | 2026-04-26 | 2026-04-26 | 4 workers parallel: 3-team unanimous-accept trades, career stats per-season+totals, FA pool (V006 migration, sign/cut/list, 18-cap), training camp (focus→cluster, once/season). Possession sim skipped per user. 195 unit + 1 integ green |
| M11| Contracts + retirement + FG calibration | ✅ Done | 2026-04-26 | 2026-04-26 | 3 workers: contract_gen tied to OVR tier, `cap` cmd shows payroll vs cap/tax/aprons, V007 retirement migration (hard@41, conditional@36+, deterministic FNV stochastic), FG calibration .499/.451/.855 (was .887). 207 unit + 1 integ |
| M12| League economy: contracts backfill + HOF + AI FA | ✅ Done | 2026-04-26 | 2026-04-26 | Scraper backfills contracts (BOS payroll $167M, league $3-7B sanity), `hof` cmd ranks retired players by career PTS, AI FA pass during season-advance signs top FAs to teams with cap room. 212 unit + 1 integ |
| M13| League liveness: injuries + news + award race | ✅ Done | 2026-04-26 | 2026-04-26 | Per-game injury rolls (Tatum * INJ 5), V008 news table records trade/sign/cut/retire/draft/award events, mid-season `awards-race` top-5 with vote shares. 221 unit + 1 integ |
| M14| Meta-game: coaching + scouting fog + records | ✅ Done | 2026-04-26 | 2026-04-26 | `Coach::overall()` + hot-seat threshold + `coach show/fire/pool`, V009 `scouted` column hides prospect ratings until `scout <player>` (5/season cap), `records --scope season|career --stat ppg|...` leaderboards. 235 unit + 1 integ |
| M15| Events + history + save mgmt | ✅ Done | 2026-04-26 | 2026-04-26 | V010 all_star table, day-41 trigger compute_all_star (Tatum reserve, Giannis E-starter), `standings --season N` recalls past seasons, `saves list/show/delete --yes`. 245 unit + 1 integ |
| M16| NBA Cup + rumors + compare | ✅ Done | 2026-04-26 | 2026-04-26 | V011 cup_match table, day 30/45/53/55 hooks (groups → 8-team KO → SAS 115-111 TOR final), `rumors --limit N` ranks players by AI-team interest, `compare BOS LAL` side-by-side payroll/top-8/chemistry. 254 unit + 1 integ |
| M17| GM tools: offers + extend + notes | ✅ Done | 2026-04-26 | 2026-04-26 | Daily AI inbox-offer auto-gen + `cmd_offers`, `extend` with morale-adjusted accept/counter/reject (Tatum 4yr/$200M accepted), V012 notes table + `notes add/remove/list` + Notes section in `messages`. Plus LeagueYear future-season fallback (5%/yr extrapolation) so multi-season saves don't panic. 264 unit + 1 integ |
| M18| Narrative: mandate + recap + export | ✅ Done | 2026-04-26 | 2026-04-26 | V013 mandate table auto-gen at season start (BOS: wins=42/develop=85/playoffs), `recap --days N` top-scorer per game, `saves export <path> --to file.json` dumps 16 tables / 1889 rows. 273 unit + 1 integ |
| M19| TUI dashboard (ratatui)      | ✅ Done    | 2026-04-26  | 2026-04-26  | `nba3k --save x.db tui` — read-only ratatui dashboard, 5 tabs (Status/Roster/Standings/Trades/News), `q` exits clean. ratatui 0.29 + crossterm 0.28. 273 unit + 1 integ green. Mutations stay in REPL/argv. |
| M20| Playable TUI shell + 8-menu (TV mode) | ✅ Done | 2026-04-27 | 2026-04-27 | `nba3k tui` — 8-menu shell (Home/Roster/Rotation/Trades/Draft/Finance/Calendar/Settings), Home dashboard (4 panels), Calendar (month grid + 6 sub-tabs), Saves overlay (Ctrl+S), New-game wizard (no-save entry). `--tv` high-contrast preset. `--legacy` falls back to M19 5-tab. tui.rs (1123 LoC) split into module tree (mod.rs + widgets.rs + 8 screens). 3 workers parallel (`nba3k-m20`). 275 unit + 1 integ green. Roster/Rotation/Trades/Draft/Finance show "Coming in M21/M22" stubs. |
| M21| Roster + Rotation Level A (TUI) | ✅ Done | 2026-04-27 | 2026-04-27 | `nba3k tui` Menu → 2/3 — Roster screen (My Roster / FA tabs, sort o/p/a/s, t-train / e-extend / x-cut / R-role, 4-panel Detail modal); Rotation Level A (5 starter slots PG/SG/SF/PF/C, adjacency-filtered picker, c clear / C clear-all). New backend: V014 `team_starters` migration, `Starters` struct in nba3k-core, Store API, build_snapshot hook (user starters override when complete + roster-valid; auto fallback otherwise). 3-worker team `nba3k-m21`. 281 unit + 1 integ green (+6). |
| M22| Trades + Draft + Finance + polish (TUI) | ✅ Done | 2026-04-27 | 2026-04-27 | `nba3k tui` Menu → 4/5/6 — Trades screen (Inbox / My Proposals / Builder / Rumors; a/r/c responses; 2-team builder), Draft screen (Board / Order; scout/pick/auto gated to draft phase), Finance screen (cap/tax/apron/minimum lines, contract table, extensions), plus `?` context help overlay. 3 parallel screen workers + main-agent integration. Build/test verification green. |
| M31| [Calendar decoupling + ESPN fetch layer](M31-calendar-and-espn.md) | ✅ Done | 2026-04-29 | 2026-04-29 | V016 `season_calendar` migration + per-save calendar; `Schedule::generate_with_dates` + calendar-aware phase helpers; pure-Rust `nba3k-scrape::sources::espn` (6 fetchers + parsers, fixture-driven). 309 passed + 1 ignored. Legacy `new --team BOS` byte-identical. |
| M32| [Live importer + --from-today flag](M32-from-today-importer.md) | ✅ Done | 2026-04-29 | 2026-04-29 | V017 `player_season_stats`; `nba3k-cli::from_today` Rust-native ESPN importer (no Python); `cmd_new --from-today`. Live verified 2026-04-29: teams=30, games_played=1231, players_with_stats=391, injuries=98, roster_moves=143. LAL roster shows Doncic+LeBron, OKC 64-18 standings match real NBA. 319 passed + 1 ignored. |
| M33| [TUI wizard + season-advance + docs](M33-tui-and-polish.md) | ✅ Done | 2026-04-29 | 2026-04-29 | TUI wizard adds `Start mode` step (Fresh / Today) with i18n EN/ZH; `season-advance` writes per-year `season_calendar` rows via `next_calendar_from_previous` (Tuesday-snapped +365d); README "Start from Today" section with known gaps. 320 passed + 2 ignored. |
| M34| [Live ESPN start is the default](M34-from-today-default.md) | ✅ Done | 2026-04-29 | 2026-04-29 | `cmd_new` now defaults to live ESPN import; new `--offline` opt-out replays the legacy seed-anchored path. `--from-today` deprecated to hidden no-op. TUI wizard drops M33's `Start mode` step + 4 i18n keys. All integ/smoke tests pinned to `--offline`. 320 passed + 2 ignored. |
| M35| [Snapshot semantics matching NBA 2K](M35-snapshot-semantics.md) | ✅ Done | 2026-04-29 | 2026-04-29 | `from_today` now imports a snapshot (standings + rosters + injuries + season-to-date stats + future schedule), not a historical replay. Past played games and trade-news backfill removed. Matches NBA 2K MyNBA "Start Today" documented behavior. 320 passed + 2 ignored. |
| M36| [Draft-pick trading system](M36-draft-picks.md) | 🚧 Partial | 2026-04-29 | — | V018 pick resolution fields; offline 420-row pick horizon; live Spotrac overlay verified `420|7|137`; `picks` CLI; `trade propose --send-picks/--receive-picks`; accepted trades transfer picks; draft order uses pick owners; seven-year + Stepien CBA tests green. TUI pick-selection surfaces pending. |
| BF1| [Roster bugfix: phase-aware caps + season-start gate](BUGFIX-offseason-roster-cap.md) + [user-cut variant](BUGFIX-season-start-user-cut.md) | ✅ Done | 2026-04-29 | 2026-04-29 | Trade roster bounds phase-aware (offseason/preseason 13-21, regular/playoffs 13-18). PreSeason → Regular gate refuses to advance if user team > 15; AI not gated, AI_FA_ROSTER_CAP 16 → 15. New-game wizard drops Season step (5 → 4). 329 passed + 2 ignored. Commit `7973832`. |

## Working agreements

- **Each phase ends with a Bash-verifiable artifact.** No phase signs off without the assertion command from its doc passing.
- **Per-phase doc is updated continuously** during the phase: sub-task status, decisions made, deviations from plan, blockers surfaced.
- **TUI 8-menu policy:** Home / Roster / Rotation / Trades / Draft / Finance / Calendar / Settings.
- **Phases M2+ use agent teams** (TeamCreate) with non-overlapping crate ownership. Orchestrator (main session) does integration + verification.
- **Memory layer** in `~/.claude/projects/.../memory/` captures durable project context that survives across sessions. Per-phase docs capture in-flight work.

## Agent team conventions

- One team per active phase. Team name format: `nba3k-m{N}`.
- Worker assignments are by **crate path ownership** (e.g., `nba3k-scrape`) so workers don't collide.
- Integration/glue work (CLI wiring, end-to-end verification) is done by orchestrator after workers complete, not by workers themselves.
- Workers communicate progress via `TaskUpdate` on the shared team task list. Orchestrator monitors via `TaskList`.

## Documents

- `RESEARCH.md` — open-items research output (one-shot, refresh as needed)
- `phases/M{N}-*.md` — per-phase plan + log
- `~/.claude/plans/bubbly-roaming-thacker.md` — original approved plan (immutable reference)
