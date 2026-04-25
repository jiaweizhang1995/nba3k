use crate::cli::{Command, DevAction, JsonFlag, NewArgs, TradeAction};
use crate::state::AppState;
use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use indexmap::IndexMap;
use nba3k_core::*;
use nba3k_season::{phases as season_phases, schedule::Schedule, standings::Standings};
use nba3k_sim::{pick_engine, GameContext, RotationSlot, TeamSnapshot};
use nba3k_trade::{
    evaluate as evaluate_mod, negotiate as negotiate_mod,
    snapshot::{LeagueSnapshot, TeamRecordSummary},
};
use rand::Rng;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

/// Default seed DB shipped by `nba3k-scrape`. CLI `new` looks here unless overridden.
const DEFAULT_SEED_PATH: &str = "data/seed_2025_26.sqlite";

pub fn dispatch(app: &mut AppState, cmd: Command) -> Result<()> {
    match cmd {
        Command::New(args) => cmd_new(app, args),
        Command::Load { path } => cmd_load(app, path),
        Command::Status(j) => cmd_status(app, j),
        Command::Save => cmd_save(app),
        Command::Quit => {
            app.should_quit = true;
            Ok(())
        }
        Command::SimDay { count } => cmd_sim_day(app, count.unwrap_or(1)),
        Command::SimTo { phase } => cmd_sim_to(app, &phase),
        Command::Standings(j) => cmd_standings(app, j),
        Command::Roster { team, json } => cmd_roster(app, team, json),
        Command::Player { name, json } => cmd_player(app, &name, json),
        Command::Trade(args) => cmd_trade(app, args.action),
        Command::Dev(args) => cmd_dev(app, args.action),
        Command::Draft(_) => bail!("draft not implemented in M3 — see roadmap M5"),
    }
}

// ----------------------------------------------------------------------
// new / load / save / status
// ----------------------------------------------------------------------

fn cmd_new(app: &mut AppState, args: NewArgs) -> Result<()> {
    let path = app
        .save_path
        .clone()
        .ok_or_else(|| anyhow!("--save <path> required for `new`"))?;

    if path.exists() {
        bail!("refusing to overwrite existing save at {}", path.display());
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }

    let seed_path = PathBuf::from(DEFAULT_SEED_PATH);
    let used_seed = if seed_path.exists() {
        std::fs::copy(&seed_path, &path).with_context(|| {
            format!("copying seed {} -> {}", seed_path.display(), path.display())
        })?;
        true
    } else {
        false
    };

    app.open_path(path.clone())?;

    let mode = GameMode::parse(&args.mode)
        .ok_or_else(|| anyhow!("unknown mode '{}': use standard|god|hardcore|sandbox", args.mode))?;

    let season = SeasonId(args.season);

    {
        let store = app.store()?;
        store.init_metadata(season)?;

        if !used_seed {
            // No seed available — fall back to M1 stub so `status` round-trips.
            let stub_team = Team {
                id: TeamId(0),
                abbrev: args.team.to_uppercase(),
                city: String::from("Placeholder City"),
                name: String::from("Placeholders"),
                conference: Conference::East,
                division: Division::Atlantic,
                gm: GMPersonality::from_archetype("Placeholder GM", GMArchetype::Conservative),
                roster: vec![],
                draft_picks: vec![],
                coach: nba3k_core::Coach::default_for(&args.team.to_uppercase()),
            };
            store.upsert_team(&stub_team)?;
        }

        let user_team_id = store
            .find_team_by_abbrev(&args.team)?
            .ok_or_else(|| anyhow!("team '{}' not found in seed", args.team))?;

        let state = SeasonState {
            season,
            phase: SeasonPhase::PreSeason,
            day: 0,
            user_team: user_team_id,
            mode,
            rng_seed: args.seed,
        };
        store.save_season_state(&state)?;
    }

    // Generate + persist 82-game schedule so subsequent sim-day commands can
    // pull pending games from the DB. Skip if no real teams (stub mode).
    let teams = app.store()?.list_teams()?;
    if teams.len() == 30 {
        generate_and_store_schedule(app, season, args.seed)?;
        init_standings(app, season)?;
    } else {
        eprintln!(
            "note: only {} teams in seed (need 30) — schedule generation skipped",
            teams.len()
        );
    }

    println!(
        "created save {} (team={} mode={} season={} seed_used={})",
        path.display(),
        args.team.to_uppercase(),
        mode,
        season,
        used_seed
    );
    Ok(())
}

fn cmd_load(app: &mut AppState, path: PathBuf) -> Result<()> {
    if !path.exists() {
        bail!("no such save: {}", path.display());
    }
    app.open_path(path.clone())?;
    println!("loaded {}", path.display());
    Ok(())
}

