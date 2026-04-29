//! M32 — Live "Start From Today" importer.
//!
//! Pulls current real-world NBA state from ESPN's public JSON endpoints
//! and writes a fully-populated save: today's standings, real played
//! games, current rosters (post-trade / post-signing), inline injuries,
//! season-to-date player aggregates, and trade-news feed.
//!
//! Pure-Rust pipeline — no Python dependency. Reuses the seed copy +
//! starter / role / FA helpers from `cmd_new` so the only delta is the
//! data ingestion layer.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{Datelike, Duration, NaiveDate};
use nba3k_core::{
    GameId, GameMode, InjurySeverity, InjuryStatus, PlayerId, PlayerSeasonStats, SeasonCalendar,
    SeasonId, SeasonPhase, SeasonState, TeamId,
};
use nba3k_scrape::cache::Cache;
use nba3k_scrape::sources::espn;
use nba3k_store::Store;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration as StdDuration;

#[derive(Debug, Default, Clone)]
pub struct TodayReport {
    pub teams_loaded: u32,
    pub games_played: u32,
    pub games_unplayed: u32,
    pub players_with_stats: u32,
    pub injuries_marked: u32,
    pub roster_moves_applied: u32,
    pub news_backfilled: u32,
}

/// Build a fresh save populated from today's real-world NBA state.
///
/// Steps:
/// 1. Pre-flight HEAD ESPN (5 s timeout) — bail loud on no network.
/// 2. Copy seed → out.
/// 3. Open store (refinery runs V016/V017).
/// 4. ESPN: teams + standings + per-day scoreboards + per-team rosters +
///    league-wide player stats + trade news.
/// 5. Map abbrev → TeamId via seed; resolve season window from V016 default.
/// 6. Replace standings, schedule, rosters, injuries, season stats, news.
/// 7. Write SeasonState (phase derived from today vs calendar).
/// 8. Run the same starter / role / FA seed pass `cmd_new` does.
pub fn build_today_save(
    out: &Path,
    user_team_abbrev: &str,
    mode: GameMode,
    today: NaiveDate,
) -> Result<TodayReport> {
    // (1) Pre-flight. If ESPN is unreachable, bail before touching disk.
    preflight()
        .map_err(|e| anyhow!("--from-today requires internet access to ESPN. {}", e))?;

    // (2) Wal/shm cleanup + seed copy. Mirror cmd_new's logic so we don't
    // produce a corrupt file from stale sidecars.
    if out.exists() {
        bail!("refusing to overwrite existing save at {}", out.display());
    }
    cleanup_wal_shm(out);
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    let seed = std::path::PathBuf::from(crate::commands::DEFAULT_SEED_PATH);
    if !seed.exists() {
        bail!(
            "seed DB not found at {}; --from-today needs the seed (run `cargo run -p nba3k-scrape --release -- --out {}`)",
            seed.display(),
            seed.display(),
        );
    }
    std::fs::copy(&seed, out).with_context(|| {
        format!("copy seed {} -> {}", seed.display(), out.display())
    })?;

    // From this point on, any error must remove the half-written file so
    // the user is left in a clean state and can retry.
    let result = run_import(out, user_team_abbrev, mode, today);
    if result.is_err() {
        let _ = std::fs::remove_file(out);
        let _ = std::fs::remove_file(out.with_extension("db-wal"));
        let _ = std::fs::remove_file(out.with_extension("db-shm"));
    }
    result
}

fn preflight() -> Result<()> {
    let cli = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_secs(5))
        .build()
        .context("build preflight client")?;
    let url = "https://site.api.espn.com/apis/site/v2/sports/basketball/nba/teams";
    let r = cli.head(url).send().context("HEAD ESPN /teams")?;
    if !r.status().is_success() {
        bail!("ESPN HEAD returned {}", r.status());
    }
    Ok(())
}

fn cleanup_wal_shm(out: &Path) {
    let with_suffix = |suf: &str| -> std::path::PathBuf {
        let mut ext = out
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        ext.push_str(suf);
        out.with_extension(ext)
    };
    let _ = std::fs::remove_file(with_suffix("-wal"));
    let _ = std::fs::remove_file(with_suffix("-shm"));
}

