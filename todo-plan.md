# todo-plan.md — "Start From Today" v2 (ESPN public JSON, Rust-native) — codex CLI execution doc

**Maintainer**: main agent (Claude). Codex picks tasks, flips status, asks main agent if scope shifts.

**Working dir**: `/Users/jimmymacmini/Desktop/claude-code-project/nba3k-claude`
**Build**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --workspace`
**Test**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo test --workspace`
**Release**: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo build --release --bin nba3k`
**Run**: `./target/debug/nba3k tui` / `./target/release/nba3k tui`

**Phase tracker**: `phases/PHASES.md`. New phases land as M31 / M32 / M33.

**Approved plan reference**: `~/.claude/plans/nba-2k-start-from-polished-flask.md` (v1 strategy; v2 supersedes the data source choice — ESPN replaces `nba_api`).

**Status legend**: `[ ]` not started · `[~]` in progress · `[x]` done · `[!]` blocked (write reason)

---

## Why this is v2 (do not relitigate)

v1 of this plan (codex executed, then reverted) used Python `nba_api` shellout for the live state pull. Three independent failures killed it:

1. `stats.nba.com` is Cloudflare-protected. nba_api's HTTP calls hit `Connection reset by peer` from Mac client networks roughly half the time. The importer had no retry path.
2. `LeagueStandings` from nba_api does NOT expose a `TeamAbbreviation` column — only `TeamID` + `TeamCity` + `TeamName`. The scraper script filtered on the missing field and silently emitted `[]` for 30 teams.
3. `nba_api` has no league-wide injury endpoint and no clean transactions endpoint. v1 punted with empty-stub Python files. That blocks the whole importer because the Doncic-in-LAL acceptance bar fails without transactions.

**ESPN's public JSON API solves all three at once and removes the Python dependency entirely.**

Live tests (run 2026-04-29 from this machine):

| Need | ESPN endpoint | Verified |
|---|---|---|
| 30 team abbrev↔team_id map | `https://site.api.espn.com/apis/site/v2/sports/basketball/nba/teams` | 200, 30 teams |
| Standings (W-L, conf rank, streak) | `https://site.web.api.espn.com/apis/v2/sports/basketball/nba/standings?season=2026` | 200, 30 teams, full per-team stats |
| Daily scoreboard + final scores + records | `https://site.api.espn.com/apis/site/v2/sports/basketball/nba/scoreboard?dates=YYYYMMDD` | 200, 9 events for 2026-01-28, "LAL @ CLE 99-129 Final" |
| **Current roster + injuries inline** | `https://site.web.api.espn.com/apis/site/v2/sports/basketball/nba/teams/{id}/roster` | 200, **Doncic on LAL with INJ:Out**, Reaves "Out" |
| All-player season stats | `https://site.web.api.espn.com/apis/common/v3/sports/basketball/nba/statistics/byathlete?season=2026&seasontype=2&limit=600` | 200, 304 players with ppg/rpg/usage in 1.8 MB |
| News (Trade type) | `https://site.api.espn.com/apis/site/v2/sports/basketball/nba/news?limit=50&type=Trade` | 200, 22 KB |

Total ≈ 224 HTTP calls for one save (1 + 1 + ≈190 daily scoreboards + 30 rosters + 1 + 1). ESPN does not enforce a 3s rate limit; 100ms spacing is fine, parallelism allowed.

**Key wins over nba_api**:
- No Python dependency. `reqwest` is already in `[workspace.dependencies]`.
- No Cloudflare blocks. ESPN serves directly.
- Roster + injuries come from one endpoint per team, not two flaky ones.
- Field names (`abbreviation`, `displayName`, `injuries[].status`) are stable across years.
- One call returns the entire 304-player season-stats leaderboard.

Trade-off: full schedule needs ≈190 daily scoreboard calls instead of one bulk game-log call. ESPN tolerates parallelism and the calls are 5-50 KB each, so total wall time on a 10-worker pool is well under a minute.

---

## Goal

Add `nba3k --save x.db new --team BOS --from-today` (and the matching TUI wizard step) so a fresh save lands on **today's real-world NBA state**: today's date, today's W-L, today's rosters (post-trades / signings / injuries), the season's already-played games as `played=1` schedule rows + minimal box scores, season-to-date player aggregates, and the real remaining schedule. **No Python dependency.** `pip install nba_api` is no longer required for `--from-today`. The existing `nba_api` shellout used by the seed-build path stays untouched (USG/TS augmentation only).

