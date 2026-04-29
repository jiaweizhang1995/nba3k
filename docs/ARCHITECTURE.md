# ARCHITECTURE.md — how the code is laid out

## Workspace (8 crates)

```
nba3k-core      Public types — Player, Team, LeagueYear, TradeOffer,
                LeagueSnapshot, SeasonCalendar, PlayerSeasonStats.
                Zero I/O. Everything downstream depends on this.
nba3k-models    7 explainable scoring models — value, contract, context,
                star protection, fit, trade-accept, stat projection.
                Pure functions, no state.
nba3k-sim       Statistical match sim + 9-D team strength vector.
                Per-possession sim is v2 territory (not built).
nba3k-trade     Trade engine: evaluator, CBA validator, GM personality,
                multi-round negotiation, 3-team support. Stateless given
                a snapshot.
nba3k-season    Schedule generation, playoffs, awards, HoF, all-star,
                NBA Cup, calendar-aware phase helpers.
nba3k-store     SQLite persistence + `refinery` migrations. Owns ALL
                schema. All writes go through this crate.
nba3k-scrape    Bootstrap scraper (Basketball-Reference, rate-limited
                1 req / 3s) + ESPN client (M31+) + calibration tools.
nba3k-cli       argv parser, REPL, TUI, command implementations.
                The thick top of the stack.
```

Pinning: every internal crate is referenced from member `Cargo.toml`
files via `workspace = true`. Top-level `Cargo.toml` is the single
source of truth for versions.

## Three entry points share one parser

`crates/nba3k-cli/src/main.rs` decides between four modes based on argv
and stdin:

1. **Subcommand given** → `commands::dispatch` and exit.
2. **`--script <file>`** → replay each line via `repl::run_script`.
3. **stdin not a TTY** → pipe mode (`repl::run_pipe`).
4. **Otherwise** → interactive `rustyline` REPL.

The same `Command` enum (`crates/nba3k-cli/src/cli.rs`) drives all
four. REPL lines are tokenized with `shlex` and fed back into `clap`.

**When you add a command, you wire it once.** Add to the `Command`
enum, then handle it in `commands::dispatch`. CLI / REPL / `--script`
all pick it up.

The TUI is launched via the `tui` subcommand. It is implemented as
screens that *call into the same command/state APIs*, not a parallel
codebase. See `crates/nba3k-cli/src/tui/screens/` for the 8-menu
layout.

## Data flow

The single source of truth is the SQLite save file. Every command:

1. Opens save → `Store` from `nba3k-store`.
2. Builds a `LeagueSnapshot` (in `nba3k-core`) for the active season.
3. Passes the snapshot read-only into `nba3k-sim` / `nba3k-trade` /
   `nba3k-season` / `nba3k-models`.
4. Persists results back through `Store`.

Snapshots are **derived state**. They are NOT cached across commands.
Each command rebuilds from the DB, so commands compose cleanly under
`--script` (and the integ test exploits this).

Trade-engine CBA roster bounds are phase-aware: offseason, draft, free
agency, and preseason use the 21-player training-camp cap, while regular,
trade-deadline-passed, and playoff phases use the modeled 18-slot cap.
The regular-season opener gate is separate from trade validation and
blocks `PreSeason` → `Regular` in CBA-enforcing modes if the user team
has more than 15 modeled standard-contract players. AI teams are not
gated and may enter the regular season above the cap; they are also not
auto-cut.

## Schema is migration-first

Schema lives only as `.sql` files in
`crates/nba3k-store/migrations/V###__name.sql`. `refinery` runs them in
order on every store open.

**Never edit a committed migration.** Add a new one. Migration numbers
are referenced in `phases/PHASES.md` as part of phase verification
(e.g., V006 = free agents, V014 = rotation, V016 = season_calendar,
V017 = player_season_stats). The numbering doubles as a phase
changelog.

Current high-water mark: **V017** (post-M32). Use V018 for the next
schema change.

## Save file convention

A save is a `.db` file plus `-shm` / `-wal` siblings (WAL mode).
`data/seed_2025_26.sqlite` is the read-only league seed — every `new`
clones it (or, post-M34, runs the live ESPN importer on top of a
clone). Don't touch the seed for game-state writes; if you need to
alter the starting universe, regenerate via `nba3k-scrape`.

## `commands.rs` is intentionally one big file

`crates/nba3k-cli/src/commands.rs` is ~6.5K lines. It is the dispatch
table for every command across CLI / REPL / TUI. Past phases have
specifically chosen to keep it as one router rather than fragment it
along feature lines. Don't split it speculatively.

Helpers that other modules call (e.g.
`populate_default_starters`, `assign_initial_roles`,
`seed_free_agents`, `season_calendar_or_default`,
`next_calendar_from_previous`) live in `commands.rs` as `pub(crate)`
functions.

## TUI architecture

`crates/nba3k-cli/src/tui/` has:

- `mod.rs` — main app loop, screen routing, key dispatch
- `widgets.rs` — shared widgets (Picker, NumberInput, TextInput, Theme)
- `screens/` — one file per screen (home / roster / rotation / trades /
  draft / finance / calendar / settings + new_game + saves + launch +
  legacy).

TUI mutations always route through `crate::commands::dispatch(app,
Command)` wrapped in `crate::tui::with_silenced_io(|| ...)`. Never
write a parallel mutation path inside a TUI screen.

## i18n

Every TUI string routes through `t(lang, T::...)`. Three files stay in
sync:

- `crates/nba3k-core/src/i18n.rs` — `T` enum
- `crates/nba3k-core/src/i18n_en.rs` — English values
- `crates/nba3k-core/src/i18n_zh.rs` — 中文 values

When you add a key, edit all three together or the build will fail at
runtime (missing arm in the lookup match).

Player names + team abbreviations + team full names are **data, not
chrome** — they stay English even in Chinese mode.

## Determinism

RNG goes through `rand_chacha::ChaCha8Rng` seeded explicitly from
`SeasonState.rng_seed`. Seeds are derived from real values, never
`thread_rng`.

When fixing a sim or trade bug, prefer a regression test that pins the
seed over a new heuristic. The integ test in
`crates/nba3k-cli/tests/integration_season1.rs` runs an entire scripted
season + 30 days of season 2 in a few seconds; piggyback on it when
possible.

## Game-mode flag

`--god` (or `mode = god` at save creation) skips CBA validation and
forces AI to accept trades. Several commands branch on this; new
validation logic must respect it (`AppState::god` / per-save `mode`
column).

## "Start From Today" pipeline (post-M35)

`nba3k-cli/src/from_today.rs` orchestrates the live import. It:

1. HEAD-pings ESPN with a 5 s timeout (preflight).
2. Copies the seed → out, runs migrations.
3. Pulls teams + standings + per-team rosters (with inline injuries) +
   league-wide season-to-date player stats from ESPN.
4. Walks scoreboards from `today` to `season_end`, inserting future
   games into `schedule`.
5. Resolves player names against the seed via lower-cased letter-only
   matching with suffix-strip retry; team-abbrev tiebreak on collision.
6. Writes `SeasonState` with `phase` derived from today vs the
   per-save `season_calendar` row.
7. Runs the same starter / role / FA seed pass `cmd_new`'s legacy path
   does.

The model matches NBA 2K MyNBA "Start Today" — a *snapshot*, not a
historical replay. Past played games and trade-news are deliberately
not imported.