fn run_import(
    out: &Path,
    user_team_abbrev: &str,
    mode: GameMode,
    today: NaiveDate,
) -> Result<TodayReport> {
    let mut report = TodayReport::default();

    let mut store = Store::open(out).context("open new save")?;

    // Cache root. Reuse the existing scrape cache so repeated runs reuse
    // disk hits. Falls back to a temp dir if the workspace cache is unwritable.
    let cache_root = std::env::var("NBA3K_CACHE_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("data/cache"));
    let cache = Cache::new(&cache_root).context("init scrape cache")?;

    // Resolve season_year from `today`. NBA seasons span Oct..June.
    let season_year = if today.month() >= 9 {
        (today.year() + 1) as u16
    } else {
        today.year() as u16
    };
    let season = SeasonId(season_year);

    // Calendar: V016 seeded the 2025-26 row at migration. Fall back to the
    // hardcoded default for other years (M33's season-advance writes real
    // rows per year).
    let cal = store
        .get_season_calendar(season)?
        .unwrap_or_else(|| SeasonCalendar::default_for(season_year));
    store.upsert_season_calendar(&cal)?;

    // (3) Teams: ESPN abbrev ↔ ESPN id ↔ seed TeamId map.
    let teams_bytes = espn::fetch_teams(&cache)?
        .ok_or_else(|| anyhow!("ESPN teams returned no data"))?;
    let espn_teams = espn::parse_teams(&teams_bytes)?;
    let mut team_map: HashMap<String, (TeamId, u32)> = HashMap::new();
    for et in &espn_teams {
        let seed_abbrev = espn_to_seed_abbrev(&et.abbrev);
        if let Some(team_id) = store.find_team_by_abbrev(seed_abbrev)? {
            // Index by ESPN abbrev so scoreboard / roster code paths can
            // look up by what ESPN itself returns; the team_id resolves
            // to the seed row.
            team_map.insert(et.abbrev.clone(), (team_id, et.id));
        } else {
            tracing::warn!(
                espn = %et.abbrev,
                seed = seed_abbrev,
                "ESPN team has no seed match — skipping"
            );
        }
    }
    if team_map.is_empty() {
        bail!("ESPN teams produced zero seed-matched team rows");
    }
    report.teams_loaded = team_map.len() as u32;

    // (4) Standings: write into existing standings table.
    let st_bytes = espn::fetch_standings(&cache, season_year)?
        .ok_or_else(|| anyhow!("ESPN standings returned no data"))?;
    let st_rows = espn::parse_standings(&st_bytes)?;
    for row in &st_rows {
        if let Some((team_id, _)) = team_map.get(&row.abbrev) {
            store.upsert_standing(*team_id, season, row.w, row.l, None)?;
        }
    }

    // (5) Schedule: replace per-season rows. Past games carry minimal
    // box scores so `record_game` produces final scores in the games
    // table. Future games stay in `schedule` with played=0.
    store.clear_schedule_for_season(season)?;
    let mut schedule_rows: Vec<(u64, SeasonId, NaiveDate, TeamId, TeamId)> = Vec::new();
    let mut completed_games: Vec<(u64, NaiveDate, TeamId, TeamId, u16, u16)> = Vec::new();
    let id_offset: u64 = (season.0 as u64) * 10_000;
    let mut game_seq: u64 = 0;

    let mut date = cal.start_date;
    while date <= cal.end_date {
        if let Some(b) = espn::fetch_scoreboard(&cache, date)? {
            let games = espn::parse_scoreboard(&b)?;
            for g in games {
                let home_id = team_map.get(&g.home_abbrev).map(|(t, _)| *t);
                let away_id = team_map.get(&g.away_abbrev).map(|(t, _)| *t);
                let (Some(home), Some(away)) = (home_id, away_id) else {
                    continue;
                };
                let game_id = id_offset + game_seq;
                game_seq += 1;
                schedule_rows.push((game_id, season, g.date, home, away));
                if g.completed {
                    if let (Some(hp), Some(ap)) = (g.home_pts, g.away_pts) {
                        completed_games.push((game_id, g.date, home, away, hp, ap));
                    }
                }
            }
        }
        date += Duration::days(1);
    }
    if schedule_rows.is_empty() {
        bail!("ESPN scoreboard loop produced zero games — schema may have changed");
    }
    store
        .bulk_insert_schedule(&schedule_rows)
        .context("bulk insert schedule")?;

    // Mark completed games as played=1 and write a minimal `games` row
    // (final score only — per-player box scores for past games are not
    // imported; downstream code tolerates empty PlayerLine vecs).
    let played_count = completed_games.len() as u32;
    for (gid, gd, home, away, hp, ap) in &completed_games {
        store.conn().execute(
            "UPDATE schedule SET played = 1 WHERE game_id = ?1",
            rusqlite::params![*gid as i64],
        )?;
        let result = nba3k_core::GameResult {
            id: GameId(*gid),
            season,
            date: *gd,
            home: *home,
            away: *away,
            home_score: *hp,
            away_score: *ap,
            box_score: nba3k_core::BoxScore {
                home_lines: vec![],
                away_lines: vec![],
            },
            overtime_periods: 0,
            is_playoffs: false,
        };
        store.record_game(&result)?;
    }
    report.games_played = played_count;
    report.games_unplayed = (schedule_rows.len() as u32).saturating_sub(played_count);

    // (6) Player season stats: name-match against existing players. Build
    // a lower-cased name index once. Unmatched names are warned but not
    // inserted — keeping ratings_json synthesis out of the hot path.
    let name_index = build_name_index(&store)?;
    let ps_bytes = espn::fetch_player_stats(&cache, season_year)?
        .ok_or_else(|| anyhow!("ESPN player_stats returned no data"))?;
    let ps_rows = espn::parse_player_stats(&ps_bytes)?;
    let mut stat_count = 0_u32;
    for r in &ps_rows {
        let Some(pid) = lookup_player(&r.display_name, &r.team_abbrev, &name_index, &store)?
        else {
            tracing::warn!(name = %r.display_name, "no seed match for player stats row");
            continue;
        };
        let row = PlayerSeasonStats {
            player_id: pid,
            season_year,
            gp: r.gp,
            mpg: r.mpg,
            ppg: r.ppg,
            rpg: r.rpg,
            apg: r.apg,
            spg: r.spg,
            bpg: r.bpg,
            fg_pct: r.fg_pct,
            three_pct: r.three_pct,
            ft_pct: r.ft_pct,
            ts_pct: r.ts_pct,
            usage: r.usage,
        };
        store.upsert_player_season_stats(&row)?;
        stat_count += 1;
    }
    report.players_with_stats = stat_count;

    // (7) Rosters per team: drives current team_id + injury status. ESPN's
    // `roster` includes every active player on each team, so this fixes
    // any post-trade / post-signing drift relative to the seed.
    let unplayed_count = report.games_unplayed;
    let mut roster_moves = 0_u32;
    let mut injuries_marked = 0_u32;
    for et in &espn_teams {
        let Some((team_id, espn_id)) = team_map.get(&et.abbrev).cloned() else {
            continue;
        };
        let Some(rb) = espn::fetch_roster(&cache, espn_id)? else {
            continue;
        };
        let (_abbrev, entries) = espn::parse_roster(&rb)?;
        for e in entries {
            let Some(pid) = lookup_player(&e.display_name, &et.abbrev, &name_index, &store)?
            else {
                tracing::warn!(name = %e.display_name, team = %et.abbrev, "no seed match for roster entry");
                continue;
            };
            // Move player to this team if they're elsewhere.
            let prior_team = player_team(&store, pid)?;
            if prior_team != Some(team_id) {
                store.assign_player_to_team(pid, team_id)?;
                roster_moves += 1;
            }
            // Map injury text into our InjuryStatus shape.
            if let Some(status_text) = e.injury_status {
                let inj = parse_injury(&status_text, e.injury_detail.as_deref(), unplayed_count);
                if let Some(inj) = inj {
                    set_player_injury(&store, pid, &inj)?;
                    injuries_marked += 1;
                }
            }
        }
    }
    report.roster_moves_applied = roster_moves;
    report.injuries_marked = injuries_marked;

    // (8) News (trade type), capped at 30-day window.
    if let Some(nb) = espn::fetch_news_trades(&cache, 50)? {
        let items = espn::parse_news_trades(&nb)?;
        let cutoff = today - Duration::days(30);
        let day_index = today.signed_duration_since(cal.start_date).num_days() as u32;
        let mut count = 0_u32;
        for it in items {
            if it.published.naive_utc().date() < cutoff {
                continue;
            }
            store.record_news(season, day_index, "trade", &it.headline, Some(&it.link))?;
            count += 1;
            if count >= 50 {
                break;
            }
        }
        report.news_backfilled = count;
    }

    // (9) SeasonState. Phase derived from today vs the calendar.
    let user_team_id = store
        .find_team_by_abbrev(user_team_abbrev)?
        .ok_or_else(|| anyhow!("unknown team '{}'", user_team_abbrev))?;
    let day = today
        .signed_duration_since(cal.start_date)
        .num_days()
        .max(0) as u32;
    let phase = if today >= cal.end_date {
        SeasonPhase::Playoffs
    } else if today >= cal.trade_deadline {
        SeasonPhase::TradeDeadlinePassed
    } else if day <= nba3k_season::PRESEASON_LAST_DAY {
        SeasonPhase::PreSeason
    } else {
        SeasonPhase::Regular
    };
    let rng_seed: u64 = today
        .signed_duration_since(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
        .num_seconds() as u64;
    let state = SeasonState {
        season,
        phase,
        day,
        user_team: user_team_id,
        mode,
        rng_seed,
    };
    store.save_season_state(&state)?;
    store.set_meta("user_team", &user_team_abbrev.to_uppercase())?;

    // (10) Match cmd_new's helper passes — starters, roles, FA pool.
    crate::commands::populate_default_starters(&store, user_team_id)?;
    for team in store.list_teams()? {
        crate::commands::assign_initial_roles(&store, team.id)?;
    }
    crate::commands::seed_free_agents(&mut store)?;

    Ok(report)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// ESPN's team abbreviations differ from BBRef's in 9 cases (BKN/CHA/GS/
/// NO/NY/PHX/SA/UTAH/WSH). Translate ESPN → BBRef so `find_team_by_abbrev`
/// resolves correctly.
fn espn_to_seed_abbrev(espn: &str) -> &str {
    match espn {
        "BKN" => "BRK",
        "CHA" => "CHO",
        "GS" => "GSW",
        "NO" => "NOP",
        "NY" => "NYK",
        "PHX" => "PHO",
        "SA" => "SAS",
        "UTAH" => "UTA",
        "WSH" => "WAS",
        other => other,
    }
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphabetic())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Walk every player row once to build a lower-cased-name → ids index.
/// Cheap on a 500-player seed; saves O(N) lookups inside the loops above.
fn build_name_index(store: &Store) -> Result<HashMap<String, Vec<PlayerId>>> {
    let mut stmt = store
        .conn()
        .prepare("SELECT id, name FROM players")
        .context("prepare players index")?;
    let mut idx: HashMap<String, Vec<PlayerId>> = HashMap::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let name: String = r.get(1)?;
        idx.entry(normalize_name(&name))
            .or_default()
            .push(PlayerId(id as u32));
    }
    Ok(idx)
}

fn player_team(store: &Store, pid: PlayerId) -> Result<Option<TeamId>> {
    let row: Option<Option<i64>> = store
        .conn()
        .query_row(
            "SELECT team_id FROM players WHERE id = ?1",
            rusqlite::params![pid.0 as i64],
            |r| r.get(0),
        )
        .ok();
    Ok(row.flatten().map(|n| TeamId(n as u8)))
}

/// Resolve an ESPN player name to a seed `PlayerId`. Match strategy:
/// 1. Exact lower-cased letter-only match.
/// 2. Strip common suffixes (jr / sr / iii / iv) and retry.
/// 3. On collision, pick the candidate currently on `team_abbrev`.
fn lookup_player(
    espn_name: &str,
    team_abbrev: &str,
    index: &HashMap<String, Vec<PlayerId>>,
    store: &Store,
) -> Result<Option<PlayerId>> {
    let key = normalize_name(espn_name);
    let mut candidates = index.get(&key).cloned().unwrap_or_default();
    if candidates.is_empty() {
        let stripped = strip_suffix(espn_name);
        if stripped != espn_name {
            let key2 = normalize_name(&stripped);
            candidates = index.get(&key2).cloned().unwrap_or_default();
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    if candidates.len() == 1 {
        return Ok(Some(candidates[0]));
    }
    // Prefer the candidate already on the ESPN team.
    if let Some(team_id) = store.find_team_by_abbrev(team_abbrev)? {
        for pid in &candidates {
            if player_team(store, *pid)? == Some(team_id) {
                return Ok(Some(*pid));
            }
        }
    }
    Ok(Some(candidates[0]))
}

fn strip_suffix(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for suf in [" jr.", " sr.", " iii", " iv", " ii"] {
        if lower.ends_with(suf) {
            return name[..name.len() - suf.len()].trim().to_string();
        }
    }
    name.to_string()
}

/// Translate ESPN's free-text injury status into an `InjuryStatus`.
fn parse_injury(
    status: &str,
    detail: Option<&str>,
    unplayed_count: u32,
) -> Option<InjuryStatus> {
    let lower = status.trim().to_ascii_lowercase();
    let (severity, games) = match lower.as_str() {
        "out for season" | "season-ending" | "season ending" => (
            InjurySeverity::SeasonEnding,
            unplayed_count.max(20) as u16,
        ),
        "out" => (InjurySeverity::LongTerm, 30),
        "day-to-day" | "day to day" | "questionable" | "gtd" => (InjurySeverity::DayToDay, 1),
        _ => return None,
    };
    Some(InjuryStatus {
        description: detail.unwrap_or(status).to_string(),
        games_remaining: games,
        severity,
    })
}

fn set_player_injury(store: &Store, pid: PlayerId, inj: &InjuryStatus) -> Result<()> {
    let json = serde_json::to_string(inj)?;
    store.conn().execute(
        "UPDATE players SET injury_json = ?1 WHERE id = ?2",
        rusqlite::params![json, pid.0 as i64],
    )?;
    Ok(())
}