**Three-mode entry**:

```
nba3k --save x.db new --team BOS                  # unchanged: fresh Oct 2025, day 0, RNG schedule
nba3k --save x.db new --team BOS --from-today     # new: live ESPN import, day≈190, real schedule + rosters + injuries
nba3k --save x.db tui                             # new wizard step: [Fresh October 2025] / [Today (live ESPN data)]
```

**Locked invariants** (do not break):
- Existing `nba3k --save x.db new --team BOS` flow stays byte-identical when `--from-today` is NOT passed. All current saves and tests keep working.
- `Store::open` runs refinery automatically; new migrations are `.sql` files only — never edit a committed migration, only add V016, V017, ... in order.
- TUI mutations route through `crate::commands::dispatch(app, Command)` wrapped in `crate::tui::with_silenced_io(|| ...)`.
- All writes go through `nba3k-store`. No new persistence path.
- Tests must pass before marking task done. Baseline: 303 passed + 1 ignored (workspace, post-M30). Each task either keeps the count the same or grows it.
- i18n: every new TUI string routes through `t(tui.lang, T::...)`. Add T keys to `i18n.rs` + `i18n_en.rs` + `i18n_zh.rs` together.
- Workspace-pinned deps only. New deps go to root `Cargo.toml` `[workspace.dependencies]` then `workspace = true`.
- `--from-today` is offline-fail-loud: when ESPN is unreachable the importer aborts with a clear message, removes the half-written file, and exits 1. No partial save.
- Determinism rule: when `--from-today` is used, the imported current real season cannot reuse a fixed RNG seed for past games (results come from real data, not sim). For year+1 onward the seeded sim path resumes.

---

## M31 — Calendar decoupling + ESPN fetch layer

Goal: parameterize the date constants and add a Rust-native ESPN client. **No user-visible change.** Old `new` path stays identical.

### T1 — Migration V016: `season_calendar` table

**Status**: `[x]`

**Goal**: Single per-season calendar row that drives schedule + phase math.

**Schema** (`crates/nba3k-store/migrations/V016__season_calendar.sql`):
```sql
CREATE TABLE season_calendar (
    season_year     INTEGER PRIMARY KEY,        -- end-year, e.g. 2026 for 2025-26
    start_date      TEXT NOT NULL,              -- ISO YYYY-MM-DD, regular-season opening night
    end_date        TEXT NOT NULL,              -- ISO YYYY-MM-DD, last regular-season game
    trade_deadline  TEXT NOT NULL,              -- ISO YYYY-MM-DD
    all_star_day    INTEGER NOT NULL DEFAULT 41,
    cup_group_day   INTEGER NOT NULL DEFAULT 30,
    cup_qf_day      INTEGER NOT NULL DEFAULT 45,
    cup_sf_day      INTEGER NOT NULL DEFAULT 53,
    cup_final_day   INTEGER NOT NULL DEFAULT 55
);
INSERT INTO season_calendar (season_year, start_date, end_date, trade_deadline)
VALUES (2026, '2025-10-21', '2026-04-12', '2026-02-05');
```

**Files**:
- New: `crates/nba3k-store/migrations/V016__season_calendar.sql`
- `crates/nba3k-core/src/season.rs`: `pub struct SeasonCalendar { pub season_year: u16, pub start_date: NaiveDate, pub end_date: NaiveDate, pub trade_deadline: NaiveDate, pub all_star_day: u32, pub cup_group_day: u32, pub cup_qf_day: u32, pub cup_sf_day: u32, pub cup_final_day: u32 }` plus a `default_for(season_year)` constructor returning the 2025-26 hardcoded values.
- `crates/nba3k-store/src/store.rs`: `get_season_calendar(season: SeasonId) -> Result<Option<SeasonCalendar>>` and `upsert_season_calendar(&SeasonCalendar)`.