fn cmd_save(app: &mut AppState) -> Result<()> {
    let store = app.store()?;
    store.conn().execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    println!("save flushed: {}", store.path().display());
    Ok(())
}

#[derive(Serialize)]
struct StatusReport {
    save_path: String,
    season: u16,
    phase: String,
    day: u32,
    user_team: String,
    user_team_id: u8,
    mode: String,
    rng_seed: u64,
    teams_count: u32,
    players_count: u32,
    schedule_total: u32,
    schedule_unplayed: u32,
    app_version: String,
    created_at: Option<String>,
}

fn cmd_status(app: &mut AppState, JsonFlag { json: as_json }: JsonFlag) -> Result<()> {
    let store = app.store().context("opening save for status")?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("save has no season_state"))?;
    let teams = store.count_teams()?;
    let players = store.count_players()?;
    let app_version = store
        .get_meta("app_version")?
        .unwrap_or_else(|| "unknown".into());
    let created_at = store.get_meta("created_at")?;
    let team_abbrev = store
        .team_abbrev(state.user_team)?
        .unwrap_or_else(|| "???".into());
    let schedule_total = store.count_schedule()?;
    let schedule_unplayed = store.count_unplayed()?;

    let report = StatusReport {
        save_path: store.path().display().to_string(),
        season: state.season.0,
        phase: format!("{:?}", state.phase),
        day: state.day,
        user_team: team_abbrev,
        user_team_id: state.user_team.0,
        mode: state.mode.to_string(),
        rng_seed: state.rng_seed,
        teams_count: teams,
        players_count: players,
        schedule_total,
        schedule_unplayed,
        app_version,
        created_at,
    };

    if as_json {
        println!("{}", serde_json::to_string_pretty(&json!(report))?);
    } else {
        println!("save:     {}", report.save_path);
        println!("season:   {} ({})", report.season, report.phase);
        println!("day:      {}", report.day);
        println!("team:     {} (id={})", report.user_team, report.user_team_id);
        println!("mode:     {}", report.mode);
        println!("seed:     {}", report.rng_seed);
        println!(
            "teams:    {} | players: {}",
            report.teams_count, report.players_count
        );
        println!(
            "schedule: {} games ({} unplayed)",
            report.schedule_total, report.schedule_unplayed
        );
        println!("version:  {}", report.app_version);
        if let Some(c) = &report.created_at {
            println!("created:  {}", c);
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// schedule init
// ----------------------------------------------------------------------

fn generate_and_store_schedule(app: &mut AppState, season: SeasonId, seed: u64) -> Result<()> {
    let teams = app.store()?.list_teams()?;
    if teams.len() != 30 {
        bail!("schedule generator needs 30 teams; found {}", teams.len());
    }
    let schedule = Schedule::generate(season, seed, &teams);
    let rows: Vec<_> = schedule
        .games
        .iter()
        .map(|g| (g.id.0, g.season, g.date, g.home, g.away))
        .collect();
    app.store()?.bulk_insert_schedule(&rows)?;
    Ok(())
}

fn init_standings(app: &mut AppState, season: SeasonId) -> Result<()> {
    let teams = app.store()?.list_teams()?;
    let store = app.store()?;
    for t in &teams {
        store.upsert_standing(t.id, season, 0, 0, None)?;
    }
    Ok(())
}

// ----------------------------------------------------------------------
// sim-day / sim-to
// ----------------------------------------------------------------------

fn cmd_sim_day(app: &mut AppState, count: u32) -> Result<()> {
    sim_n_days(app, count, false)
}

fn cmd_sim_to(app: &mut AppState, phase_arg: &str) -> Result<()> {
    let target = match phase_arg.to_ascii_lowercase().as_str() {
        "regular" => SeasonPhase::Regular,
        "regular-end" | "playoffs" => SeasonPhase::Playoffs,
        "trade-deadline" => SeasonPhase::TradeDeadlinePassed,
        other => bail!("unknown phase '{}': use regular|regular-end|playoffs|trade-deadline", other),
    };
    sim_until_phase(app, target)
}

fn sim_until_phase(app: &mut AppState, target: SeasonPhase) -> Result<()> {
    let cap = 365u32; // sanity guard so we never infinite-loop
    let mut iter = 0u32;
    loop {
        let state = current_state(app)?;
        if state.phase == target {
            break;
        }
        if matches!(state.phase, SeasonPhase::Playoffs | SeasonPhase::OffSeason)
            && target != state.phase
        {
            bail!("season already past target phase {:?}", target);
        }
        sim_n_days(app, 1, true)?;
        iter += 1;
        if iter > cap {
            bail!("sim-to bailing: exceeded {} sim days without reaching {:?}", cap, target);
        }
    }
    println!("reached phase {:?}", target);
    Ok(())
}

fn sim_n_days(app: &mut AppState, count: u32, quiet: bool) -> Result<()> {
    let mut state = current_state(app)?;
    let total_teams = app.store()?.count_teams()?;
    if total_teams != 30 {
        bail!("cannot sim: save has {} teams (need 30 — was the seed missing?)", total_teams);
    }

    let teams = app.store()?.list_teams()?;
    let teams_by_id: std::collections::HashMap<TeamId, Team> =
        teams.iter().cloned().map(|t| (t.id, t)).collect();
    let engine = pick_engine("statistical");

    // Re-seed the RNG deterministically per-day from `state.rng_seed + state.day`.
    // This keeps sims reproducible across save/load cycles.
    let mut games_played = 0u32;
    let mut days_run = 0u32;

    for _ in 0..count {
        let day_index = state.day;
        let date = day_index_to_date(day_index);

        // PreSeason days: just advance the counter, no games.
        if matches!(state.phase, SeasonPhase::PreSeason) {
            state.day += 1;
            if state.day > season_phases::PRESEASON_LAST_DAY {
                state.phase = SeasonPhase::Regular;
            }
            app.store()?.save_season_state(&state)?;
            days_run += 1;
            continue;
        }

        // Regular / TradeDeadlinePassed: simulate today's games.
        let pending = app.store()?.pending_games_through(date)?;
        let today: Vec<_> = pending.into_iter().filter(|g| g.date == date).collect();

        let mut day_seed = state.rng_seed.wrapping_add(state.day as u64);
        for game_row in today {
            let mut rng = ChaCha8Rng::seed_from_u64(day_seed);
            day_seed = day_seed.wrapping_add(1);

            let home_team = teams_by_id
                .get(&game_row.home)
                .ok_or_else(|| anyhow!("missing home team {}", game_row.home))?;
            let away_team = teams_by_id
                .get(&game_row.away)
                .ok_or_else(|| anyhow!("missing away team {}", game_row.away))?;

            let home_snap = build_snapshot(app, home_team)?;
            let away_snap = build_snapshot(app, away_team)?;

            let ctx = GameContext {
                game_id: GameId(game_row.game_id),
                season: state.season,
                date,
                is_playoffs: false,
                home_back_to_back: false,
                away_back_to_back: false,
            };
            let result = engine.simulate_game(&home_snap, &away_snap, &ctx, &mut rng);
            app.store()?.record_game(&result)?;
            games_played += 1;
        }

        // Trade deadline crossing.
        if state.phase == SeasonPhase::Regular && season_phases::is_after_trade_deadline(date) {
            state.phase = SeasonPhase::TradeDeadlinePassed;
        }

        state.day += 1;
        days_run += 1;
        app.store()?.save_season_state(&state)?;

        // Check phase advancement against schedule completion.
        if regular_complete(app, state.season)? {
            state.phase = SeasonPhase::Playoffs;
            app.store()?.save_season_state(&state)?;
            break;
        }
    }

    // Recompute standings from `games` table → write `standings` rows.
    rebuild_standings(app, state.season)?;

    if !quiet {
        println!(
            "simulated {} day(s); {} game(s) played; phase={:?} day={}",
            days_run, games_played, state.phase, state.day
        );
    }
    Ok(())
}

fn current_state(app: &mut AppState) -> Result<SeasonState> {
    app.store()?
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state in save"))
}

fn day_index_to_date(day: u32) -> NaiveDate {
    let start = NaiveDate::from_ymd_opt(2025, 10, 14).expect("valid"); // preseason ~ Oct 14
    start + chrono::Duration::days(day as i64)
}

fn regular_complete(app: &mut AppState, season: SeasonId) -> Result<bool> {
    let store = app.store()?;
    let teams = store.list_teams()?;
    let mut st = Standings::new(&teams);
    for g in store.read_games(season)? {
        st.record_game_result(&g);
    }
    let target = store.scheduled_games_per_team()?;
    Ok(st.records.iter().all(|(team, rec)| {
        let want = target.get(team).copied().unwrap_or(0);
        (rec.games_played() as u32) >= want
    }))
}

fn rebuild_standings(app: &mut AppState, season: SeasonId) -> Result<()> {
    let store = app.store()?;
    let teams = store.list_teams()?;
    let mut st = Standings::new(&teams);
    for g in store.read_games(season)? {
        st.record_game_result(&g);
    }
    for (team, rec) in &st.records {
        store.upsert_standing(*team, season, rec.wins, rec.losses, Some(rec.conf_rank))?;
    }
    Ok(())
}

// ----------------------------------------------------------------------
// snapshot construction
// ----------------------------------------------------------------------

fn build_snapshot(app: &mut AppState, team: &Team) -> Result<TeamSnapshot> {
    use std::path::Path;
    use std::sync::OnceLock;
    static STAR_ROSTER: OnceLock<nba3k_models::star_protection::StarRoster> = OnceLock::new();
    let roster_index = STAR_ROSTER.get_or_init(|| {
        nba3k_models::star_protection::load_star_roster(Path::new(
            nba3k_models::star_protection::STAR_ROSTER_PATH,
        ))
        .unwrap_or_default()
    });

    let mut roster = app.store()?.roster_for_team(team.id)?;
    // Franchise tags take precedence over scrape-OVR ranking — the source data
    // compresses ratings so a tagged Luka must sit ahead of an untagged Hayes.
    roster.sort_by(|a, b| {
        let a_star = roster_index.is_tagged(&team.abbrev, &a.name);
        let b_star = roster_index.is_tagged(&team.abbrev, &b.name);
        b_star.cmp(&a_star).then_with(|| b.overall.cmp(&a.overall))
    });
    let top: Vec<_> = roster.into_iter().take(8).collect();

    // Eight-man rotation minute share, summing to ~5.0 (5 men × 1.0 each).
    // NBA starter avg ~36 mins (= 0.75 share). Sixth man ~26 mins. Bench ~16.
    let weights: [f32; 8] = [0.78, 0.74, 0.70, 0.62, 0.55, 0.40, 0.30, 0.30];
    // Usage shares sum to ~1.0. Top-1 at 0.27 (real NBA: Doncic ~33%, but
    // PG-distributor archetype assumes 0.22; we land in between to keep
    // the usage-excess creator bonus from over-firing).
    let usage: [f32; 8] = [0.27, 0.22, 0.17, 0.14, 0.09, 0.05, 0.03, 0.03];

    let team_overall = if top.is_empty() {
        50
    } else {
        (top.iter().map(|p| p.overall as u32).sum::<u32>() / top.len() as u32) as u8
    };

    let rotation: Vec<RotationSlot> = top
        .iter()
        .enumerate()
        .map(|(i, p)| RotationSlot {
            player: p.id,
            name: p.name.clone(),
            position: p.primary_position,
            minutes_share: weights.get(i).copied().unwrap_or(0.4),
            usage: usage.get(i).copied().unwrap_or(0.05),
            ratings: p.ratings,
            age: p.age,
            overall: p.overall,
            potential: p.potential,
        })
        .collect();

    Ok(TeamSnapshot {
        id: team.id,
        abbrev: team.abbrev.clone(),
        overall: team_overall,
        home_court_advantage: 2.0,
        rotation,
    })
}

// ----------------------------------------------------------------------
// standings / roster / player
// ----------------------------------------------------------------------

#[derive(Serialize)]
struct StandingsRow {
    rank: u32,
    team: String,
    team_id: u8,
    conference: String,
    division: String,
    wins: u16,
    losses: u16,
    pct: f32,
}

fn cmd_standings(app: &mut AppState, JsonFlag { json: as_json }: JsonFlag) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;
    let rows = store.read_standings(state.season)?;

    let mapped: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let gp = r.wins + r.losses;
            let pct = if gp == 0 { 0.0 } else { r.wins as f32 / gp as f32 };
            StandingsRow {
                rank: (i as u32) + 1,
                team: r.abbrev.clone(),
                team_id: r.team.0,
                conference: format!("{:?}", r.conference),
                division: format!("{:?}", r.division),
                wins: r.wins,
                losses: r.losses,
                pct,
            }
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string_pretty(&mapped)?);
    } else {
        println!(
            "{:>3}  {:<3}  {:<4}  {:<10}  {:>4}  {:>4}  {:>5}",
            "#", "TM", "CONF", "DIV", "W", "L", "PCT"
        );
        for r in mapped {
            println!(
                "{:>3}  {:<3}  {:<4}  {:<10}  {:>4}  {:>4}  {:>5.3}",
                r.rank, r.team, r.conference, r.division, r.wins, r.losses, r.pct
            );
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct RosterEntry {
    id: u32,
    name: String,
    pos: String,
    age: u8,
    overall: u8,
    potential: u8,
}

fn cmd_roster(app: &mut AppState, team: Option<String>, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;
    let team_id = match team {
        Some(abbrev) => store
            .find_team_by_abbrev(&abbrev)?
            .ok_or_else(|| anyhow!("no team '{}'", abbrev))?,
        None => state.user_team,
    };
    let mut roster = store.roster_for_team(team_id)?;
    roster.sort_by(|a, b| b.overall.cmp(&a.overall));
    let abbrev = store.team_abbrev(team_id)?.unwrap_or_else(|| "???".into());

    let mapped: Vec<_> = roster
        .iter()
        .map(|p| RosterEntry {
            id: p.id.0,
            name: p.name.clone(),
            pos: p.primary_position.to_string(),
            age: p.age,
            overall: p.overall,
            potential: p.potential,
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string_pretty(&mapped)?);
    } else {
        println!("{} roster ({} players):", abbrev, mapped.len());
        println!(
            "{:<5}  {:<28}  {:<3}  {:>3}  {:>3}  {:>3}",
            "ID", "NAME", "POS", "AGE", "OVR", "POT"
        );
        for p in mapped {
            println!(
                "{:<5}  {:<28}  {:<3}  {:>3}  {:>3}  {:>3}",
                p.id, p.name, p.pos, p.age, p.overall, p.potential
            );
        }
    }
    Ok(())
}

pub fn cmd_player(app: &mut AppState, name: &str, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let p = store
        .find_player_by_name(name)?
        .ok_or_else(|| anyhow!("no player matching '{}'", name))?;
    let team = match p.team {
        Some(id) => store.team_abbrev(id)?.unwrap_or_else(|| "???".into()),
        None => "FA".into(),
    };

    if as_json {
        let v = json!({
            "id": p.id.0,
            "name": p.name,
            "position": p.primary_position.to_string(),
            "age": p.age,
            "overall": p.overall,
            "potential": p.potential,
            "team": team,
            "ratings": p.ratings,
            "no_trade_clause": p.no_trade_clause,
            "trade_kicker_pct": p.trade_kicker_pct,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("{} ({}) — {}", p.name, p.primary_position, team);
        println!("age {} | OVR {} | POT {}", p.age, p.overall, p.potential);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// LeagueSnapshot construction
// ----------------------------------------------------------------------

struct OwnedSnapshot {
    teams: Vec<Team>,
    players_by_id: HashMap<PlayerId, Player>,
    picks_by_id: HashMap<DraftPickId, DraftPick>,
    standings: HashMap<TeamId, TeamRecordSummary>,
    season: SeasonId,
    phase: SeasonPhase,
    date: NaiveDate,
    league_year: LeagueYear,
}

impl OwnedSnapshot {
    fn view(&self) -> LeagueSnapshot<'_> {
        LeagueSnapshot {
            current_season: self.season,
            current_phase: self.phase,
            current_date: self.date,
            league_year: self.league_year,
            teams: &self.teams,
            players_by_id: &self.players_by_id,
            picks_by_id: &self.picks_by_id,
            standings: &self.standings,
        }
    }
}

fn build_league_snapshot(app: &mut AppState) -> Result<OwnedSnapshot> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state in save"))?;
    let teams = store.list_teams()?;
    let players: Vec<Player> = store.all_active_players()?;
    let picks = store.all_picks()?;
    let standing_rows = store.read_standings(state.season)?;

    let players_by_id: HashMap<PlayerId, Player> =
        players.into_iter().map(|p| (p.id, p)).collect();
    let picks_by_id: HashMap<DraftPickId, DraftPick> =
        picks.into_iter().map(|p| (p.id, p)).collect();

    let mut standings: HashMap<TeamId, TeamRecordSummary> = HashMap::new();
    for (i, r) in standing_rows.iter().enumerate() {
        standings.insert(
            r.team,
            TeamRecordSummary {
                wins: r.wins,
                losses: r.losses,
                conf_rank: r.conf_rank.unwrap_or((i as u8) + 1),
                point_diff: 0,
            },
        );
    }
    for t in &teams {
        standings.entry(t.id).or_default();
    }

    let date = day_index_to_date(state.day);
    let league_year = LeagueYear::for_season(state.season)
        .ok_or_else(|| anyhow!("no LeagueYear constants for season {}", state.season.0))?;

    Ok(OwnedSnapshot {
        teams,
        players_by_id,
        picks_by_id,
        standings,
        season: state.season,
        phase: state.phase,
        date,
        league_year,
    })
}

// ----------------------------------------------------------------------
// trade dispatch
// ----------------------------------------------------------------------

fn cmd_trade(app: &mut AppState, action: TradeAction) -> Result<()> {
    match action {
        TradeAction::Propose { from, to, send, receive, json } => {
            cmd_trade_propose(app, &from, &to, &send, &receive, json)
        }
        TradeAction::List(JsonFlag { json }) => cmd_trade_list(app, json),
        TradeAction::Respond { id, action, json } => {
            cmd_trade_respond(app, TradeId(id), &action, json)
        }
        TradeAction::Chain { id, json } => cmd_trade_chain(app, TradeId(id), json),
    }
}

fn resolve_team(store: &nba3k_store::Store, abbrev: &str) -> Result<TeamId> {
    store
        .find_team_by_abbrev(abbrev)?
        .ok_or_else(|| anyhow!("no team '{}'", abbrev))
}

/// Resolve player tokens from a team's roster, falling back to fuzzy
/// `find_player_by_name` if the exact roster scan misses (handles two-way
/// players or roster moves that haven't synced).
fn resolve_player(
    store: &nba3k_store::Store,
    team: TeamId,
    token: &str,
) -> Result<PlayerId> {
    let token = token.trim();
    if looks_like_pick_token(token) {
        bail!("pick assets ('{}') not supported until M5", token);
    }
    // Try team roster first.
    let roster = store.roster_for_team(team)?;
    if let Some(p) = roster
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(token))
    {
        return Ok(p.id);
    }
    if let Some(p) = roster
        .iter()
        .find(|p| p.name.to_ascii_lowercase().contains(&token.to_ascii_lowercase()))
    {
        return Ok(p.id);
    }
    // Fall back to global lookup.
    if let Some(p) = store.find_player_by_name(token)? {
        if p.team == Some(team) {
            return Ok(p.id);
        }
        bail!(
            "player '{}' is on {} not {}",
            p.name,
            p.team
                .and_then(|t| store.team_abbrev(t).ok().flatten())
                .unwrap_or_else(|| "FA".into()),
            store.team_abbrev(team)?.unwrap_or_else(|| "???".into())
        );
    }
    bail!("no player '{}' on team", token);
}

fn looks_like_pick_token(s: &str) -> bool {
    // crude: starts with 4 digits then '-'
    let bytes = s.as_bytes();
    bytes.len() >= 5 && bytes[..4].iter().all(|c| c.is_ascii_digit()) && bytes[4] == b'-'
}

fn cmd_trade_propose(
    app: &mut AppState,
    from: &str,
    to: &str,
    send: &[String],
    receive: &[String],
    as_json: bool,
) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;

    let from_id = resolve_team(store, from)?;
    let to_id = resolve_team(store, to)?;
    if from_id == to_id {
        bail!("cannot trade with yourself");
    }

    let send_ids: Vec<PlayerId> = send
        .iter()
        .map(|t| resolve_player(store, from_id, t))
        .collect::<Result<_>>()?;
    let receive_ids: Vec<PlayerId> = receive
        .iter()
        .map(|t| resolve_player(store, to_id, t))
        .collect::<Result<_>>()?;

    let mut assets_by_team = IndexMap::new();
    assets_by_team.insert(
        from_id,
        TradeAssets { players_out: send_ids, picks_out: vec![], cash_out: Cents::ZERO },
    );
    assets_by_team.insert(
        to_id,
        TradeAssets { players_out: receive_ids, picks_out: vec![], cash_out: Cents::ZERO },
    );

    let offer = TradeOffer {
        id: TradeId(0),
        initiator: from_id,
        assets_by_team,
        round: 1,
        parent: None,
    };

    let god_active = state.mode == GameMode::God || app.force_god;

    // Build the snapshot once and run the engine.
    let snap_owned = build_league_snapshot(app)?;
    let snapshot = snap_owned.view();

    let final_state = if god_active {
        // God mode short-circuits: user's offer is always Accepted, no CBA gate.
        NegotiationState::Accepted(offer.clone())
    } else {
        let mut rng = ChaCha8Rng::seed_from_u64(state.rng_seed.wrapping_add(state.day as u64));
        // Round 1: receiving team evaluates.
        let evaluation = evaluate_mod::evaluate(&offer, to_id, &snapshot, &mut rng);
        let initial_chain_state = NegotiationState::Open { chain: vec![offer.clone()] };
        match evaluation.verdict {
            Verdict::Accept => NegotiationState::Accepted(offer),
            Verdict::Reject(reason) => {
                NegotiationState::Rejected { final_offer: offer, reason }
            }
            Verdict::Counter(_) => {
                // Generate the counter via Worker D.
                match negotiate_mod::generate_counter(&offer, to_id, &snapshot, &mut rng) {
                    Some(counter) => match initial_chain_state {
                        NegotiationState::Open { mut chain } => {
                            chain.push(counter);
                            NegotiationState::Open { chain }
                        }
                        other => other,
                    },
                    None => NegotiationState::Rejected {
                        final_offer: offer,
                        reason: RejectReason::BadFaith,
                    },
                }
            }
        }
    };

    let store = app.store()?;
    let id = store.insert_trade_chain(state.season, state.day, &final_state)?;
    print_chain_outcome(id, &final_state, as_json, store)?;
    Ok(())
}

fn cmd_trade_list(app: &mut AppState, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;
    let chains = store.list_trade_chains(state.season)?;

    let rows: Vec<_> = chains
        .iter()
        .map(|(id, st)| chain_summary_row(*id, st, store))
        .collect::<Result<Vec<_>>>()?;

    if as_json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!(
            "{:>4}  {:<8}  {:>5}  {:<10}",
            "ID", "STATUS", "ROUND", "TEAMS"
        );
        for r in &rows {
            println!(
                "{:>4}  {:<8}  {:>5}  {}",
                r.id, r.status, r.round, r.teams
            );
        }
    }
    Ok(())
}

fn cmd_trade_respond(
    app: &mut AppState,
    id: TradeId,
    action: &str,
    as_json: bool,
) -> Result<()> {
    let snap_owned = build_league_snapshot(app)?;
    let snapshot = snap_owned.view();
    let store = app.store()?;
    let state_chain = store
        .read_trade_chain(id)?
        .ok_or_else(|| anyhow!("no trade chain id={}", id))?;

    let new_state = match action.to_ascii_lowercase().as_str() {
        "accept" => match state_chain {
            NegotiationState::Open { chain } => {
                let last = chain
                    .last()
                    .cloned()
                    .ok_or_else(|| anyhow!("empty chain"))?;
                NegotiationState::Accepted(last)
            }
            other => other,
        },
        "reject" => match state_chain {
            NegotiationState::Open { chain } => {
                let last = chain
                    .last()
                    .cloned()
                    .ok_or_else(|| anyhow!("empty chain"))?;
                NegotiationState::Rejected {
                    final_offer: last,
                    reason: RejectReason::Other("user rejected".into()),
                }
            }
            other => other,
        },
        "counter" => {
            let mut rng = ChaCha8Rng::seed_from_u64(snap_owned.season.0 as u64 ^ id.0);
            negotiate_mod::step(state_chain, &snapshot, &mut rng)
        }
        other => bail!("unknown respond action '{}': use accept|reject|counter", other),
    };

    let store = app.store()?;
    store.update_trade_chain(id, &new_state)?;
    print_chain_outcome(id, &new_state, as_json, store)?;
    Ok(())
}

fn cmd_trade_chain(app: &mut AppState, id: TradeId, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let state_chain = store
        .read_trade_chain(id)?
        .ok_or_else(|| anyhow!("no trade chain id={}", id))?;
    let chain_offers: Vec<&TradeOffer> = match &state_chain {
        NegotiationState::Open { chain } => chain.iter().collect(),
        NegotiationState::Accepted(o) => vec![o],
        NegotiationState::Rejected { final_offer, .. } => vec![final_offer],
        NegotiationState::Stalled => vec![],
    };

    let rendered: Vec<serde_json::Value> = chain_offers
        .iter()
        .map(|o| {
            let by_team: Vec<_> = o
                .assets_by_team
                .iter()
                .map(|(team, assets)| {
                    let abbrev = store
                        .team_abbrev(*team)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| format!("{}", team.0));
                    let players: Vec<_> = assets
                        .players_out
                        .iter()
                        .map(|pid| {
                            store
                                .player_name(*pid)
                                .ok()
                                .flatten()
                                .unwrap_or_else(|| format!("#{}", pid.0))
                        })
                        .collect();
                    json!({"team": abbrev, "players_out": players})
                })
                .collect();
            json!({"round": o.round, "by_team": by_team})
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string_pretty(&rendered)?);
    } else {
        println!("trade #{} — {} offer(s):", id, rendered.len());
        for (i, off) in rendered.iter().enumerate() {
            println!("  round {}: {}", i + 1, off);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ChainSummary {
    id: u64,
    status: String,
    round: u8,
    verdict: String,
    teams: String,
}

fn chain_summary_row(
    id: TradeId,
    st: &NegotiationState,
    store: &nba3k_store::Store,
) -> Result<ChainSummary> {
    let (status, verdict, latest, teams_str) = match st {
        NegotiationState::Open { chain } => {
            let latest = chain.last().cloned();
            ("open".to_string(), "Counter".to_string(), latest, teams_for(chain.last(), store))
        }
        NegotiationState::Accepted(o) => (
            "accepted".to_string(),
            "Accept".to_string(),
            Some(o.clone()),
            teams_for(Some(o), store),
        ),
        NegotiationState::Rejected { final_offer, reason } => (
            "rejected".to_string(),
            format!("Reject({:?})", reason),
            Some(final_offer.clone()),
            teams_for(Some(final_offer), store),
        ),
        NegotiationState::Stalled => (
            "stalled".to_string(),
            "Stalled".to_string(),
            None,
            String::new(),
        ),
    };
    let round = latest.as_ref().map(|o| o.round).unwrap_or(0);
    Ok(ChainSummary {
        id: id.0,
        status,
        round,
        verdict,
        teams: teams_str,
    })
}

fn teams_for(offer: Option<&TradeOffer>, store: &nba3k_store::Store) -> String {
    let Some(o) = offer else { return String::new() };
    o.assets_by_team
        .keys()
        .map(|t| store.team_abbrev(*t).ok().flatten().unwrap_or_else(|| format!("{}", t.0)))
        .collect::<Vec<_>>()
        .join("/")
}

fn print_chain_outcome(
    id: TradeId,
    st: &NegotiationState,
    as_json: bool,
    store: &nba3k_store::Store,
) -> Result<()> {
    let summary = chain_summary_row(id, st, store)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "trade #{} — verdict: {} | status: {} | round: {} | teams: {}",
            summary.id, summary.verdict.to_lowercase(), summary.status, summary.round, summary.teams
        );
    }
    Ok(())
}

// ----------------------------------------------------------------------
// dev calibrate-trade
// ----------------------------------------------------------------------

fn cmd_dev(app: &mut AppState, action: DevAction) -> Result<()> {
    match action {
        DevAction::CalibrateTrade { runs, json } => cmd_dev_calibrate(app, runs, json),
    }
}

fn cmd_dev_calibrate(app: &mut AppState, runs: u32, as_json: bool) -> Result<()> {
    use rand::seq::SliceRandom;

    let snap_owned = build_league_snapshot(app)?;
    let snapshot = snap_owned.view();

    let mut rng = ChaCha8Rng::seed_from_u64(0xC0FFEE);
    let team_ids: Vec<TeamId> = snap_owned.teams.iter().map(|t| t.id).collect();
    if team_ids.len() < 2 {
        bail!("need ≥2 teams for calibration; found {}", team_ids.len());
    }

    let mut accept = 0u32;
    let mut reject = 0u32;
    let mut counter = 0u32;
    let mut by_archetype: HashMap<String, (u32, u32, u32)> = HashMap::new();

    for _ in 0..runs {
        let pair: Vec<&TeamId> = team_ids.choose_multiple(&mut rng, 2).collect();
        let (a, b) = (*pair[0], *pair[1]);

        let a_roster: Vec<&Player> = snapshot.roster(a);
        let b_roster: Vec<&Player> = snapshot.roster(b);
        if a_roster.is_empty() || b_roster.is_empty() {
            continue;
        }
        let a_pick = a_roster[rng.gen_range(0..a_roster.len())].id;
        let b_pick = b_roster[rng.gen_range(0..b_roster.len())].id;

        let mut assets = IndexMap::new();
        assets.insert(
            a,
            TradeAssets { players_out: vec![a_pick], picks_out: vec![], cash_out: Cents::ZERO },
        );
        assets.insert(
            b,
            TradeAssets { players_out: vec![b_pick], picks_out: vec![], cash_out: Cents::ZERO },
        );
        let offer = TradeOffer {
            id: TradeId(0),
            initiator: a,
            assets_by_team: assets,
            round: 1,
            parent: None,
        };
        let evaluation = evaluate_mod::evaluate(&offer, b, &snapshot, &mut rng);

        let arch = snapshot
            .team(b)
            .map(|t| format!("{:?}", t.gm.archetype))
            .unwrap_or_else(|| "Unknown".into());
        let entry = by_archetype.entry(arch).or_insert((0, 0, 0));
        match evaluation.verdict {
            Verdict::Accept => {
                accept += 1;
                entry.0 += 1;
            }
            Verdict::Reject(_) => {
                reject += 1;
                entry.1 += 1;
            }
            Verdict::Counter(_) => {
                counter += 1;
                entry.2 += 1;
            }
        }
    }

    let total = accept + reject + counter;
    let pct = |n: u32| if total == 0 { 0.0 } else { (n as f64 / total as f64) * 100.0 };

    if as_json {
        let mut by_arch_json = serde_json::Map::new();
        for (k, (a, r, c)) in &by_archetype {
            by_arch_json.insert(
                k.clone(),
                json!({"accept": a, "reject": r, "counter": c}),
            );
        }
        let v = json!({
            "total": total,
            "accept": accept,
            "reject": reject,
            "counter": counter,
            "by_archetype": by_arch_json,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("calibration ({} runs):", total);
        println!(
            "  accept:  {:>4} ({:>5.1}%)",
            accept,
            pct(accept)
        );
        println!(
            "  reject:  {:>4} ({:>5.1}%)",
            reject,
            pct(reject)
        );
        println!(
            "  counter: {:>4} ({:>5.1}%)",
            counter,
            pct(counter)
        );
        println!("by archetype:");
        let mut entries: Vec<_> = by_archetype.iter().collect();
        entries.sort_by_key(|(k, _)| k.clone());
        for (k, (a, r, c)) in entries {
            println!(
                "  {:<14} accept={:>3}  reject={:>3}  counter={:>3}",
                k, a, r, c
            );
        }
    }
    Ok(())
}