**Acceptance**:
- New saves auto-have a 2026 row after `cmd_new` (it runs `Store::open` which runs migrations).
- Existing saves (e.g. user's old `.db` files) auto-receive the row on next open via refinery.

**Verification**: `cargo test -p nba3k-store -- season_calendar` round-trips a struct.

---

### T2 — Parameterize `Schedule::generate` and `phases::*`

**Status**: `[x]`

**Goal**: Drop the load-bearing `SEASON_START` / `SEASON_END` / trade-deadline constants. Caller passes dates / `SeasonCalendar`.

**Files**:
- `crates/nba3k-season/src/schedule.rs`: keep `pub const SEASON_START` / `SEASON_END` as fallback defaults. Add `Schedule::generate_with_dates(season: SeasonId, seed: u64, teams: ..., start: NaiveDate, end: NaiveDate) -> Self`. The existing `Schedule::generate(season, seed, teams)` becomes a thin wrapper that calls the new function with the const defaults — old callers stay green.
- `crates/nba3k-season/src/phases.rs`: keep the legacy `is_after_trade_deadline(date)` / `is_trade_deadline_day(date)` for back-compat. Add `pub fn is_after_trade_deadline_for(date: NaiveDate, cal: &SeasonCalendar) -> bool` and `pub fn is_trade_deadline_day_for(date: NaiveDate, cal: &SeasonCalendar) -> bool` and a `pub fn trade_deadline(cal: &SeasonCalendar) -> NaiveDate` accessor.
- `crates/nba3k-cli/src/commands.rs`: add `fn season_calendar_or_default(store: &Store, season: SeasonId) -> SeasonCalendar` near the existing `ALL_STAR_DAY` / `CUP_*_DAY` constants. The const stay as fallback only. Every call site in `commands.rs` (schedule generation, sim-day all-star/cup triggers, phase advancement, trade-deadline checks) routes through this helper.
- `crates/nba3k-cli/src/commands.rs` `generate_and_store_schedule`: load the calendar then pass `cal.start_date, cal.end_date` into `Schedule::generate_with_dates`.

**Acceptance**:
- All existing tests pass with defaults wired through the new helper.
- A new test in `crates/nba3k-season/tests/schedule_tests.rs` constructs a custom calendar (e.g. start=2025-10-15, end=2026-04-20) and asserts `Schedule::generate_with_dates` produces 1230 games inside that window.

**Verification**:
- `cargo test --workspace` (≥ 303 passed + 1 ignored).
- `cargo run --bin nba3k -- --save /tmp/oldnew.db new --team BOS` works exactly as before; `sqlite3 /tmp/oldnew.db 'SELECT MIN(date), MAX(date), COUNT(*) FROM schedule;'` returns `2025-10-21 | 2026-04-12 | 1230`.

---

### T3 — New `nba3k-scrape::sources::espn` module

**Status**: `[x]`

**Goal**: Pure-Rust ESPN client. Six fetchers + parse pairs. Reuses existing `Cache` (JSON, 7-day TTL) and adds an ESPN-specific politeness gate (100ms / req, ≥ 3 parallel connections OK).

**Files**:
- New: `crates/nba3k-scrape/src/sources/espn.rs`
- `crates/nba3k-scrape/src/sources/mod.rs`: add `pub mod espn;`
- `crates/nba3k-scrape/src/lib.rs`: re-export the public types from `espn`

**API** (each `fetch_*` returns `Ok(Some(bytes))` on cache hit or live success, `Ok(None)` if ESPN responds 4xx/5xx, `Err(e)` on transport failure; `parse_*` is pure):

```rust
// Tiny POD types, all serde::Deserialize
pub struct EspnTeam { pub id: u32, pub abbrev: String, pub display_name: String }
pub struct EspnStandingRow { pub abbrev: String, pub conf: String, pub w: u16, pub l: u16, pub conf_rank: u16, pub div: String, pub div_rank: u16, pub streak: i32, pub last10: String }
pub struct EspnGameRow { pub date: NaiveDate, pub home_abbrev: String, pub away_abbrev: String, pub home_pts: Option<u16>, pub away_pts: Option<u16>, pub completed: bool, pub home_record: Option<String>, pub away_record: Option<String> }
pub struct EspnRosterEntry { pub espn_id: u64, pub display_name: String, pub jersey: Option<String>, pub position: Option<String>, pub age: Option<u8>, pub height_in: Option<u16>, pub weight_lb: Option<u16>, pub injury_status: Option<String> /* "Out"/"Day-To-Day"/etc. */, pub injury_detail: Option<String> }
pub struct EspnPlayerSeasonStat { pub espn_id: u64, pub display_name: String, pub team_abbrev: String, pub gp: u16, pub mpg: f32, pub ppg: f32, pub rpg: f32, pub apg: f32, pub spg: f32, pub bpg: f32, pub fg_pct: f32, pub three_pct: f32, pub ft_pct: f32, pub ts_pct: f32, pub usage: f32 }
pub struct EspnNewsItem { pub published: chrono::DateTime<chrono::Utc>, pub headline: String, pub link: String, pub categories: Vec<String> }

pub fn fetch_teams(cache: &Cache) -> Result<Option<Vec<u8>>>;
pub fn parse_teams(bytes: &[u8]) -> Result<Vec<EspnTeam>>;

pub fn fetch_standings(cache: &Cache, season_year: u16) -> Result<Option<Vec<u8>>>;
pub fn parse_standings(bytes: &[u8]) -> Result<Vec<EspnStandingRow>>;

pub fn fetch_scoreboard(cache: &Cache, date: NaiveDate) -> Result<Option<Vec<u8>>>;
pub fn parse_scoreboard(bytes: &[u8]) -> Result<Vec<EspnGameRow>>;

pub fn fetch_roster(cache: &Cache, espn_team_id: u32) -> Result<Option<Vec<u8>>>;
pub fn parse_roster(bytes: &[u8]) -> Result<(String /* abbrev */, Vec<EspnRosterEntry>)>;

pub fn fetch_player_stats(cache: &Cache, season_year: u16) -> Result<Option<Vec<u8>>>;
pub fn parse_player_stats(bytes: &[u8]) -> Result<Vec<EspnPlayerSeasonStat>>;

pub fn fetch_news_trades(cache: &Cache, limit: u32) -> Result<Option<Vec<u8>>>;
pub fn parse_news_trades(bytes: &[u8]) -> Result<Vec<EspnNewsItem>>;
```

**URLs** (hardcoded constants at the top of `espn.rs`):
```
https://site.api.espn.com/apis/site/v2/sports/basketball/nba/teams
https://site.web.api.espn.com/apis/v2/sports/basketball/nba/standings?season={year}
https://site.api.espn.com/apis/site/v2/sports/basketball/nba/scoreboard?dates={YYYYMMDD}
https://site.web.api.espn.com/apis/site/v2/sports/basketball/nba/teams/{id}/roster
https://site.web.api.espn.com/apis/common/v3/sports/basketball/nba/statistics/byathlete?season={year}&seasontype=2&limit=600
https://site.api.espn.com/apis/site/v2/sports/basketball/nba/news?limit={limit}&type=Trade
```

**Cache keys** (under `data/cache/espn/`):
- `teams.json`
- `standings_{year}.json`
- `scoreboard_{YYYYMMDD}.json` (one per game day)
- `roster_{espn_team_id}.json`
- `player_stats_{year}.json`
- `news_trades.json`

**Politeness**:
- `espn` source uses a 100 ms per-request gate (separate from the 3s BBRef gate). Add a second instance of the existing `politeness::PerSourceGate` keyed by `"espn"` if the current implementation supports per-source keys; otherwise add a small `EspnGate` next to the BBRef one.
- Cache TTL: 12 hours for standings / scoreboard / roster / player_stats; 1 hour for news. Use `cache::ttl_seconds(...)` shape that already exists; add new helpers if needed.
- Retry: 3 attempts on transport error / 5xx with backoff `[300ms, 800ms, 2s]`. **Do not retry 404** — that's a real "no game on this date" answer for scoreboards.

**Acceptance**:
- For each `parse_*`, a fixture-driven test in `crates/nba3k-scrape/tests/espn_parse.rs` reads a checked-in JSON file from `crates/nba3k-scrape/tests/fixtures/espn/<endpoint>.json` (recorded once, ≤ 200 KB each) and asserts the parsed counts and a couple of well-known field values (e.g. `parse_roster` of LAL fixture contains a row with `display_name == "Luka Doncic"` and `injury_status == Some("Out")`).
- No live network in tests — all fixture-driven.

**Verification**: `cargo test -p nba3k-scrape -- espn`.

---

### T4 — Phase doc M31 + PHASES.md row

**Status**: `[x]`

**Files**:
- New: `phases/M31-calendar-and-espn.md` (goals, sub-tasks T1–T3, acceptance, verification command).
- Update: `phases/PHASES.md` add an M31 row, status `In progress` while codex works it, `Done` when all three sub-tasks are `[x]` and verification passes.

**Verification**: `cargo test --workspace` final green; old `new --team BOS` byte-identical.

---

## M32 — Live importer + `--from-today` flag

Goal: brand-new save written from live ESPN data; the only user-visible delta is the `--from-today` flag.

### T5 — Migration V017: `player_season_stats`

**Status**: `[x]`

**Schema** (`crates/nba3k-store/migrations/V017__player_season_stats.sql`):
```sql
CREATE TABLE player_season_stats (
    player_id   INTEGER NOT NULL,
    season_year INTEGER NOT NULL,
    gp          INTEGER NOT NULL DEFAULT 0,
    mpg         REAL    NOT NULL DEFAULT 0,
    ppg         REAL    NOT NULL DEFAULT 0,
    rpg         REAL    NOT NULL DEFAULT 0,
    apg         REAL    NOT NULL DEFAULT 0,
    spg         REAL    NOT NULL DEFAULT 0,
    bpg         REAL    NOT NULL DEFAULT 0,
    fg_pct      REAL    NOT NULL DEFAULT 0,
    three_pct   REAL    NOT NULL DEFAULT 0,
    ft_pct      REAL    NOT NULL DEFAULT 0,
    ts_pct      REAL    NOT NULL DEFAULT 0,
    usage       REAL    NOT NULL DEFAULT 0,
    PRIMARY KEY (player_id, season_year),
    FOREIGN KEY (player_id) REFERENCES players(id) ON DELETE CASCADE
);
CREATE INDEX idx_pss_season ON player_season_stats(season_year);
```

**Files**:
- New: `crates/nba3k-store/migrations/V017__player_season_stats.sql`.
- `crates/nba3k-store/src/store.rs`: `upsert_player_season_stats(&PlayerSeasonStats)`, `list_player_season_stats(season) -> Vec<PlayerSeasonStats>`, `get_player_season_stats(player_id, season) -> Option<...>`.
- `crates/nba3k-core/src/player.rs`: `pub struct PlayerSeasonStats { ... }`.
- `crates/nba3k-cli/src/commands.rs` `records --scope season`: when a row exists in `player_season_stats`, prefer it over the on-the-fly aggregate from box scores.

**Acceptance**: round-trip test inserts 5 rows, lists them, fetches one.

**Verification**: `cargo test -p nba3k-store -- player_season_stats`.

---

### T6 — Importer module `nba3k-scrape::from_today`

**Status**: `[x]`

**Goal**: One entry point that takes a `(out_path, user_team, mode, today)` and produces a fully-loaded save from live ESPN data. **No Python.** **No `nba_api`.** All HTTP via `reqwest` (already a workspace dep).

**Signature**:
```rust
pub struct TodayReport {
    pub teams_loaded: u32,
    pub games_played: u32,
    pub games_unplayed: u32,
    pub players_with_stats: u32,
    pub injuries_marked: u32,
    pub roster_moves_applied: u32,        // players whose team_id changed vs seed
    pub news_backfilled: u32,
}

pub fn build_today_save(
    out: &Path,
    user_team_abbrev: &str,
    mode: GameMode,
    today: NaiveDate,
) -> Result<TodayReport>;
```

**Flow** (in order):

1. **Pre-flight**: HEAD `https://site.api.espn.com/apis/site/v2/sports/basketball/nba/teams` with a 5 s timeout. On failure, bail with: `"--from-today requires internet access to ESPN. Reach https://site.api.espn.com/ failed: <error>"`. Importer must NOT leave a partial file — wrap the rest in a closure and `fs::remove_file(out)` on `Err`.
2. **Seed copy + wal/shm cleanup**: factor the existing `cmd_new` block into `commands::new_helpers::copy_seed_to(out)` and call it. Same as legacy.
3. **Open store**: `Store::open(out)` runs migrations (V016 + V017 are now in place).
4. **Fetch teams**: `espn::fetch_teams` → build `BTreeMap<abbrev → espn_team_id>`. Persist nothing yet — the seed already has team rows; we just need the ID map for later roster calls.
5. **Resolve season window**: parse the first ISO date in the upcoming `scoreboard` window. Call `fetch_scoreboard(today)`; from `leagues[0].season.{startDate, endDate}` derive `season_calendar.start_date` / `end_date`. Compute `season_year = today.year() + (1 if today.month() >= 9 else 0)`. Upsert the calendar row.
6. **Fetch standings**: `fetch_standings(season_year)` → `parse_standings`. Replace `standings` table (`UPDATE standings SET wins=?, losses=? WHERE team_id=? AND season=?`) for all 30 teams.
7. **Fetch scoreboards day-by-day** from `start_date` to `end_date`:
    - **Parallelism**: chunk dates into batches of 10, spawn `std::thread::scope` workers, each calls `fetch_scoreboard` (politeness gate already serializes per-source).
    - For each game row: insert `schedule (season, date, home_id, away_id, played)` with `played = completed`. If `completed`, also insert into `games` with a minimal `box_score_json = {"home_pts": N, "away_pts": M, "minimal": true}`.
    - **Truncate first**: `DELETE FROM schedule WHERE season=?; DELETE FROM games WHERE season=?;` — we replace, do not merge.
8. **Fetch player season stats**: `fetch_player_stats(season_year)` → `parse_player_stats`. For each row, look up the player by exact-name match against `players.name`; on collision, prefer the player whose current team matches `team_abbrev`. Insert into `player_season_stats`. Log unmatched names with `tracing::warn!` and continue — never panic.
9. **Apply current rosters (overrides 2024-25 seed)**: parallel `fetch_roster` for all 30 ESPN team IDs. For each roster entry:
    - Resolve `players` row by exact-name match. If not found, INSERT a new player row with sane defaults (overall = 60, position parsed from ESPN, age from `dateOfBirth` if present, contract = league-min stub).
    - If the player's current `team_id` differs from the ESPN team, UPDATE it. Increment `roster_moves_applied`.
    - If `injury_status` is `Some("Out")` / `"Day-To-Day"` / `"Out For Season"`, set `players.injury_json` via the existing `Player.injury` schema. Mapping: `"Out"` → 30 games; `"Day-To-Day"` / `"GTD"` → 1; `"Out For Season"` → games_remaining = unplayed_count_in_schedule.
10. **News backfill**: `fetch_news_trades(50)` → keep items with `published >= today - 30d`, insert into `news` with `kind = "TRADE"`. Cap at 50 entries.
11. **SeasonState**: `day = (today - calendar.start_date).num_days() as u32`. `phase = if today >= calendar.end_date { SeasonPhase::Playoffs } else if today >= calendar.trade_deadline { SeasonPhase::TradeDeadlinePassed } else { SeasonPhase::Regular }`. `user_team = lookup(abbrev)`. `mode = mode`. `rng_seed = entropy seed`.
12. **All-Star roster**: if `day >= calendar.all_star_day` and `all_star` is empty for this season, call existing `compute_all_star`. Cup intentionally NOT backfilled (known gap — see M33 docs).
13. **Helpers reused** (do not reimplement): `assign_initial_roles`, `populate_default_starters`, `seed_free_agents` from `nba3k-cli::commands` (export them via `commands::new_helpers`).
14. **Return** `TodayReport` with the counters.

**Files**:
- New: `crates/nba3k-scrape/src/from_today.rs` (the entry function + private helpers).
- `crates/nba3k-scrape/src/lib.rs`: `pub mod from_today; pub use from_today::{build_today_save, TodayReport};`.
- `crates/nba3k-cli/src/commands.rs`: extract the seed copy / wal cleanup / role+starter+FA setup blocks from `cmd_new` into a new pub(crate) module `crate::commands::new_helpers` so both the legacy path and the importer call them. **No behavior change** to legacy.
- New: `crates/nba3k-cli/src/commands/new_helpers.rs` (or co-located inside `commands.rs` as a sub-module — codex's call).

**Acceptance**:
- `build_today_save("/tmp/today.db", "BOS", GameMode::Standard, today)` with internet → opens via `cmd_status --json` and reports `{phase: "Regular"|"TradeDeadlinePassed"|"Playoffs", day: ≥150, season: 2026, user_team: "BOS"}`.
- `roster --team LAL` shows **Luka Doncic and LeBron James together** (current real reality on 2026-04-29).
- `standings` matches ESPN within 1 game per team (exact match preferred; ±1 acceptable due to scrape lag).
- `records --scope season --stat ppg` returns a top-5 close to real-life leaders (Shai Gilgeous-Alexander, Jokić, Doncic, etc.).

**Verification** (run by main agent after codex finishes; codex itself runs only the tests):
```bash
cargo run --release --bin nba3k -- --save /tmp/today.db new --team BOS --from-today
./target/release/nba3k --save /tmp/today.db status --json
./target/release/nba3k --save /tmp/today.db standings
./target/release/nba3k --save /tmp/today.db roster --team LAL
./target/release/nba3k --save /tmp/today.db records --scope season --stat ppg
```

Codex-side test: a unit test in `crates/nba3k-scrape/tests/from_today_offline.rs` that runs the full importer flow against checked-in fixture files (the same files from T3's `tests/fixtures/espn/`) bypassing the `fetch_*` HTTP layer via a `for_test_with_bytes(...)` ctor. Asserts:
- `roster_moves_applied >= 1` (Doncic moved off PHX — wait, Doncic was traded to LAL in February 2025; in any reasonable seed-vs-now diff there will be at least one move).
- `injuries_marked >= 1`.
- `players_with_stats >= 200`.
- `games_played >= 1000` (current season is far advanced).

---

### T7 — Wire `--from-today` into `cmd_new`

**Status**: `[x]`

**Files**:
- `crates/nba3k-cli/src/cli.rs`: in `NewArgs` add
  ```rust
  /// Build the save from today's real NBA state via ESPN public API.
  #[arg(long)]
  pub from_today: bool,
  ```
- `crates/nba3k-cli/src/commands.rs:160` `cmd_new`: if `args.from_today`, branch to `nba3k_scrape::build_today_save(...)` with `today = chrono::Local::now().date_naive()`. Skip the legacy seed-copy + `Schedule::generate` blocks (they're already moved to `new_helpers` per T6). After import, call `app.open_path(path)` and persist `meta.user_team`.
- New workspace dep: `crates/nba3k-cli/Cargo.toml` adds `nba3k-scrape = { workspace = true }` (only used at top-level).

**Acceptance**:
- `nba3k --save x.db new --team BOS` (no flag) → byte-identical legacy behavior.
- `nba3k --save x.db new --team BOS --from-today` (with internet) → live save written.
- `--from-today` with no internet → `Error: --from-today requires internet access to ESPN. ...`, exit code 1, no `.db` left behind.

**Verification**:
```bash
cargo run --bin nba3k -- --save /tmp/old.db new --team BOS                # legacy path
cargo run --bin nba3k -- --save /tmp/new.db new --team BOS --from-today   # live path
```

---

### T8 — Phase doc M32 + PHASES.md row

**Status**: `[x]`

**Files**:
- New: `phases/M32-from-today-importer.md`.
- Update: `phases/PHASES.md` row M32.

---

## M33 — TUI wizard + season-advance + docs

### T9 — TUI new-game wizard "Start Today" step

**Status**: `[x]`

**Goal**: New step before `mode_picker` lets the user choose `Fresh October 2025` (default) or `Today (live ESPN data)`. When `Today` is chosen, the season field is locked to the current real season and `from_today=true` flows into the `New` Command sent through `commands::dispatch`.

**Files**:
- `crates/nba3k-cli/src/tui/screens/new_game.rs`: introduce `enum StartMode { Fresh, Today }` step. Render it after `team_picker`, before `mode_picker`. On confirm, the dispatched `Command::New(NewArgs { from_today: matches!(start_mode, StartMode::Today), ... })` reflects the choice.
- `crates/nba3k-core/src/i18n.rs` + `i18n_en.rs` + `i18n_zh.rs`: new keys `T::NewGameStartTitle`, `T::NewGameStartFresh`, `T::NewGameStartToday`, `T::NewGameStartTodayHint` (hint text mentions "needs internet to ESPN").

**Acceptance**:
- TUI new-save flow shows the new step.
- Selecting `Today (live ESPN data)` and confirming triggers the live import (visible: `nba3k status --json` after exit shows `phase: Regular`/`TradeDeadlinePassed`/`Playoffs`, `day > 0`).
- Selecting `Fresh October 2025` keeps existing behavior.

**Verification**: `./target/release/nba3k tui` manual walkthrough; documented in M33 phase doc.

---

### T10 — `season-advance` writes a fresh `season_calendar` row for year+1

**Status**: `[x]`

**Goal**: When the seeded sim path advances to the next season, write next year's calendar entry so subsequent calls keep working without falling back to const defaults.

**Files**:
- `crates/nba3k-cli/src/commands.rs` `season_advance` handler: after bumping `SeasonState.season`, compute via a private helper `next_calendar_from_previous(prev: &SeasonCalendar) -> SeasonCalendar`:
  - `next_start = prev_start + Duration::days(365)` rounded forward to the next Tuesday.
  - `next_end = next_start + Duration::days(174)`.
  - `next_trade_deadline = next_start + Duration::days(107)`.
  - All-star / cup day offsets: copy from previous.
  - Persist via `store.upsert_season_calendar(&next_cal)`.
- The `Schedule::generate_with_dates` call inside `season_advance` reads the row that was just written.

**Acceptance**:
- A new test `crates/nba3k-cli/tests/season_advance_calendar.rs` advances 2025-26 → 2026-27 → 2027-28 and asserts three rows exist in `season_calendar` and `Schedule::generate_with_dates` was called with each year's window.

**Verification**: `cargo test -p nba3k-cli -- season_advance_calendar`.

---

### T11 — README + known gaps + M33 phase doc

**Status**: `[x]`

**Files**:
- `README.md`: add a "Start from Today" section under "三种交互方式" or "快速开始" — usage, requirement (internet only — no Python), and the known gaps:
  - No Cup backfill (current-year Cup history is skipped).
  - News backfill is the last 30 days of trade-typed items only.
  - Already-played games are imported as final scores only — no per-player box-score detail for past games.
  - Player matching is exact-name; misspellings or junior/senior collisions log a warning and skip.
  - Once the regular season ends in real life, the importer puts the save into `Playoffs` phase, but **playoff bracket backfill is not implemented** — the user can still run `playoffs sim` to generate one.
- New: `phases/M33-tui-and-polish.md`.
- Update: `phases/PHASES.md` row M33.

**Verification**: `cargo test --workspace` final green; manual run of the e2e block from T6 passes.

---

## Things codex must NOT do

- **Do not introduce a Python dependency.** No `nba_api`. No `pip install`. The existing `crates/nba3k-scrape/src/sources/nba_api.rs` (USG/TS augmentation for the seed-build) stays untouched, but no new Python scripts and no new `nba_api` calls.
- **Do not create a `crates/nba3k-scrape/py/` directory.** Delete it if it appears.
- **Do not edit committed migrations.** Always add new V### files.
- **Do not silently downgrade an empty fetch to success.** If ESPN returns an unexpected empty payload (e.g. 0-team standings), bail loudly — that's a parse bug, not a real state.
- **Do not parallelize beyond 10 concurrent ESPN requests.** Be polite.
- **Do not disable the legacy `cmd_new` path.** Both paths must coexist.

## Open questions for main agent (codex pings here if hit)

- ESPN occasionally returns `{}` for a scoreboard date with no games (off-day). That is a 200 OK + valid empty response, NOT a failure. `parse_scoreboard` must return `Ok(vec![])` and the importer must move on.
- ESPN player names occasionally include suffixes (`"LeBron James Jr."` is hypothetical, but `"Wendell Carter Jr."` is real). Match strategy: exact `display_name` first, then case-insensitive, then strip `Jr.` / `Sr.` / `III` and retry. Document the policy inline.
- If ESPN returns a player on a team that does not exist in the seed, INSERT a stub player rather than dropping the row. Mark `tracing::warn!`.
- Time zone: ESPN dates are in UTC. `chrono::Local::now().date_naive()` is local. For "today" we accept a 1-day fudge — if ESPN says a game is on `2026-04-29 03:00Z`, treat it as `2026-04-29` (date part only).

## Definition of Done

All eleven tasks `[x]`. `cargo test --workspace` green (≥ 303 passed + 1 ignored, with new tests added by T2/T3/T5/T6/T10). The e2e block from T6's verification executes end-to-end with internet access. PHASES.md shows M31, M32, M33 all `Done` with their Bash verification commands recorded.
