use crate::cli::{
    CoachAction, Command, DevAction, DraftAction, FaAction, JsonFlag, NewArgs, NotesAction,
    PlayoffsAction, SavesAction, TradeAction,
};
use crate::state::AppState;
use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use indexmap::IndexMap;
use nba3k_core::*;
use nba3k_season::{phases as season_phases, schedule::Schedule, standings::Standings};
use nba3k_sim::{pick_engine, roll_injuries_from_box, tick_injury, GameContext, RotationSlot, TeamSnapshot};
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

/// Day-of-season marker for the All-Star Game (M15-A). When `state.day`
/// first reaches/passes this value, the simulator runs `compute_all_star`
/// once per season and persists the roster + a news entry.
const ALL_STAR_DAY: u32 = 41;

/// Day-of-season markers for the NBA Cup (M16-A). Each one is an
/// idempotent ratchet: the trigger only fires when the corresponding
/// cup_match rows for that round are still empty.
const CUP_GROUP_DAY: u32 = 30;
const CUP_QF_DAY: u32 = 45;
const CUP_SF_DAY: u32 = 53;
const CUP_FINAL_DAY: u32 = 55;

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
        Command::SimWeek { no_pause } => cmd_sim_paced(app, 7, !no_pause, "week"),
        Command::SimMonth { no_pause } => cmd_sim_paced(app, 30, !no_pause, "month"),
        Command::Standings { season, json } => cmd_standings(app, season, JsonFlag { json }),
        Command::Roster { team, json } => cmd_roster(app, team, json),
        Command::RosterSetRole { player, role } => cmd_roster_set_role(app, &player, &role),
        Command::Player { name, json } => cmd_player(app, &name, json),
        Command::Trade(args) => cmd_trade(app, args.action),
        Command::Dev(args) => cmd_dev(app, args.action),
        Command::Draft(args) => match args.action {
            DraftAction::Board(JsonFlag { json }) => cmd_draft_board(app, json),
            DraftAction::Order(JsonFlag { json }) => cmd_draft_order(app, json),
            DraftAction::Sim(JsonFlag { json }) => cmd_draft_sim(app, json),
            DraftAction::Pick { player } => cmd_draft_pick(app, &player),
        },
        Command::Chemistry { team, json } => cmd_chemistry(app, &team, json),
        Command::Awards { season, json } => cmd_awards(app, season, json),
        Command::Playoffs(args) => match args.action {
            PlayoffsAction::Bracket(JsonFlag { json }) => cmd_playoffs_bracket(app, json),
            PlayoffsAction::Sim(JsonFlag { json }) => cmd_playoffs_sim(app, json),
        },
        Command::SeasonSummary(JsonFlag { json }) => cmd_season_summary(app, json),
        Command::SeasonAdvance(JsonFlag { json }) => cmd_season_advance(app, json),
        Command::Messages(JsonFlag { json }) => cmd_messages(app, json),
        Command::Career { name, json } => cmd_career(app, &name, json),
        Command::Fa(args) => match args.action {
            FaAction::List(JsonFlag { json }) => cmd_fa_list(app, json),
            FaAction::Sign { player } => cmd_fa_sign(app, &player),
            FaAction::Cut { player } => cmd_fa_cut(app, &player),
        },
        Command::Training { player, focus } => cmd_training(app, &player, &focus),
        Command::Cap { team, json } => cmd_cap(app, team, json),
        Command::Retire { player } => cmd_retire(app, &player),
        Command::Hof { limit, json } => cmd_hof(app, limit, json),
        Command::AwardsRace(JsonFlag { json }) => cmd_awards_race(app, json),
        Command::News { limit, json } => cmd_news(app, limit, json),
        Command::Coach(args) => match args.action {
            CoachAction::Show { team, json } => cmd_coach_show(app, team, json),
            CoachAction::Fire { team } => cmd_coach_fire(app, team),
            CoachAction::Pool { limit, json } => cmd_coach_pool(app, limit, json),
        },
        Command::Scout { player } => cmd_scout(app, &player),
        Command::Records { scope, stat, limit, json } => {
            cmd_records(app, &scope, &stat, limit, json)
        }
        Command::AllStar { season, json } => cmd_all_star(app, season, json),
        Command::Saves(args) => match args.action {
            SavesAction::List { dir, json } => cmd_saves_list(app, dir, json),
            SavesAction::Show { path, json } => cmd_saves_show(app, path, json),
            SavesAction::Delete { path, yes } => cmd_saves_delete(app, path, yes),
            SavesAction::Export { path, to } => cmd_saves_export(app, path, to),
        },
        Command::Cup { season, json } => cmd_cup(app, season, json),
        Command::Rumors { limit, json } => cmd_rumors(app, limit, json),
        Command::Compare { team_a, team_b, json } => cmd_compare(app, &team_a, &team_b, json),
        Command::Offers { limit, json } => cmd_offers(app, limit, json),
        Command::Extend { player, salary_m, years } => cmd_extend(app, &player, salary_m, years),
        Command::Notes(args) => match args.action {
            NotesAction::Add { player, text } => cmd_notes_add(app, &player, text.as_deref()),
            NotesAction::Remove { player } => cmd_notes_remove(app, &player),
            NotesAction::List { json } => cmd_notes_list(app, json),
        },
        Command::Recap { days, json } => cmd_recap(app, days, json),
        Command::Tui { tv, legacy } => cmd_tui(app, tv, legacy),
    }
}

fn cmd_tui(app: &mut AppState, tv: bool, legacy: bool) -> Result<()> {
    if legacy {
        crate::tui::run_legacy(app)
    } else {
        crate::tui::run(app, tv)
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

    // Stale SQLite sidecars from a previous run will be replayed onto the new
    // file's pages and corrupt it. Wipe `-wal` and `-shm` before copying the
    // seed so SQLite opens cleanly.
    let wal_path = path.with_extension({
        let mut ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        ext.push_str("-wal");
        ext
    });
    let shm_path = path.with_extension({
        let mut ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        ext.push_str("-shm");
        ext
    });
    let _ = std::fs::remove_file(&wal_path);
    let _ = std::fs::remove_file(&shm_path);

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
            .ok_or_else(|| anyhow!("unknown team '{}' (try one of: ATL, BOS, LAL, ...)", args.team))?;

        let state = SeasonState {
            season,
            phase: SeasonPhase::PreSeason,
            day: 0,
            user_team: user_team_id,
            mode,
            rng_seed: args.seed,
        };
        store.save_season_state(&state)?;
        // Mirror the user team into `meta.user_team` so subcommands that
        // need a quick lookup (draft pick, training) don't have to deserialize
        // the full SeasonState every call.
        store.set_meta("user_team", &args.team.to_uppercase())?;
        populate_default_starters(store, user_team_id)?;
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
    {
        let store = app.store()?;
        if let Some(state) = store.load_season_state()? {
            populate_default_starters(store, state.user_team)?;
        }
    }
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
    // Offset game_ids by season so multi-season saves don't collide with
    // prior years' schedule rows on the unique `game_id` index.
    let id_offset: u64 = (season.0 as u64) * 10_000;
    let schedule = Schedule::generate(season, seed, &teams);
    let rows: Vec<_> = schedule
        .games
        .iter()
        .map(|g| (g.id.0 + id_offset, g.season, g.date, g.home, g.away))
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
    let key = phase_arg.to_ascii_lowercase().replace('-', "").replace('_', "");
    // Day-marker targets: skip until state.day reaches the named milestone.
    if let Some(target_day) = match key.as_str() {
        "allstar" | "asg" => Some(ALL_STAR_DAY),
        "cupgroup" => Some(CUP_GROUP_DAY),
        "cupqf" => Some(CUP_QF_DAY),
        "cupsf" => Some(CUP_SF_DAY),
        "cupfinal" | "cup" => Some(CUP_FINAL_DAY),
        _ => None,
    } {
        return sim_until_day(app, target_day);
    }
    // Phase targets — `season-end` / `playoffs-end` are special: sim regular
    // season to Playoffs phase, then auto-run the playoff bracket (since
    // sim-day doesn't progress through playoffs — bracket is event-based),
    // then flip to OffSeason.
    if matches!(key.as_str(), "seasonend" | "playoffsend" | "yearend") {
        // Step 1: ensure we're at or past Playoffs.
        let state = current_state(app)?;
        if !matches!(state.phase, SeasonPhase::Playoffs | SeasonPhase::OffSeason) {
            sim_until_phase(app, SeasonPhase::Playoffs)?;
        }
        // Step 2: auto-run bracket if Finals not yet decided.
        let state = current_state(app)?;
        if state.phase == SeasonPhase::Playoffs {
            let series = app.store()?.read_series(state.season)?;
            let finals_done = series
                .iter()
                .any(|s| s.round == nba3k_season::PlayoffRound::Finals as u8);
            if !finals_done {
                cmd_playoffs_sim(app, false)?;
            }
        }
        // Step 3: flip Playoffs → OffSeason if still pending.
        let mut state = current_state(app)?;
        if state.phase == SeasonPhase::Playoffs {
            state.phase = SeasonPhase::OffSeason;
            app.store()?.save_season_state(&state)?;
        }
        println!("reached phase OffSeason — season {} complete", state.season.0);
        return Ok(());
    }
    let target = match key.as_str() {
        "regular" | "regularseason" => SeasonPhase::Regular,
        "regularend" | "playoffs" => SeasonPhase::Playoffs,
        "tradedeadline" | "tradedeadlinepassed" => SeasonPhase::TradeDeadlinePassed,
        "offseason" => SeasonPhase::OffSeason,
        other => bail!(
            "unknown phase '{}': try regular | regular-end | playoffs | trade-deadline | offseason | all-star | cup-final | season-end",
            other
        ),
    };
    sim_until_phase(app, target)
}

/// Sim until `state.day >= target_day`. Used by named day markers (all-star,
/// cup-final). Bails if season is already past the marker.
pub(crate) fn sim_until_day(app: &mut AppState, target_day: u32) -> Result<()> {
    let cap = 365u32;
    let mut iter = 0u32;
    loop {
        let state = current_state(app)?;
        if state.day >= target_day {
            break;
        }
        if matches!(state.phase, SeasonPhase::Playoffs | SeasonPhase::OffSeason) {
            bail!(
                "season already past day {} (phase={:?}, day={})",
                target_day, state.phase, state.day
            );
        }
        sim_n_days(app, 1, true)?;
        iter += 1;
        if iter > cap {
            bail!("sim-to bailing after {} days; never reached day {}", cap, target_day);
        }
    }
    println!("reached day {}", target_day);
    Ok(())
}

/// Sim N days, pausing early when an interactive event needs the user's
/// attention. v1 pauses on:
///   - new incoming trade offers (read_open_chains_targeting non-empty AND
///     count grew during the sim)
///   - user-team injury added today (news kind=injury for our roster)
fn cmd_sim_paced(app: &mut AppState, days: u32, allow_pause: bool, label: &str) -> Result<()> {
    let initial_state = current_state(app)?;
    let user_team = initial_state.user_team;
    let season = initial_state.season;

    // Snapshot offer count + recent news id BEFORE we start so we can detect
    // *new* events triggered during the sim.
    let baseline_offers = if allow_pause {
        app.store()?
            .read_open_chains_targeting(season, user_team)?
            .len()
    } else {
        0
    };
    let baseline_news_count = if allow_pause {
        app.store()?.recent_news(200)?.len()
    } else {
        0
    };

    let user_player_ids: std::collections::HashSet<PlayerId> = if allow_pause {
        app.store()?
            .roster_for_team(user_team)?
            .into_iter()
            .map(|p| p.id)
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let mut simmed = 0u32;
    let mut pause_reason: Option<String> = None;
    for _ in 0..days {
        let state = current_state(app)?;
        if matches!(state.phase, SeasonPhase::Playoffs | SeasonPhase::OffSeason) {
            pause_reason = Some(format!("season phase changed to {:?}", state.phase));
            break;
        }
        sim_n_days(app, 1, true)?;
        simmed += 1;

        if !allow_pause {
            continue;
        }
        // After this day's sim, check for new pause-worthy events.
        let store = app.store()?;
        let cur_offers = store.read_open_chains_targeting(season, user_team)?.len();
        if cur_offers > baseline_offers {
            pause_reason = Some(format!(
                "{} new trade offer(s) pending — `nba3k offers` to review",
                cur_offers - baseline_offers
            ));
            break;
        }
        let news = store.recent_news(50)?;
        let new_news = &news[..news.len().saturating_sub(baseline_news_count.min(news.len()))];
        if let Some(injury) = new_news.iter().find(|n| {
            n.kind == "injury"
                && user_player_ids
                    .iter()
                    .any(|pid| n.headline.contains(&format!("#{}", pid.0)) || true)
        }) {
            // Coarse match: any injury news during this paced sim — caller
            // can read `news` for details. Avoids tight coupling to news
            // headline format.
            let _ = injury;
            // Filter: only pause if the injury keyword references our team's
            // abbreviation when present. Otherwise skip — most injuries are
            // other teams.
            let user_abbrev = store.team_abbrev(user_team)?.unwrap_or_default();
            let user_injury = new_news.iter().any(|n| {
                n.kind == "injury" && (n.headline.contains(&user_abbrev) || n.body.as_deref().is_some_and(|b| b.contains(&user_abbrev)))
            });
            if user_injury {
                pause_reason = Some(format!(
                    "{} player injured — `nba3k news` to review",
                    user_abbrev
                ));
                break;
            }
        }
    }
    let final_state = current_state(app)?;
    if let Some(reason) = pause_reason {
        println!(
            "sim-{} paused: {}\n  simmed {} day(s); now phase={:?} day={}",
            label, reason, simmed, final_state.phase, final_state.day
        );
    } else {
        println!(
            "sim-{} complete: {} day(s); phase={:?} day={}",
            label, simmed, final_state.phase, final_state.day
        );
    }
    Ok(())
}

pub(crate) fn sim_until_phase(app: &mut AppState, target: SeasonPhase) -> Result<()> {
    let cap = 365u32; // sanity guard so we never infinite-loop
    let mut iter = 0u32;
    let phase_ord = |p: SeasonPhase| -> u8 {
        match p {
            SeasonPhase::PreSeason => 0,
            SeasonPhase::Regular => 1,
            SeasonPhase::TradeDeadlinePassed => 2,
            SeasonPhase::Playoffs => 3,
            SeasonPhase::OffSeason => 4,
            SeasonPhase::Draft => 5,
            SeasonPhase::FreeAgency => 6,
        }
    };
    loop {
        let state = current_state(app)?;
        if state.phase == target {
            break;
        }
        // Bail only if season is genuinely past the target on the linear order.
        if phase_ord(state.phase) > phase_ord(target) {
            bail!(
                "season already past target phase {:?} (current={:?})",
                target, state.phase
            );
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

pub(crate) fn sim_n_days(app: &mut AppState, count: u32, quiet: bool) -> Result<()> {
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

        // Decrement existing injuries before today's games. A player whose
        // counter reaches 0 has their `injury` slot cleared so they re-enter
        // tonight's rotation eligibility.
        decrement_injuries_for_day(app)?;

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
            let new_injuries = roll_injuries_from_box(&result.box_score, &mut rng);
            app.store()?.record_game(&result)?;
            apply_new_injuries(app, &new_injuries)?;
            games_played += 1;
        }

        // Trade deadline crossing — kick off the AI volume spike on the
        // deadline day before flipping to TradeDeadlinePassed.
        if state.phase == SeasonPhase::Regular && season_phases::is_trade_deadline_day(date) {
            let deadline_seed = state.rng_seed.wrapping_add(state.day as u64).wrapping_add(0xDEAD);
            run_ai_trade_volume(app, state.season, state.user_team, deadline_seed)?;
        }

        // M17-A: daily AI offers targeting the user team. Only fires during
        // the regular season — once the trade deadline passes the inbox is
        // closed for new chains.
        if state.phase == SeasonPhase::Regular {
            let offer_seed = state
                .rng_seed
                .wrapping_add(state.day as u64)
                .wrapping_add(0x0FFE_4517);
            run_ai_inbox_offers(app, state.season, state.day, state.user_team, offer_seed)?;
        }

        if state.phase == SeasonPhase::Regular && season_phases::is_after_trade_deadline(date) {
            state.phase = SeasonPhase::TradeDeadlinePassed;
        }

        state.day += 1;
        days_run += 1;
        app.store()?.save_season_state(&state)?;

        // M15-A all-star trigger: at/after the day-41 mid-season marker,
        // fire the All-Star selection once per season. Skip if a roster has
        // already been recorded so re-running sim doesn't double-pick.
        if state.day >= ALL_STAR_DAY
            && state.phase == SeasonPhase::Regular
            && app.store()?.read_all_star(state.season)?.is_empty()
        {
            run_all_star_pass(app, state.season)?;
        }

        // M16-A NBA Cup triggers. Each round is gated by an emptiness
        // check on `cup_match` so re-running sim across the markers is
        // idempotent. Days are intentionally ratchets (>=) rather than
        // exact equals so a sim that overshoots (e.g. sim-day 50 from
        // day 25) still fires the group stage on the same call.
        if state.day >= CUP_GROUP_DAY
            && state.phase == SeasonPhase::Regular
            && app.store()?.read_cup_matches(state.season)?.is_empty()
        {
            run_cup_group_stage(app, &teams, state.season, state.day, state.rng_seed)?;
        }
        let cup_rows = app.store()?.read_cup_matches(state.season)?;
        let group_done = cup_rows.iter().any(|r| r.round == "group");
        let qf_done = cup_rows.iter().any(|r| r.round == "qf");
        let sf_done = cup_rows.iter().any(|r| r.round == "sf");
        let final_done = cup_rows.iter().any(|r| r.round == "final");
        if state.day >= CUP_QF_DAY && group_done && !qf_done {
            run_cup_qf(app, &teams, state.season, state.day, state.rng_seed)?;
        }
        if state.day >= CUP_SF_DAY && qf_done && !sf_done {
            run_cup_sf(app, &teams, state.season, state.day, state.rng_seed)?;
        }
        if state.day >= CUP_FINAL_DAY && sf_done && !final_done {
            run_cup_final(app, &teams, state.season, state.day, state.rng_seed)?;
        }

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

/// Walk every active player; if injured, tick their counter down by 1 and
/// persist. Players whose counter hits 0 have `injury` cleared so they're
/// eligible for tonight's rotation.
fn decrement_injuries_for_day(app: &mut AppState) -> Result<()> {
    let store = app.store()?;
    let players = store.all_active_players()?;
    for mut p in players {
        let Some(status) = p.injury.as_ref() else { continue };
        p.injury = tick_injury(status);
        store.upsert_player(&p)?;
    }
    Ok(())
}

fn apply_new_injuries(
    app: &mut AppState,
    new_injuries: &[(PlayerId, InjuryStatus)],
) -> Result<()> {
    if new_injuries.is_empty() {
        return Ok(());
    }
    let store = app.store()?;
    let active = store.all_active_players()?;
    let mut by_id: HashMap<PlayerId, Player> =
        active.into_iter().map(|p| (p.id, p)).collect();
    for (pid, status) in new_injuries {
        let Some(player) = by_id.get_mut(pid) else { continue };
        if player.injury.is_none() {
            player.injury = Some(status.clone());
            store.upsert_player(player)?;
        }
    }
    Ok(())
}

fn current_state(app: &mut AppState) -> Result<SeasonState> {
    app.store()?
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state in save"))
}

pub(crate) fn day_index_to_date(day: u32) -> NaiveDate {
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

/// M15-A All-Star pass: build mid-season standings + aggregate, run
/// `compute_all_star`, persist East/West starters + reserves, and drop one
/// news row. Caller guards on the day-41 marker + emptiness check; this
/// function is idempotent on the store side because `record_all_star` upserts
/// on (season, player_id).
fn run_all_star_pass(app: &mut AppState, season: SeasonId) -> Result<()> {
    let store = app.store()?;
    let teams = store.list_teams()?;
    let players = store.all_active_players()?;
    let position_of: HashMap<PlayerId, Position> =
        players.iter().map(|p| (p.id, p.primary_position)).collect();
    let player_team: HashMap<PlayerId, TeamId> = players
        .iter()
        .filter_map(|p| p.team.map(|t| (p.id, t)))
        .collect();
    let team_abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();

    let games = store.read_games(season)?;
    let mut standings = Standings::new(&teams);
    for g in &games {
        if !g.is_playoffs {
            standings.record_game_result(g);
        }
    }
    let regular: Vec<_> = games.iter().filter(|g| !g.is_playoffs).cloned().collect();
    let aggregate = nba3k_season::aggregate_season(&regular);

    let rosters = nba3k_season::compute_all_star(
        &aggregate,
        &standings,
        season,
        |pid| position_of.get(&pid).copied(),
        |pid| player_team.get(&pid).copied(),
    );

    // Skip until the eligible pool is deep enough for full conference rosters.
    // `compute_all_star` requires `p.games >= 20` per player, so the day-41
    // marker is the earliest the trigger fires but the roster may still be
    // thin then. If either conference can't field 5 starters + 7 reserves, we
    // bail without writing rows — the next sim-day re-fires the guard.
    // Gate: each conference needs at least 4 starters + 4 reserves so the
    // headline rosters are meaningful. Old gate (strict 5+7) was tripped when
    // the ballot pool ran short on Centers (compute_all_star returns fewer
    // reserves when the positional caps consume slots without enough Cs).
    let east_full = rosters.iter().any(|r| {
        r.conference == nba3k_core::Conference::East
            && r.starters.len() >= 4
            && r.reserves.len() >= 4
    });
    let west_full = rosters.iter().any(|r| {
        r.conference == nba3k_core::Conference::West
            && r.starters.len() >= 4
            && r.reserves.len() >= 4
    });
    if !(east_full && west_full) {
        return Ok(());
    }

    let news_day = store
        .load_season_state()?
        .map(|s| s.day)
        .unwrap_or(ALL_STAR_DAY);

    let mut roster_count = 0usize;
    for roster in &rosters {
        for &pid in &roster.starters {
            store.record_all_star(season, roster.conference, pid, "starter")?;
            roster_count += 1;
        }
        for &pid in &roster.reserves {
            store.record_all_star(season, roster.conference, pid, "reserve")?;
            roster_count += 1;
        }
    }

    // Headline: "All-Star rosters announced — East: <hosts> | West: <hosts>".
    // We use the conference's top starter team-abbrev pair as a quick handle.
    let summarize = |roster: &nba3k_season::AllStarRoster| -> String {
        roster
            .starters
            .iter()
            .filter_map(|pid| player_team.get(pid))
            .filter_map(|tid| team_abbrev.get(tid))
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("/")
    };
    let east_summary = rosters
        .iter()
        .find(|r| r.conference == nba3k_core::Conference::East)
        .map(summarize)
        .unwrap_or_default();
    let west_summary = rosters
        .iter()
        .find(|r| r.conference == nba3k_core::Conference::West)
        .map(summarize)
        .unwrap_or_default();
    let headline = format!(
        "All-Star rosters announced ({} {} selected) — East: {} | West: {}",
        season.0, roster_count, east_summary, west_summary
    );
    store.record_news(season, news_day, "all_star", &headline, None)?;
    Ok(())
}

// ----------------------------------------------------------------------
// M16-A NBA Cup engine
//
// 30 teams split deterministically into 6 groups of 5:
//   east-A/B/C  = East teams sorted by team_id, chunked into 5s
//   west-A/B/C  = West teams sorted by team_id, chunked into 5s
//
// Group stage = round-robin within each group (10 matches × 6 groups = 60).
// KO bracket = 8 teams (6 group winners + 2 best runners-up) → QF → SF →
// Final. All matches go through the same `Engine::simulate_game` the
// regular season uses, with `is_playoffs = false` so the realism engine
// doesn't flip into postseason mode for an exhibition tournament.
// ----------------------------------------------------------------------

/// Stable group-id for the index `(conference, slot)` where slot ∈ 0..3.
fn cup_group_id(conf: Conference, slot: usize) -> &'static str {
    match (conf, slot) {
        (Conference::East, 0) => "east-A",
        (Conference::East, 1) => "east-B",
        (Conference::East, 2) => "east-C",
        (Conference::West, 0) => "west-A",
        (Conference::West, 1) => "west-B",
        _ => "west-C",
    }
}

/// Partition 30 teams into 6 groups of 5. East/West are split first; within
/// each conference teams are sorted by `team_id` ascending and chunked into
/// runs of 5. Returns `[(group_id, Vec<Team>); 6]`.
fn cup_groups(teams: &[Team]) -> Vec<(&'static str, Vec<Team>)> {
    let mut east: Vec<Team> = teams
        .iter()
        .filter(|t| t.conference == Conference::East)
        .cloned()
        .collect();
    let mut west: Vec<Team> = teams
        .iter()
        .filter(|t| t.conference == Conference::West)
        .cloned()
        .collect();
    east.sort_by_key(|t| t.id.0);
    west.sort_by_key(|t| t.id.0);

    let mut out: Vec<(&'static str, Vec<Team>)> = Vec::with_capacity(6);
    for (slot, chunk) in east.chunks(5).enumerate().take(3) {
        out.push((cup_group_id(Conference::East, slot), chunk.to_vec()));
    }
    for (slot, chunk) in west.chunks(5).enumerate().take(3) {
        out.push((cup_group_id(Conference::West, slot), chunk.to_vec()));
    }
    out
}

/// Simulate one cup match via the same statistical engine used for regular
/// season + playoffs. Returns (home_score, away_score). The `seed` is mixed
/// from the season RNG seed and a per-match nonce so reruns are stable.
fn simulate_cup_match(
    app: &mut AppState,
    home: &Team,
    away: &Team,
    season: SeasonId,
    day: u32,
    seed: u64,
) -> Result<(u16, u16)> {
    let home_snap = build_snapshot(app, home)?;
    let away_snap = build_snapshot(app, away)?;
    let engine = pick_engine("statistical");
    let ctx = GameContext {
        // Cup matches don't persist into `games`; this id is decorative.
        game_id: GameId(seed),
        season,
        date: day_index_to_date(day),
        is_playoffs: false,
        home_back_to_back: false,
        away_back_to_back: false,
    };
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let result = engine.simulate_game(&home_snap, &away_snap, &ctx, &mut rng);
    Ok((result.home_score, result.away_score))
}

fn run_cup_group_stage(
    app: &mut AppState,
    teams: &[Team],
    season: SeasonId,
    day: u32,
    rng_seed: u64,
) -> Result<()> {
    let groups = cup_groups(teams);
    let mut nonce: u64 = 0;
    for (gid, group_teams) in &groups {
        for i in 0..group_teams.len() {
            for j in (i + 1)..group_teams.len() {
                let home = &group_teams[i];
                let away = &group_teams[j];
                let seed = rng_seed.wrapping_add(0xC0FFEE).wrapping_add(nonce);
                nonce += 1;
                let (hs, as_) = simulate_cup_match(app, home, away, season, day, seed)?;
                app.store()?.record_cup_match(
                    season,
                    "group",
                    Some(gid),
                    home.id,
                    away.id,
                    hs,
                    as_,
                    day,
                )?;
            }
        }
    }
    Ok(())
}

/// Per-team group-stage record for KO seeding.
#[derive(Debug, Clone)]
struct CupGroupStanding {
    team: TeamId,
    group_id: String,
    wins: u8,
    point_diff: i32,
}

fn cup_group_standings(rows: &[nba3k_store::CupMatchRow]) -> Vec<CupGroupStanding> {
    let mut by_team: HashMap<TeamId, CupGroupStanding> = HashMap::new();
    for r in rows.iter().filter(|r| r.round == "group") {
        let gid = r.group_id.clone().unwrap_or_default();
        let entry_h = by_team.entry(r.home_team).or_insert(CupGroupStanding {
            team: r.home_team,
            group_id: gid.clone(),
            wins: 0,
            point_diff: 0,
        });
        entry_h.point_diff += r.home_score as i32 - r.away_score as i32;
        if r.home_score > r.away_score {
            entry_h.wins += 1;
        }
        let entry_a = by_team.entry(r.away_team).or_insert(CupGroupStanding {
            team: r.away_team,
            group_id: gid,
            wins: 0,
            point_diff: 0,
        });
        entry_a.point_diff += r.away_score as i32 - r.home_score as i32;
        if r.away_score > r.home_score {
            entry_a.wins += 1;
        }
    }
    by_team.into_values().collect()
}

/// 8-team KO field: each group's leader + two best runners-up. Sorting
/// inside a group is by (wins desc, point_diff desc, team_id asc).
fn cup_ko_field(standings: &[CupGroupStanding]) -> Vec<TeamId> {
    let mut by_group: HashMap<String, Vec<CupGroupStanding>> = HashMap::new();
    for row in standings {
        by_group.entry(row.group_id.clone()).or_default().push(row.clone());
    }
    let mut group_ids: Vec<String> = by_group.keys().cloned().collect();
    group_ids.sort();

    let mut winners: Vec<CupGroupStanding> = Vec::new();
    let mut runners: Vec<CupGroupStanding> = Vec::new();
    for gid in &group_ids {
        let mut g = by_group.remove(gid).unwrap_or_default();
        g.sort_by(|a, b| {
            b.wins
                .cmp(&a.wins)
                .then(b.point_diff.cmp(&a.point_diff))
                .then(a.team.0.cmp(&b.team.0))
        });
        if let Some(first) = g.first().cloned() {
            winners.push(first);
        }
        if let Some(second) = g.get(1).cloned() {
            runners.push(second);
        }
    }
    runners.sort_by(|a, b| {
        b.wins
            .cmp(&a.wins)
            .then(b.point_diff.cmp(&a.point_diff))
            .then(a.team.0.cmp(&b.team.0))
    });
    let mut field: Vec<TeamId> = winners.iter().map(|s| s.team).collect();
    let need = 8usize.saturating_sub(field.len());
    for r in runners.iter().take(need) {
        field.push(r.team);
    }
    field.truncate(8);
    field
}

fn run_cup_ko_round(
    app: &mut AppState,
    teams: &[Team],
    season: SeasonId,
    day: u32,
    rng_seed: u64,
    round_label: &str,
    nonce_salt: u64,
    field: &[TeamId],
) -> Result<Vec<TeamId>> {
    let teams_by_id: HashMap<TeamId, Team> =
        teams.iter().cloned().map(|t| (t.id, t)).collect();
    let mut winners: Vec<TeamId> = Vec::with_capacity(field.len() / 2);
    let mut nonce = 0u64;
    for pair in field.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let home_t = teams_by_id
            .get(&pair[0])
            .cloned()
            .ok_or_else(|| anyhow!("missing cup team {}", pair[0]))?;
        let away_t = teams_by_id
            .get(&pair[1])
            .cloned()
            .ok_or_else(|| anyhow!("missing cup team {}", pair[1]))?;
        let seed = rng_seed.wrapping_add(nonce_salt).wrapping_add(nonce);
        nonce += 1;
        let (hs, as_) = simulate_cup_match(app, &home_t, &away_t, season, day, seed)?;
        app.store()?.record_cup_match(
            season,
            round_label,
            None,
            home_t.id,
            away_t.id,
            hs,
            as_,
            day,
        )?;
        winners.push(if hs >= as_ { home_t.id } else { away_t.id });
    }
    Ok(winners)
}

fn run_cup_qf(
    app: &mut AppState,
    teams: &[Team],
    season: SeasonId,
    day: u32,
    rng_seed: u64,
) -> Result<()> {
    let rows = app.store()?.read_cup_matches(season)?;
    let standings = cup_group_standings(&rows);
    let field = cup_ko_field(&standings);
    if field.len() < 8 {
        return Ok(());
    }
    run_cup_ko_round(app, teams, season, day, rng_seed, "qf", 0xCAFEB0BA, &field)?;
    Ok(())
}

fn run_cup_sf(
    app: &mut AppState,
    teams: &[Team],
    season: SeasonId,
    day: u32,
    rng_seed: u64,
) -> Result<()> {
    let rows = app.store()?.read_cup_matches(season)?;
    let qf_winners: Vec<TeamId> = rows
        .iter()
        .filter(|r| r.round == "qf")
        .map(|r| if r.home_score >= r.away_score { r.home_team } else { r.away_team })
        .collect();
    if qf_winners.len() < 4 {
        return Ok(());
    }
    run_cup_ko_round(app, teams, season, day, rng_seed, "sf", 0xBEEFFACE, &qf_winners)?;
    Ok(())
}

fn run_cup_final(
    app: &mut AppState,
    teams: &[Team],
    season: SeasonId,
    day: u32,
    rng_seed: u64,
) -> Result<()> {
    let rows = app.store()?.read_cup_matches(season)?;
    let sf_winners: Vec<TeamId> = rows
        .iter()
        .filter(|r| r.round == "sf")
        .map(|r| if r.home_score >= r.away_score { r.home_team } else { r.away_team })
        .collect();
    if sf_winners.len() < 2 {
        return Ok(());
    }
    let winners = run_cup_ko_round(
        app,
        teams,
        season,
        day,
        rng_seed,
        "final",
        0xF1_AA_1EE5,
        &sf_winners,
    )?;
    if let Some(champ) = winners.first().copied() {
        let store = app.store()?;
        let abbrev = store
            .team_abbrev(champ)?
            .unwrap_or_else(|| format!("#{}", champ));
        let headline = format!("NBA Cup {} — {} lifts the trophy", season.0, abbrev);
        store.record_news(season, day, "cup", &headline, None)?;
    }
    Ok(())
}

// ----------------------------------------------------------------------
// snapshot construction
// ----------------------------------------------------------------------

fn star_roster_index() -> &'static nba3k_models::star_protection::StarRoster {
    use std::path::Path;
    use std::sync::OnceLock;
    static STAR_ROSTER: OnceLock<nba3k_models::star_protection::StarRoster> = OnceLock::new();
    STAR_ROSTER.get_or_init(|| {
        nba3k_models::star_protection::load_star_roster(Path::new(
            nba3k_models::star_protection::STAR_ROSTER_PATH,
        ))
        .unwrap_or_default()
    })
}

pub(crate) fn populate_default_starters(
    store: &nba3k_store::Store,
    user_team: TeamId,
) -> Result<bool> {
    let starters = store.read_starters(user_team)?;
    if starters.iter_assigned().next().is_some() {
        return Ok(false);
    }

    let roster = store.roster_for_team(user_team)?;
    if roster.len() < 5 {
        return Ok(false);
    }

    let team_abbrev = store
        .team_abbrev(user_team)?
        .unwrap_or_else(|| format!("T{}", user_team.0));
    let rotation = build_position_aware_rotation(&roster, star_roster_index(), &team_abbrev);
    let mut written = 0usize;
    for slot in rotation.iter().take(5) {
        store.upsert_starter(user_team, &slot.position.to_string(), slot.player)?;
        written += 1;
    }
    Ok(written == 5)
}

fn build_snapshot(app: &mut AppState, team: &Team) -> Result<TeamSnapshot> {
    let roster_index = star_roster_index();

    let mut roster = app.store()?.roster_for_team(team.id)?;
    // Drop active-injury players so the rotation pulls from next-up bench.
    roster.retain(|p| {
        p.injury
            .as_ref()
            .map(|i| i.games_remaining == 0)
            .unwrap_or(true)
    });

    // M21 Rotation Level A: if the user has set a complete starting 5 for
    // this team AND all 5 are still on the active roster, honor it. Bench
    // and minutes stay auto. Any partial / stale override falls through to
    // the position-aware auto-builder below.
    let user_starters = app.store()?.read_starters(team.id)?;
    let rotation = user_starters_or_auto(
        &user_starters,
        &roster,
        roster_index,
        &team.abbrev,
    );

    let team_overall = if rotation.is_empty() {
        50
    } else {
        // Per-slot weights (sum 1.0): top-3 carry 60% (0.30/0.20/0.10),
        // 4-8 share 40% (0.10/0.08/0.07/0.08/0.07).
        const SLOT_WEIGHTS: [f32; 8] = [0.30, 0.20, 0.10, 0.10, 0.08, 0.07, 0.08, 0.07];
        let mut ranked: Vec<u8> = rotation.iter().map(|r| r.overall).collect();
        ranked.sort_by(|a, b| b.cmp(a));
        let mut sum = 0.0_f32;
        let mut total_w = 0.0_f32;
        for (i, ovr) in ranked.iter().enumerate() {
            let w = SLOT_WEIGHTS.get(i).copied().unwrap_or(0.0);
            sum += (*ovr as f32) * w;
            total_w += w;
        }
        (sum / total_w.max(0.01)).round().clamp(0.0, 99.0) as u8
    };

    Ok(TeamSnapshot {
        id: team.id,
        abbrev: team.abbrev.clone(),
        overall: team_overall,
        home_court_advantage: 2.0,
        rotation,
    })
}

/// M21 Rotation Level A entry point. If the user has saved a complete +
/// roster-valid starting 5 for this team, lock those 5 as the positional
/// starters and let the existing auto-builder fill the 3 bench slots
/// around them. Any partial override or trade-stale player_id falls
/// through to the pure auto rotation — no half-apply.
fn user_starters_or_auto(
    starters: &Starters,
    roster: &[Player],
    star_roster: &nba3k_models::star_protection::StarRoster,
    team_abbrev: &str,
) -> Vec<RotationSlot> {
    if let Some(rotation) = apply_user_starters(starters, roster, star_roster, team_abbrev) {
        return rotation;
    }
    build_position_aware_rotation(roster, star_roster, team_abbrev)
}

/// Returns `Some` only when every slot is filled AND every player_id is
/// present in `roster` (which is already filtered to "on this team, not
/// active-injured"). A traded / retired / FA'd starter collapses the
/// override and the caller falls back to auto.
fn apply_user_starters(
    starters: &Starters,
    roster: &[Player],
    star_roster: &nba3k_models::star_protection::StarRoster,
    team_abbrev: &str,
) -> Option<Vec<RotationSlot>> {
    if !starters.is_complete() {
        return None;
    }
    let starter_share = |pos: Position| -> f32 {
        match pos {
            Position::PG => 0.71,
            Position::SG => 0.69,
            Position::SF => 0.67,
            Position::PF => 0.65,
            Position::C => 0.58,
        }
    };
    let starter_usage = |pos: Position| -> f32 {
        match pos {
            Position::PG => 0.24,
            Position::SG => 0.22,
            Position::SF => 0.20,
            Position::PF => 0.20,
            Position::C => 0.16,
        }
    };

    use std::collections::HashSet;
    let mut used: HashSet<PlayerId> = HashSet::new();
    let mut rotation: Vec<RotationSlot> = Vec::with_capacity(8);
    for (pos, pid) in starters.iter_assigned() {
        let player = roster.iter().find(|p| p.id == pid)?;
        if !used.insert(player.id) {
            // Same player picked twice across slots — invalid, fall back.
            return None;
        }
        rotation.push(RotationSlot {
            player: player.id,
            name: player.name.clone(),
            position: pos,
            minutes_share: starter_share(pos),
            usage: starter_usage(pos),
            ratings: player.ratings,
            age: player.age,
            overall: player.overall,
            potential: player.potential,
        });
    }

    // Bench (3 slots) reuses the auto-builder's "weakest starter position
    // gets a backup" logic, but seeded with the user's chosen starters.
    let bench_share = |pos: Position| -> f32 {
        match pos {
            Position::C => 0.42,
            _ => 0.36,
        }
    };
    let mut bench_used_positions: HashSet<Position> = HashSet::new();
    for _ in 0..3 {
        let weakest_pos = rotation
            .iter()
            .take(5)
            .filter(|s| !bench_used_positions.contains(&s.position))
            .min_by_key(|s| s.ratings.overall_for(s.position) as u32)
            .map(|s| s.position)
            .or_else(|| {
                rotation
                    .iter()
                    .take(5)
                    .min_by_key(|s| s.ratings.overall_for(s.position) as u32)
                    .map(|s| s.position)
            })
            .unwrap_or(Position::SF);
        bench_used_positions.insert(weakest_pos);

        let adjacent = |a: Position, b: Position| -> bool {
            let idx = |p: Position| -> i32 {
                match p {
                    Position::PG => 0,
                    Position::SG => 1,
                    Position::SF => 2,
                    Position::PF => 3,
                    Position::C => 4,
                }
            };
            (idx(a) - idx(b)).abs() <= 1
        };
        let fits = |p: &Player, pos: Position| -> bool {
            p.primary_position == pos
                || p.secondary_position == Some(pos)
                || adjacent(p.primary_position, pos)
        };
        let score_for = |p: &Player, pos: Position| -> u32 {
            let base = p.ratings.overall_for(pos) as u32;
            if star_roster.is_tagged(team_abbrev, &p.name) && p.primary_position == pos {
                base + 5
            } else {
                base
            }
        };

        let bench = roster
            .iter()
            .filter(|p| !used.contains(&p.id) && fits(p, weakest_pos))
            .max_by_key(|p| score_for(p, weakest_pos));
        let chosen = bench.or_else(|| {
            roster
                .iter()
                .filter(|p| !used.contains(&p.id))
                .max_by_key(|p| p.overall as u32)
        });
        if let Some(p) = chosen {
            used.insert(p.id);
            rotation.push(RotationSlot {
                player: p.id,
                name: p.name.clone(),
                position: weakest_pos,
                minutes_share: bench_share(weakest_pos),
                usage: starter_usage(weakest_pos) * 0.55,
                ratings: p.ratings,
                age: p.age,
                overall: p.overall,
                potential: p.potential,
            });
        }
    }

    Some(rotation)
}

/// Build an 8-man rotation respecting position constraints: 1 PG / 1 SG / 1 SF
/// / 1 PF / 1 C starters (best fit by `overall_for(pos)` allowing primary or
/// secondary position), then 3 bench by *position gap* — each iteration the
/// weakest starter slot gets backed up by the best available player at that
/// position. Falls back across positions when a true positional fit is missing.
///
/// Minutes shares are position-anchored (PG ~34 mpg, C ~28 mpg) instead of
/// purely rank-driven, so a team with a backup C doesn't waste minutes.
/// Franchise-tag players get a tiebreaker bump within the same position.
fn build_position_aware_rotation(
    roster: &[Player],
    star_roster: &nba3k_models::star_protection::StarRoster,
    team_abbrev: &str,
) -> Vec<RotationSlot> {
    use std::collections::HashSet;

    let positions = [
        Position::PG,
        Position::SG,
        Position::SF,
        Position::PF,
        Position::C,
    ];
    let starter_share = |pos: Position| -> f32 {
        match pos {
            Position::PG => 0.71, // 34 mpg
            Position::SG => 0.69, // 33 mpg
            Position::SF => 0.67, // 32 mpg
            Position::PF => 0.65, // 31 mpg
            Position::C => 0.58,  // 28 mpg — Cs share with backup big
        }
    };
    let bench_share = |pos: Position| -> f32 {
        match pos {
            Position::C => 0.42, // backup C absorbs ~20 mpg
            _ => 0.36,
        }
    };
    // Position-anchored usage: ball-handling positions create more shots.
    let starter_usage = |pos: Position| -> f32 {
        match pos {
            Position::PG => 0.24,
            Position::SG => 0.22,
            Position::SF => 0.20,
            Position::PF => 0.20,
            Position::C => 0.16,
        }
    };

    // Adjacent-position fallback: real NBA frequently slots a PF as SF
    // (McDaniels, KD), an SG as SF (Booker, Mitchell), a C as PF (small-ball).
    // The strict primary/secondary check missed these and surfaced raw bench
    // wings as SF starters when the actual SF was filed under PF. The adjacent
    // map mirrors NBA convention; cross-position picks keep position-distance
    // 1 (PG↔SG, SG↔SF, SF↔PF, PF↔C). Strict primary still preferred via the
    // `score_for` ranking — adjacency only kicks in for tiebreakers.
    let adjacent = |a: Position, b: Position| -> bool {
        let idx = |p: Position| -> i32 {
            match p {
                Position::PG => 0,
                Position::SG => 1,
                Position::SF => 2,
                Position::PF => 3,
                Position::C => 4,
            }
        };
        (idx(a) - idx(b)).abs() <= 1
    };
    let fits = |p: &Player, pos: Position| -> bool {
        p.primary_position == pos
            || p.secondary_position == Some(pos)
            || adjacent(p.primary_position, pos)
    };
    // Star tag bump: franchise stars priority for their primary position.
    let score_for = |p: &Player, pos: Position, star_bump: bool| -> u32 {
        let base = p.ratings.overall_for(pos) as u32;
        if star_bump && star_roster.is_tagged(team_abbrev, &p.name) && p.primary_position == pos {
            base + 5
        } else {
            base
        }
    };

    let mut used: HashSet<PlayerId> = HashSet::new();
    let mut rotation: Vec<RotationSlot> = Vec::with_capacity(8);

    // 5 starters, one per position. If positional pool empty (rare), fall back
    // to highest cross-position OVR with mismatch flag.
    for &pos in &positions {
        let starter = roster
            .iter()
            .filter(|p| !used.contains(&p.id) && fits(p, pos))
            .max_by_key(|p| score_for(p, pos, true));
        let chosen = starter.or_else(|| {
            roster
                .iter()
                .filter(|p| !used.contains(&p.id))
                .max_by_key(|p| p.overall as u32)
        });
        if let Some(p) = chosen {
            used.insert(p.id);
            rotation.push(RotationSlot {
                player: p.id,
                name: p.name.clone(),
                position: pos,
                minutes_share: starter_share(pos),
                usage: starter_usage(pos),
                ratings: p.ratings,
                age: p.age,
                overall: p.overall,
                potential: p.potential,
            });
        }
    }

    // 3 bench: each iteration pick the weakest STARTER position and back it
    // up with the best available player. Each position gets AT MOST one bench
    // slot (otherwise the loop would stack 3 Cs on a team where the C starter
    // is the weakest — CLE's Dean Wade case). After 3 bench picks the rotation
    // covers the 3 weakest positions with a depth player.
    let mut bench_used_positions: HashSet<Position> = HashSet::new();
    for _ in 0..3 {
        let weakest_pos = rotation
            .iter()
            .take(5)
            .filter(|s| !bench_used_positions.contains(&s.position))
            .min_by_key(|s| s.ratings.overall_for(s.position) as u32)
            .map(|s| s.position)
            // Fallback if all positions already backed up: pick lowest-rated position regardless.
            .or_else(|| {
                rotation
                    .iter()
                    .take(5)
                    .min_by_key(|s| s.ratings.overall_for(s.position) as u32)
                    .map(|s| s.position)
            })
            .unwrap_or(Position::SF);
        bench_used_positions.insert(weakest_pos);
        let bench = roster
            .iter()
            .filter(|p| !used.contains(&p.id) && fits(p, weakest_pos))
            .max_by_key(|p| p.ratings.overall_for(weakest_pos) as u32);
        let chosen = bench.or_else(|| {
            roster
                .iter()
                .filter(|p| !used.contains(&p.id))
                .max_by_key(|p| p.overall as u32)
        });
        if let Some(p) = chosen {
            used.insert(p.id);
            rotation.push(RotationSlot {
                player: p.id,
                name: p.name.clone(),
                position: weakest_pos,
                minutes_share: bench_share(weakest_pos),
                usage: starter_usage(weakest_pos) * 0.55,
                ratings: p.ratings,
                age: p.age,
                overall: p.overall,
                potential: p.potential,
            });
        }
    }

    rotation
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

fn cmd_standings(
    app: &mut AppState,
    season_arg: Option<u16>,
    JsonFlag { json: as_json }: JsonFlag,
) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;
    let target = season_arg.map(SeasonId).unwrap_or(state.season);
    let rows = store.read_standings(target)?;

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
    role: String,
    morale: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    injury: Option<InjuryStatus>,
}

fn cmd_roster(app: &mut AppState, team: Option<String>, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;
    let team_id = match team {
        Some(abbrev) => store
            .find_team_by_abbrev(&abbrev)?
            .ok_or_else(|| anyhow!("unknown team '{}'", abbrev))?,
        None => state.user_team,
    };
    let mut roster = store.roster_for_team(team_id)?;
    roster.sort_by(|a, b| b.overall.cmp(&a.overall));
    let abbrev = store.team_abbrev(team_id)?.unwrap_or_else(|| "???".into());

    let mapped: Vec<_> = roster
        .iter()
        .map(|p| RosterEntry {
            id: p.id.0,
            name: clean_name(&p.name),
            pos: p.primary_position.to_string(),
            age: p.age,
            overall: p.overall,
            potential: p.potential,
            role: short_role(p.role),
            morale: p.morale,
            injury: p.injury.clone(),
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string_pretty(&mapped)?);
    } else {
        println!("{} roster ({} players):", abbrev, mapped.len());
        // Pad ID column to fit the widest u32 we'll print, so 8/9/10-digit
        // ids all line up with the NAME column.
        let id_w = mapped.iter().map(|p| p.id.to_string().len()).max().unwrap_or(5).max(2);
        // Leading single-char column (`*` for injured) keeps healthy and
        // injured rows the same width.
        println!(
            " {:<id_w$}  {:<26}  {:<3}  {:>3}  {:>3}  {:>3}  {:<6}  {:>5}  {:>3}",
            "ID", "NAME", "POS", "AGE", "OVR", "POT", "ROLE", "MORAL", "INJ",
            id_w = id_w
        );
        for p in mapped {
            let (mark, inj_col) = match &p.injury {
                Some(i) if i.games_remaining > 0 => ("*", format!("{}", i.games_remaining)),
                _ => (" ", "-".to_string()),
            };
            println!(
                "{}{:<id_w$}  {:<26}  {:<3}  {:>3}  {:>3}  {:>3}  {:<6}  {:>5.2}  {:>3}",
                mark, p.id, p.name, p.pos, p.age, p.overall, p.potential, p.role, p.morale, inj_col,
                id_w = id_w
            );
        }
    }
    Ok(())
}

/// Strip the duplicate-whitespace quirk from scraped names like
/// "John Tonje  (TW)" so the roster table doesn't break alignment.
fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short_role(r: PlayerRole) -> String {
    match r {
        PlayerRole::Star => "Star",
        PlayerRole::Starter => "Start",
        PlayerRole::SixthMan => "6th",
        PlayerRole::RolePlayer => "Role",
        PlayerRole::BenchWarmer => "Bench",
        PlayerRole::Prospect => "Prosp",
    }
    .to_string()
}

pub fn cmd_player(app: &mut AppState, name: &str, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let p = store
        .find_player_by_name(name)?
        .ok_or_else(|| anyhow!("unknown player '{}'", name))?;
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
            "role": p.role.to_string(),
            "morale": p.morale,
            "no_trade_clause": p.no_trade_clause,
            "trade_kicker_pct": p.trade_kicker_pct,
            "injury": p.injury,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("{} ({}) — {}", clean_name(&p.name), p.primary_position, team);
        println!(
            "age {} | OVR {} | POT {} | role {} | morale {:.2}",
            p.age, p.overall, p.potential, p.role, p.morale
        );
        if p.no_trade_clause {
            println!("flags: NTC");
        }
        if let Some(pct) = p.trade_kicker_pct {
            println!("trade kicker: {}%", pct);
        }
        if let Some(i) = p.injury.as_ref() {
            if i.games_remaining > 0 {
                println!(
                    "INJURED: {}, {} game{} out",
                    i.description,
                    i.games_remaining,
                    if i.games_remaining == 1 { "" } else { "s" }
                );
            }
        }
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
        TradeAction::Propose3 { leg, json } => cmd_trade_propose3(app, &leg, json),
    }
}

fn resolve_team(store: &nba3k_store::Store, abbrev: &str) -> Result<TeamId> {
    store
        .find_team_by_abbrev(abbrev)?
        .ok_or_else(|| anyhow!("unknown team '{}'", abbrev))
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

    let send_clean: Vec<&String> = send.iter().filter(|s| !s.trim().is_empty()).collect();
    let receive_clean: Vec<&String> =
        receive.iter().filter(|s| !s.trim().is_empty()).collect();
    if send_clean.is_empty() {
        bail!("--send requires at least one player");
    }
    if receive_clean.is_empty() {
        bail!("--receive requires at least one player");
    }

    let send_ids: Vec<PlayerId> = send_clean
        .iter()
        .map(|t| resolve_player(store, from_id, t))
        .collect::<Result<_>>()?;
    let receive_ids: Vec<PlayerId> = receive_clean
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

    if let NegotiationState::Accepted(offer) = &final_state {
        apply_accepted_trade(app, offer)?;
        let headline = trade_headline(offer, app.store()?);
        app.store()?
            .record_news(state.season, state.day, "trade", &headline, None)?;
    }

    let store = app.store()?;
    let id = store.insert_trade_chain(state.season, state.day, &final_state)?;
    print_chain_outcome(id, &final_state, as_json, store)?;
    Ok(())
}

/// AI-initiated trade volume spike. Fires on the deadline day from the sim
/// loop. Each AI team rolls against its aggression trait; when a team
/// "shops", it picks a random AI counterparty and tries swapping a bench
/// piece. Trades that both sides accept (per the evaluator's verdict) get
/// executed. The user's team never auto-trades.
fn run_ai_trade_volume(
    app: &mut AppState,
    season: SeasonId,
    user_team: TeamId,
    seed: u64,
) -> Result<()> {
    use rand::seq::SliceRandom;

    let snap_owned = build_league_snapshot(app)?;
    let snap = snap_owned.view();

    let teams: Vec<TeamId> = snap
        .teams
        .iter()
        .filter(|t| t.id != user_team)
        .map(|t| t.id)
        .collect();
    if teams.len() < 2 {
        return Ok(());
    }

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut accepted: Vec<TradeOffer> = Vec::new();

    for shopper_id in &teams {
        let shopper = snap.team(*shopper_id);
        let aggression = shopper
            .map(|t| t.gm.traits.aggression)
            .unwrap_or(0.5);
        // Aggressive GMs (≥0.7) act ~50% of deadlines; conservative (~0.3)
        // act ~10%. Tunable.
        let p_act = (aggression * 0.5).clamp(0.0, 0.6);
        if rng.gen::<f32>() > p_act {
            continue;
        }
        let candidates: Vec<TeamId> =
            teams.iter().filter(|t| **t != *shopper_id).copied().collect();
        let Some(&partner_id) = candidates.choose(&mut rng) else {
            continue;
        };

        let mut shopper_roster: Vec<&Player> = snap.roster(*shopper_id);
        let mut partner_roster: Vec<&Player> = snap.roster(partner_id);
        if shopper_roster.is_empty() || partner_roster.is_empty() {
            continue;
        }
        // Pick mid-rotation pieces (rank 6-12) so we don't trade away cores.
        shopper_roster.sort_by(|a, b| b.overall.cmp(&a.overall));
        partner_roster.sort_by(|a, b| b.overall.cmp(&a.overall));
        let pick_idx_a = (6 + rng.gen_range(0..4)).min(shopper_roster.len() - 1);
        let pick_idx_b = (6 + rng.gen_range(0..4)).min(partner_roster.len() - 1);
        let pa = shopper_roster[pick_idx_a];
        let pb = partner_roster[pick_idx_b];

        let mut assets = IndexMap::new();
        assets.insert(
            *shopper_id,
            TradeAssets {
                players_out: vec![pa.id],
                picks_out: vec![],
                cash_out: Cents::ZERO,
            },
        );
        assets.insert(
            partner_id,
            TradeAssets {
                players_out: vec![pb.id],
                picks_out: vec![],
                cash_out: Cents::ZERO,
            },
        );
        let offer = TradeOffer {
            id: TradeId(0),
            initiator: *shopper_id,
            assets_by_team: assets,
            round: 1,
            parent: None,
        };

        // Both sides must accept for the trade to fire. Run evaluator from
        // each side independently.
        let ev_partner = evaluate_mod::evaluate(&offer, partner_id, &snap, &mut rng);
        let ev_shopper = evaluate_mod::evaluate(&offer, *shopper_id, &snap, &mut rng);
        if matches!(ev_partner.verdict, Verdict::Accept)
            && matches!(ev_shopper.verdict, Verdict::Accept)
        {
            accepted.push(offer);
        }
    }

    if accepted.is_empty() {
        return Ok(());
    }

    // Persist + execute (drop the snapshot first — it borrows the store).
    drop(snap);
    drop(snap_owned);
    for offer in &accepted {
        apply_accepted_trade(app, offer)?;
        let headline = trade_headline(offer, app.store()?);
        app.store()?
            .record_news(season, 0, "trade", &headline, None)?;
        let chain = NegotiationState::Accepted(offer.clone());
        app.store()?
            .insert_trade_chain(season, 0, &chain)?;
    }
    eprintln!("trade deadline: {} AI trade(s) executed", accepted.len());
    Ok(())
}

/// M17-A: per-day generation of inbound AI offers targeting the user team.
/// Each non-user team rolls against `aggression × 0.01` (so a max-aggressive
/// GM proposes ~1.5%/day, a default 0.5 GM ~0.5%/day). When a roll fires the
/// team picks the highest-"interest" user-team player (rumors heuristic) and
/// builds a comparable-OVR 2-team offer. Only persists when the forward
/// `evaluate` from the user perspective predicts ≥ `MIN_OFFER_PROBABILITY`.
fn run_ai_inbox_offers(
    app: &mut AppState,
    season: SeasonId,
    day: u32,
    user_team: TeamId,
    seed: u64,
) -> Result<()> {
    /// Forward acceptance probability from the user's POV below which we
    /// drop an offer on the floor instead of cluttering the inbox.
    const MIN_OFFER_PROBABILITY: f32 = 0.40;

    // Look up which AI teams already have a live open chain in the inbox
    // BEFORE we build the snapshot — keeps the borrow on the store short.
    let already_active: std::collections::HashSet<TeamId> = {
        let store = app.store()?;
        store
            .read_open_chains_targeting(season, user_team)?
            .into_iter()
            .filter_map(|(_, st)| match st {
                NegotiationState::Open { chain } => {
                    chain.last().map(|o| o.initiator)
                }
                _ => None,
            })
            .collect()
    };

    let snap_owned = build_league_snapshot(app)?;
    let snap = snap_owned.view();

    let user_roster: Vec<&Player> = snap.roster(user_team);
    if user_roster.is_empty() {
        return Ok(());
    }

    use nba3k_models::stat_projection::infer_archetype;
    use std::collections::HashSet;

    struct TeamFingerprint {
        archetypes: HashSet<String>,
        position_counts: HashMap<Position, u32>,
    }
    let mut fingerprints: HashMap<TeamId, TeamFingerprint> = HashMap::new();
    for t in snap.teams.iter() {
        let mut roster: Vec<&Player> = snap.roster(t.id);
        roster.sort_by(|a, b| b.overall.cmp(&a.overall));
        roster.truncate(8);
        let mut archetypes: HashSet<String> = HashSet::new();
        let mut position_counts: HashMap<Position, u32> = HashMap::new();
        for p in &roster {
            archetypes.insert(infer_archetype(p));
            *position_counts.entry(p.primary_position).or_insert(0) += 1;
        }
        fingerprints.insert(t.id, TeamFingerprint { archetypes, position_counts });
    }

    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let other_team_ids: Vec<TeamId> = snap
        .teams
        .iter()
        .filter(|t| t.id != user_team)
        .map(|t| t.id)
        .collect();

    let mut pending: Vec<TradeOffer> = Vec::new();

    for ai_team in &other_team_ids {
        // One open offer per AI team at a time — the user has to clear the
        // existing chain (accept/reject/counter) before we'll spam another.
        if already_active.contains(ai_team) {
            continue;
        }
        let aggression = snap
            .team(*ai_team)
            .map(|t| t.gm.traits.aggression)
            .unwrap_or(0.5);
        // Per-day fire rate scales with aggression: 0.5 → 0.5%/day,
        // 1.0 → 1.0%/day, 1.5 (max) → ~1.5%/day. Capped to keep the inbox
        // from overflowing if traits drift past expected ranges.
        let p_fire = (aggression as f64 * 0.01).clamp(0.0, 0.02);
        if rng.gen::<f64>() > p_fire {
            continue;
        }

        let Some(suitor_fp) = fingerprints.get(ai_team) else { continue };

        // Pick the user player who best fills a hole on the suitor.
        let mut targets: Vec<(&Player, f32)> = Vec::new();
        for p in &user_roster {
            let arch = infer_archetype(p);
            let pos_count = suitor_fp
                .position_counts
                .get(&p.primary_position)
                .copied()
                .unwrap_or(0);
            let score = if !suitor_fp.archetypes.contains(&arch) {
                1.0_f32
            } else if pos_count <= 1 {
                0.5
            } else {
                0.0
            };
            if score >= 0.5 {
                let weight = score + (p.overall as f32 / 100.0) * 0.1;
                targets.push((*p, weight));
            }
        }
        if targets.is_empty() {
            continue;
        }
        targets.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.0.overall.cmp(&a.0.overall))
        });
        let target = targets[0].0;

        let mut ai_roster: Vec<&Player> = snap.roster(*ai_team);
        if ai_roster.is_empty() {
            continue;
        }
        ai_roster.sort_by(|a, b| b.overall.cmp(&a.overall));
        // Pick a comparable-or-slightly-better outgoing piece so the user
        // POV evaluator sees an above-water offer.
        let pick_idx = ai_roster
            .iter()
            .position(|p| p.overall <= target.overall)
            .unwrap_or(ai_roster.len() - 1);
        let asset = ai_roster[pick_idx];

        let mut assets = IndexMap::new();
        assets.insert(
            *ai_team,
            TradeAssets {
                players_out: vec![asset.id],
                picks_out: vec![],
                cash_out: Cents::ZERO,
            },
        );
        assets.insert(
            user_team,
            TradeAssets {
                players_out: vec![target.id],
                picks_out: vec![],
                cash_out: Cents::ZERO,
            },
        );
        let starter = TradeOffer {
            id: TradeId(0),
            initiator: *ai_team,
            assets_by_team: assets,
            round: 1,
            parent: None,
        };

        // Skip CBA-illegal proposals — they'd just hard-reject in the inbox.
        if nba3k_trade::cba::validate(&starter, &snap).is_err() {
            continue;
        }

        let mut local_rng = ChaCha8Rng::seed_from_u64(
            seed.wrapping_add(ai_team.0 as u64).wrapping_add(target.id.0 as u64),
        );
        let evaluation = evaluate_mod::evaluate(&starter, user_team, &snap, &mut local_rng);
        // Don't queue offers the user is guaranteed to reject (untouchable
        // star, CBA violation, or insufficient value). On the composite path,
        // `confidence` IS the acceptance probability; on the short-circuit
        // paths it represents certainty of the *rejection*, so we have to
        // gate on the verdict before reading confidence as p(accept).
        match evaluation.verdict {
            Verdict::Reject(_) => continue,
            Verdict::Accept | Verdict::Counter(_) => {
                if evaluation.confidence < MIN_OFFER_PROBABILITY {
                    continue;
                }
            }
        }

        pending.push(starter);
    }

    if pending.is_empty() {
        return Ok(());
    }

    drop(snap);
    drop(snap_owned);
    let store = app.store()?;
    for offer in pending {
        let chain = NegotiationState::Open { chain: vec![offer] };
        store.insert_trade_chain(season, day, &chain)?;
    }
    Ok(())
}

/// One-line summary of an accepted trade, used as the news-feed headline.
/// "BOS sends Sam Hauser to LAL for LeBron James" for 2-team trades,
/// "3-team trade: BOS / LAL / DAL — 4 players" for 3+ team trades.
fn trade_headline(offer: &TradeOffer, store: &nba3k_store::Store) -> String {
    let teams: Vec<TeamId> = offer.assets_by_team.keys().copied().collect();
    let abbrev = |t: TeamId| {
        store
            .team_abbrev(t)
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("{}", t.0))
    };
    let names = |pids: &[PlayerId]| -> String {
        let parts: Vec<String> = pids
            .iter()
            .map(|p| {
                store
                    .player_name(*p)
                    .ok()
                    .flatten()
                    .map(|n| clean_name(&n))
                    .unwrap_or_else(|| format!("#{}", p.0))
            })
            .collect();
        if parts.is_empty() {
            "(nothing)".to_string()
        } else {
            parts.join(", ")
        }
    };
    if teams.len() == 2 {
        let a = teams[0];
        let b = teams[1];
        let a_out = &offer
            .assets_by_team
            .get(&a)
            .map(|x| x.players_out.clone())
            .unwrap_or_default();
        let b_out = &offer
            .assets_by_team
            .get(&b)
            .map(|x| x.players_out.clone())
            .unwrap_or_default();
        format!(
            "{} sends {} to {} for {}",
            abbrev(a),
            names(a_out),
            abbrev(b),
            names(b_out)
        )
    } else {
        let total_players: usize = offer
            .assets_by_team
            .values()
            .map(|a| a.players_out.len())
            .sum();
        let abbrs: Vec<String> = teams.iter().map(|t| abbrev(*t)).collect();
        format!(
            "{}-team trade: {} — {} players",
            teams.len(),
            abbrs.join(" / "),
            total_players
        )
    }
}

/// Walk `assets_by_team` and swap player team assignments. For a 2-team
/// trade A↔B, A's `players_out` move to B and B's `players_out` move to A.
/// For 3+ team trades each team sends to the next team in iteration order
/// (round-robin) — same convention the trade-engine uses internally.
fn apply_accepted_trade(app: &mut AppState, offer: &TradeOffer) -> Result<()> {
    let teams: Vec<TeamId> = offer.assets_by_team.keys().copied().collect();
    if teams.len() < 2 {
        return Ok(());
    }
    // Build outbound list: (player, current_team, dest_team).
    let mut moves: Vec<(PlayerId, TeamId)> = Vec::new();
    for (i, sender) in teams.iter().enumerate() {
        let dest = teams[(i + 1) % teams.len()];
        let assets = offer.assets_by_team.get(sender).expect("present");
        for &pid in &assets.players_out {
            moves.push((pid, dest));
        }
    }
    let store = app.store()?;
    for (pid, dest) in moves {
        store.assign_player_to_team(pid, dest)?;
    }
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

    if let NegotiationState::Accepted(offer) = &new_state {
        apply_accepted_trade(app, offer)?;
        let headline = trade_headline(offer, app.store()?);
        let state = current_state(app)?;
        app.store()?
            .record_news(state.season, state.day, "trade", &headline, None)?;
    }

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
        let status = match &state_chain {
            NegotiationState::Accepted(_) => "accepted",
            NegotiationState::Rejected { .. } => "rejected",
            NegotiationState::Open { .. } => "open",
            NegotiationState::Stalled => "stalled",
        };
        println!(
            "trade #{} ({}) — {} offer(s):",
            id, status, chain_offers.len()
        );
        for (i, offer) in chain_offers.iter().enumerate() {
            println!("  round {}", i + 1);
            for (team_id, assets) in &offer.assets_by_team {
                let abbrev = store
                    .team_abbrev(*team_id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| format!("{}", team_id.0));
                let names: Vec<String> = assets
                    .players_out
                    .iter()
                    .map(|pid| {
                        store
                            .player_name(*pid)
                            .ok()
                            .flatten()
                            .map(|n| clean_name(&n))
                            .unwrap_or_else(|| format!("#{}", pid.0))
                    })
                    .collect();
                let players_part = if names.is_empty() {
                    "(no players)".to_string()
                } else {
                    names.join(", ")
                };
                println!("    {} sends: {}", abbrev, players_part);
            }
        }
        if let NegotiationState::Rejected { reason, .. } = &state_chain {
            println!("  reason: {:?}", reason);
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
        NegotiationState::Open { chain } => (
            "open".to_string(),
            "counter".to_string(),
            chain.last().cloned(),
            teams_for(chain.last(), store),
        ),
        NegotiationState::Accepted(o) => (
            "accepted".to_string(),
            "accept".to_string(),
            Some(o.clone()),
            teams_for(Some(o), store),
        ),
        NegotiationState::Rejected { final_offer, reason } => (
            "rejected".to_string(),
            format!("reject — {}", reject_reason_to_string(reason)),
            Some(final_offer.clone()),
            teams_for(Some(final_offer), store),
        ),
        NegotiationState::Stalled => (
            "stalled".to_string(),
            "stalled".to_string(),
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

/// Human-readable rendering of `RejectReason`. Doesn't lowercase the
/// embedded message (so player names like "LeBron James" stay capitalized).
fn reject_reason_to_string(r: &RejectReason) -> String {
    match r {
        RejectReason::InsufficientValue => "insufficient value".to_string(),
        RejectReason::CbaViolation(s) => format!("CBA: {}", s),
        RejectReason::NoTradeClause(pid) => format!("no-trade clause (player #{})", pid.0),
        RejectReason::BadFaith => "bad-faith offer".to_string(),
        RejectReason::OutOfRoundCap => "negotiation rounds exhausted".to_string(),
        RejectReason::Other(s) => s.clone(),
    }
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
            summary.id, summary.verdict, summary.status, summary.round, summary.teams
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
        DevAction::TeamStrength { team } => cmd_dev_team_strength(app, &team),
    }
}

fn cmd_dev_team_strength(app: &mut AppState, abbrev: &str) -> Result<()> {
    use nba3k_sim::engine::team_quality::{
        ratings_from_vector, vector_from_rotation, QualityToRatingWeights,
    };
    let team_id = app
        .store()?
        .find_team_by_abbrev(abbrev)?
        .ok_or_else(|| anyhow!("unknown team '{}'", abbrev))?;
    let team = app
        .store()?
        .list_teams()?
        .into_iter()
        .find(|t| t.id == team_id)
        .ok_or_else(|| anyhow!("team not in list"))?;
    let snap = build_snapshot(app, &team)?;
    println!("=== {} rotation ({} slots) ===", team.abbrev, snap.rotation.len());
    for (i, s) in snap.rotation.iter().enumerate() {
        println!(
            "  [{}] {:<25} {:?} mins={:.2} usg={:.2} OVR={} 3PT={} BH={} PD={}",
            i, clean_name(&s.name), s.position, s.minutes_share, s.usage,
            s.overall, s.ratings.three_point, s.ratings.ball_handle, s.ratings.perimeter_defense
        );
    }
    let v = vector_from_rotation(&snap.rotation);
    let weights = QualityToRatingWeights::default();
    let (o, d) = ratings_from_vector(&v, &weights);
    println!();
    println!("=== {} team_quality vector ===", team.abbrev);
    println!("  team_efg              {:.2}", v.team_efg);
    println!("  top3_offense          {:.2}", v.top3_offense);
    println!("  playmaking            {:.2}", v.playmaking);
    println!("  spacing               {:.0}", v.spacing);
    println!("  ft_rate               {:.2}", v.ft_rate);
    println!("  rim_protection        {:.2}", v.rim_protection);
    println!("  perimeter_containment {:.0}", v.perimeter_containment);
    println!("  defensive_versatility {:.0}", v.defensive_versatility);
    println!("  defensive_disruption  {:.2}", v.defensive_disruption);
    println!();
    println!("ORtg={:.1}  DRtg={:.1}  NetRtg={:+.1}", o, d, o - d);
    Ok(())
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
        entries.sort_by_key(|(k, _)| (*k).clone());
        for (k, (a, r, c)) in entries {
            println!(
                "  {:<14} accept={:>3}  reject={:>3}  counter={:>3}",
                k, a, r, c
            );
        }
    }
    Ok(())
}

fn cmd_roster_set_role(app: &mut AppState, query: &str, role_str: &str) -> Result<()> {
    let role = match role_str.to_ascii_lowercase().as_str() {
        "star" => PlayerRole::Star,
        "starter" => PlayerRole::Starter,
        "sixth" | "sixthman" | "sixth-man" => PlayerRole::SixthMan,
        "role" | "roleplayer" | "role-player" => PlayerRole::RolePlayer,
        "bench" | "benchwarmer" | "bench-warmer" => PlayerRole::BenchWarmer,
        "prospect" => PlayerRole::Prospect,
        other => bail!(
            "unknown role '{}': use star|starter|sixth|role|bench|prospect",
            other
        ),
    };
    let players = app.store()?.all_active_players()?;
    let needle = query.to_ascii_lowercase();
    let mut hits: Vec<Player> = players
        .into_iter()
        .filter(|p| p.name.to_ascii_lowercase().contains(&needle))
        .collect();
    if hits.is_empty() {
        bail!("unknown player '{}'", query);
    }
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|p| p.name.as_str()).take(5).collect();
        bail!(
            "ambiguous match for '{}': {} candidates (e.g. {:?}). Refine the query.",
            query,
            hits.len(),
            names
        );
    }
    let mut player = hits.remove(0);
    player.set_role(role);
    let name = player.name.clone();
    let new_role = player.role;
    let morale = player.morale;
    app.store()?.upsert_player(&player)?;
    println!(
        "{}: role -> {} (morale {:.2})",
        name, new_role, morale
    );
    Ok(())
}

// ----------------------------------------------------------------------
// M5: chemistry / awards / playoffs / season-summary
// ----------------------------------------------------------------------

fn cmd_chemistry(app: &mut AppState, abbrev: &str, as_json: bool) -> Result<()> {
    let team_id = resolve_team(app.store()?, abbrev)?;
    let owned = build_league_snapshot(app)?;
    let snap = owned.view();
    let score = nba3k_models::team_chemistry::team_chemistry(&snap, team_id);
    let team_abbrev = abbrev.to_uppercase();
    if as_json {
        let reasons: Vec<_> = score
            .reasons()
            .iter()
            .map(|r| json!({ "label": r.label, "delta": r.delta }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "team": team_abbrev,
                "score": score.value,
                "reasons": reasons,
            }))?
        );
    } else {
        println!("chemistry {}: {:.3}", team_abbrev, score.value);
        for r in score.reasons() {
            println!("  {:<22} {:+.3}", r.label, r.delta);
        }
    }
    Ok(())
}

fn cmd_awards(app: &mut AppState, season_arg: Option<u16>, as_json: bool) -> Result<()> {
    let season = season_arg
        .map(SeasonId)
        .unwrap_or_else(|| current_state(app).map(|s| s.season).unwrap_or(SeasonId(2026)));
    let store = app.store()?;
    let games = store.read_games(season)?;
    let teams = store.list_teams()?;
    let players = store.all_active_players()?;
    let position_of: HashMap<PlayerId, Position> =
        players.iter().map(|p| (p.id, p.primary_position)).collect();
    let player_name: HashMap<PlayerId, String> =
        players.iter().map(|p| (p.id, p.name.clone())).collect();
    let team_abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();

    let mut standings = Standings::new(&teams);
    for g in &games {
        if !g.is_playoffs {
            standings.record_game_result(g);
        }
    }
    let regular: Vec<_> = games.iter().filter(|g| !g.is_playoffs).cloned().collect();
    let aggregate = nba3k_season::aggregate_season(&regular);

    let mut bundle = nba3k_season::compute_all_awards(
        &aggregate,
        &standings,
        season,
        None,
        None,
        |_pid| false,
        |pid| position_of.get(&pid).copied(),
    );

    // Filter (TW) two-way contracts out of Sixth Man — NBA rule.
    // Promote the next eligible ballot entry to winner.
    let is_two_way = |pid: PlayerId| {
        player_name
            .get(&pid)
            .map(|n| n.contains("(TW)"))
            .unwrap_or(false)
    };
    if bundle.sixth_man.winner.map(is_two_way).unwrap_or(false) {
        bundle.sixth_man.winner = bundle
            .sixth_man
            .ballot
            .iter()
            .find(|(p, _)| !is_two_way(*p))
            .map(|(p, _)| *p);
    }

    // COY fallback: if no prev-season standings, award to the best regular
    // record. Better than always-null when the user just simmed one season.
    if bundle.coy.winner.is_none() {
        if let Some(best) = standings
            .records
            .values()
            .max_by(|a, b| {
                a.win_pct()
                    .partial_cmp(&b.win_pct())
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.point_diff.cmp(&b.point_diff))
            })
        {
            bundle.coy.winner = Some(best.team);
        }
    }

    let render_player = |pid: Option<PlayerId>| -> serde_json::Value {
        match pid {
            Some(p) => json!({
                "player_id": p.0,
                "name": player_name.get(&p).cloned().unwrap_or_else(|| format!("#{}", p.0)),
            }),
            None => json!(null),
        }
    };
    let render_team = |tid: Option<TeamId>| -> serde_json::Value {
        match tid {
            Some(t) => json!({
                "team_id": t.0,
                "abbrev": team_abbrev.get(&t).cloned().unwrap_or_default(),
            }),
            None => json!(null),
        }
    };

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "season": season.0,
                "mvp": render_player(bundle.mvp.winner),
                "dpoy": render_player(bundle.dpoy.winner),
                "roy": render_player(bundle.roy.winner),
                "sixth_man": render_player(bundle.sixth_man.winner),
                "mip": render_player(bundle.mip.winner),
                "coy": render_team(bundle.coy.winner),
                "all_nba": bundle.all_nba.iter().map(|t| {
                    t.ballot.iter().map(|(p, s)| json!({"player_id": p.0, "share": s})).collect::<Vec<_>>()
                }).collect::<Vec<_>>(),
                "all_defensive": bundle.all_defensive.iter().map(|t| {
                    t.ballot.iter().map(|(p, s)| json!({"player_id": p.0, "share": s})).collect::<Vec<_>>()
                }).collect::<Vec<_>>(),
            }))?
        );
    } else {
        println!("Awards (season {}):", season.0);
        let line = |k: &str, pid: Option<PlayerId>| {
            let name = pid
                .and_then(|p| player_name.get(&p).cloned())
                .unwrap_or_else(|| "—".to_string());
            println!("  {:<10} {}", k, name);
        };
        line("MVP", bundle.mvp.winner);
        line("DPOY", bundle.dpoy.winner);
        line("ROY", bundle.roy.winner);
        line("Sixth Man", bundle.sixth_man.winner);
        line("MIP", bundle.mip.winner);
        let coy = bundle
            .coy
            .winner
            .and_then(|t| team_abbrev.get(&t).cloned())
            .unwrap_or_else(|| "—".to_string());
        println!("  {:<10} {}", "COY", coy);
    }

    // Persist (overwrite-safe). Each named award also drops a news row so the
    // feed reflects who won — `record_award` is `INSERT OR REPLACE` so re-runs
    // won't double up the awards table, but the news log is append-only and
    // will accumulate one row per re-run; that's intentional (each ceremony
    // is a fresh announcement).
    let news_day = store
        .load_season_state()
        .ok()
        .flatten()
        .map(|s| s.day)
        .unwrap_or(0);
    let award_news = |store: &nba3k_store::Store, label: &str, pid: PlayerId| -> Result<()> {
        let name = player_name
            .get(&pid)
            .cloned()
            .map(|n| clean_name(&n))
            .unwrap_or_else(|| format!("#{}", pid.0));
        let headline = format!("{} winner: {}", label, name);
        store.record_news(season, news_day, "award", &headline, None)?;
        Ok(())
    };
    if let Some(p) = bundle.mvp.winner {
        store.record_award(season, "MVP", p)?;
        award_news(store, "MVP", p)?;
    }
    if let Some(p) = bundle.dpoy.winner {
        store.record_award(season, "DPOY", p)?;
        award_news(store, "DPOY", p)?;
    }
    if let Some(p) = bundle.roy.winner {
        store.record_award(season, "ROY", p)?;
        award_news(store, "ROY", p)?;
    }
    if let Some(p) = bundle.sixth_man.winner {
        store.record_award(season, "SIXTH_MAN", p)?;
        award_news(store, "Sixth Man", p)?;
    }
    if let Some(p) = bundle.mip.winner {
        store.record_award(season, "MIP", p)?;
        award_news(store, "MIP", p)?;
    }
    Ok(())
}

fn cmd_playoffs_bracket(app: &mut AppState, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    let store = app.store()?;
    let teams = store.list_teams()?;
    let team_abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();

    // If playoff series have been recorded, render those — otherwise show
    // the seeding (R1 matchups) computed from regular-season standings.
    let recorded = store.read_series(state.season)?;

    if !recorded.is_empty() {
        if as_json {
            let series: Vec<_> = recorded
                .iter()
                .map(|s| {
                    json!({
                        "round": s.round,
                        "home": team_abbrev.get(&s.home_team).cloned().unwrap_or_default(),
                        "away": team_abbrev.get(&s.away_team).cloned().unwrap_or_default(),
                        "home_wins": s.home_wins,
                        "away_wins": s.away_wins,
                        "winner": if s.home_wins == 4 {
                            team_abbrev.get(&s.home_team).cloned()
                        } else if s.away_wins == 4 {
                            team_abbrev.get(&s.away_team).cloned()
                        } else { None },
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "season": state.season.0,
                    "series": series,
                }))?
            );
        } else {
            println!("Bracket (season {}):", state.season.0);
            for s in &recorded {
                let label = match s.round {
                    1 => "R1",
                    2 => "Semis",
                    3 => "ConfFinals",
                    4 => "Finals",
                    _ => "?",
                };
                println!(
                    "  {:<10} {} {} - {} {}",
                    label,
                    team_abbrev.get(&s.home_team).cloned().unwrap_or_default(),
                    s.home_wins,
                    s.away_wins,
                    team_abbrev.get(&s.away_team).cloned().unwrap_or_default(),
                );
            }
        }
        return Ok(());
    }

    let mut standings = Standings::new(&teams);
    for g in store.read_games(state.season)? {
        if !g.is_playoffs {
            standings.record_game_result(&g);
        }
    }
    standings.recompute_ranks();
    let bracket = nba3k_season::generate_bracket(&standings, state.season);

    if as_json {
        let series: Vec<_> = bracket
            .r1
            .iter()
            .map(|s| {
                json!({
                    "round": format!("{:?}", s.round),
                    "conference": s.conference.map(|c| format!("{:?}", c)),
                    "home": team_abbrev.get(&s.home).cloned().unwrap_or_default(),
                    "away": team_abbrev.get(&s.away).cloned().unwrap_or_default(),
                    "home_seed": s.home_seed,
                    "away_seed": s.away_seed,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "season": state.season.0,
                "r1": series,
            }))?
        );
    } else {
        let provisional = standings.records.values().all(|r| r.games_played() == 0);
        if provisional {
            println!(
                "R1 bracket (season {} — provisional, no regular-season games yet):",
                state.season.0
            );
        } else {
            println!("R1 bracket (season {}):", state.season.0);
        }
        for s in &bracket.r1 {
            println!(
                "  {:?} {} ({}) v {} ({})",
                s.conference.unwrap_or(Conference::East),
                team_abbrev.get(&s.home).cloned().unwrap_or_default(),
                s.home_seed,
                team_abbrev.get(&s.away).cloned().unwrap_or_default(),
                s.away_seed,
            );
        }
    }
    Ok(())
}

fn cmd_playoffs_sim(app: &mut AppState, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    if !matches!(state.phase, SeasonPhase::Playoffs) {
        bail!(
            "playoffs sim requires phase Playoffs; current phase is {:?}. Run `sim-to playoffs` first.",
            state.phase
        );
    }
    // Idempotency: refuse if a Finals series already exists for this season.
    let existing_series = app.store()?.read_series(state.season)?;
    let finals_done = existing_series
        .iter()
        .any(|s| s.round == nba3k_season::PlayoffRound::Finals as u8);
    if finals_done {
        bail!(
            "playoffs already simulated for season {} (champion already crowned). Advance the season with `season-advance`.",
            state.season.0
        );
    }

    let teams = app.store()?.list_teams()?;
    let teams_by_id: HashMap<TeamId, Team> = teams.iter().cloned().map(|t| (t.id, t)).collect();
    let team_abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();

    let mut standings = Standings::new(&teams);
    for g in app.store()?.read_games(state.season)? {
        if !g.is_playoffs {
            standings.record_game_result(&g);
        }
    }
    standings.recompute_ranks();
    let bracket = nba3k_season::generate_bracket(&standings, state.season);
    let engine = pick_engine("statistical");

    let mut current_round = bracket.r1.clone();
    let mut next_game_id = 100_000u64;
    let mut start_date = NaiveDate::from_ymd_opt(2026, 4, 18).expect("valid");
    let mut all_results: Vec<nba3k_season::SeriesResult> = Vec::new();
    let mut champion: Option<TeamId> = None;
    let mut finals_mvp: Option<PlayerId> = None;

    for round_idx in 0..4 {
        let mut next_round: Vec<nba3k_season::Series> = Vec::new();
        let mut round_results: Vec<nba3k_season::SeriesResult> = Vec::new();
        for series in &current_round {
            let home_snap = build_snapshot(app, teams_by_id.get(&series.home).expect("home"))?;
            let away_snap = build_snapshot(app, teams_by_id.get(&series.away).expect("away"))?;
            let mut rng = ChaCha8Rng::seed_from_u64(
                state.rng_seed
                    .wrapping_add(state.season.0 as u64)
                    .wrapping_add(round_idx as u64 * 1000)
                    .wrapping_add(next_game_id),
            );
            let result = nba3k_season::simulate_series(
                series.clone(),
                engine.as_ref(),
                &home_snap,
                &away_snap,
                state.season,
                start_date,
                &mut next_game_id,
                &mut rng,
            );
            for game in &result.games {
                app.store()?.record_game(game)?;
            }
            let row = nba3k_store::SeriesRow {
                season: state.season,
                round: result.series.round.ord(),
                home_team: result.series.home,
                away_team: result.series.away,
                home_wins: result.home_wins,
                away_wins: result.away_wins,
                games: result.games.clone(),
            };
            app.store()?.record_series(&row)?;
            round_results.push(result);
        }

        // Pair winners into next round (preserving canonical bracket order).
        let mut winners: Vec<(TeamId, u8)> = round_results
            .iter()
            .map(|r| {
                let winner = if r.home_wins == 4 { r.series.home } else { r.series.away };
                let seed = if r.home_wins == 4 { r.series.home_seed } else { r.series.away_seed };
                (winner, seed)
            })
            .collect();

        all_results.extend(round_results);

        if winners.len() == 1 {
            champion = Some(winners[0].0);
            if let Some(last) = all_results.last() {
                finals_mvp = nba3k_season::compute_finals_mvp(last);
            }
            break;
        }

        let next_round_kind = match round_idx {
            0 => nba3k_season::PlayoffRound::Semis,
            1 => nba3k_season::PlayoffRound::ConfFinals,
            2 => nba3k_season::PlayoffRound::Finals,
            _ => break,
        };

        // Pair adjacent winners — bracket halves stay matched (1/8 vs 4/5 in
        // upper half, 3/6 vs 2/7 in lower half preserved by canonical order).
        for pair in winners.chunks_mut(2) {
            if pair.len() != 2 {
                continue;
            }
            let (a, b) = (pair[0], pair[1]);
            let (home, away, home_seed, away_seed) =
                if a.1 <= b.1 { (a.0, b.0, a.1, b.1) } else { (b.0, a.0, b.1, a.1) };
            let conf = if matches!(next_round_kind, nba3k_season::PlayoffRound::Finals) {
                None
            } else {
                teams_by_id.get(&home).map(|t| t.conference)
            };
            next_round.push(nba3k_season::Series {
                round: next_round_kind,
                conference: conf,
                home,
                away,
                home_seed,
                away_seed,
            });
        }
        current_round = next_round;
        start_date = start_date + chrono::Duration::days(20);
    }

    if as_json {
        let series_json: Vec<_> = all_results
            .iter()
            .map(|r| {
                json!({
                    "round": format!("{:?}", r.series.round),
                    "home": team_abbrev.get(&r.series.home).cloned().unwrap_or_default(),
                    "away": team_abbrev.get(&r.series.away).cloned().unwrap_or_default(),
                    "home_wins": r.home_wins,
                    "away_wins": r.away_wins,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "season": state.season.0,
                "champion": champion.and_then(|t| team_abbrev.get(&t).cloned()),
                "finals_mvp": finals_mvp.map(|p| p.0),
                "series": series_json,
            }))?
        );
    } else {
        println!("Playoffs (season {}):", state.season.0);
        for r in &all_results {
            println!(
                "  {:?}: {} {} - {} {}",
                r.series.round,
                team_abbrev.get(&r.series.home).cloned().unwrap_or_default(),
                r.home_wins,
                r.away_wins,
                team_abbrev.get(&r.series.away).cloned().unwrap_or_default(),
            );
        }
        if let Some(c) = champion {
            println!(
                "Champion: {}",
                team_abbrev.get(&c).cloned().unwrap_or_default()
            );
        }
    }
    Ok(())
}

/// GM inbox — surfaces trade demands from unhappy stars, role mismatches,
/// and other roster alerts. Unhappy stars (`morale < 0.4` AND `role == Star`)
/// generate a trade demand. Senior players (age ≥ 36) on a rebuilder roster
/// get an "asked to be moved" line. Tunable later.
fn cmd_messages(app: &mut AppState, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    let store = app.store()?;
    let user_team = state.user_team;
    let roster = store.roster_for_team(user_team)?;

    let mut alerts: Vec<(String, String, String)> = Vec::new(); // (kind, player, msg)
    for p in &roster {
        let name = clean_name(&p.name);
        // Active injury — surface so the GM sees their depth chart status.
        if let Some(i) = p.injury.as_ref() {
            if i.games_remaining > 0 {
                alerts.push((
                    "injury".to_string(),
                    name.clone(),
                    format!(
                        "{} — {}, {} game{} out.",
                        name,
                        i.description,
                        i.games_remaining,
                        if i.games_remaining == 1 { "" } else { "s" }
                    ),
                ));
                continue;
            }
        }
        // Trade demand: any OVR-80+ player who's lost morale (< 0.5).
        if p.overall >= 80 && p.morale < 0.5 {
            alerts.push((
                "trade-demand".to_string(),
                name.clone(),
                format!(
                    "{} (OVR {}, role {}) is unhappy (morale {:.2}) — they're asking out.",
                    name, p.overall, p.role, p.morale
                ),
            ));
            continue;
        }
        // Role mismatch: high-OVR talent slotted into a low role even if
        // morale hasn't fully dropped yet.
        if p.overall >= 80
            && matches!(p.role, PlayerRole::BenchWarmer | PlayerRole::SixthMan)
        {
            alerts.push((
                "role-mismatch".to_string(),
                name.clone(),
                format!(
                    "{} is an OVR-{} talent slotted as {} — morale will drop fast.",
                    name, p.overall, p.role
                ),
            ));
            continue;
        }
        // Senior on a fading roster.
        if p.age >= 36 && p.morale < 0.5 {
            alerts.push((
                "veteran-restless".to_string(),
                name.clone(),
                format!(
                    "{} ({}yo) wants a contender — consider a buyout or deadline move.",
                    name, p.age
                ),
            ));
        }
    }

    // M17-C: surface tracked players from `notes` so the GM sees them in
    // the same view they already check daily. Resolve names via the active
    // pool first, then fall back to whatever the store still remembers
    // about retired/cut entries so the row never disappears.
    let note_rows = store.list_notes()?;
    let note_names: Vec<String> = if note_rows.is_empty() {
        Vec::new()
    } else {
        let active = store.all_active_players()?;
        let mut by_id: HashMap<PlayerId, String> = HashMap::with_capacity(active.len());
        for p in active {
            by_id.insert(p.id, clean_name(&p.name));
        }
        let mut names = Vec::with_capacity(note_rows.len());
        for n in &note_rows {
            let name = if let Some(s) = by_id.get(&n.player_id) {
                s.clone()
            } else if let Some(s) = store.player_name(n.player_id)? {
                clean_name(&s)
            } else {
                format!("#{}", n.player_id.0)
            };
            names.push(name);
        }
        names
    };

    if as_json {
        let arr: Vec<_> = alerts
            .iter()
            .map(|(k, p, m)| json!({"kind": k, "player": p, "message": m}))
            .collect();
        let payload = json!({
            "alerts": arr,
            "notes": note_names,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        if alerts.is_empty() {
            println!("Inbox: no alerts. Roster is happy.");
        } else {
            let injuries: Vec<_> = alerts.iter().filter(|(k, _, _)| k == "injury").collect();
            let other: Vec<_> = alerts.iter().filter(|(k, _, _)| k != "injury").collect();
            println!(
                "Inbox ({} alert{}):",
                alerts.len(),
                if alerts.len() == 1 { "" } else { "s" }
            );
            if !injuries.is_empty() {
                println!("  Injuries:");
                for (_, _name, msg) in &injuries {
                    println!("    {}", msg);
                }
            }
            for (kind, _name, msg) in &other {
                println!("  [{}] {}", kind, msg);
            }
        }
        if !note_names.is_empty() {
            println!(
                "Notes ({} tracked player{}):",
                note_names.len(),
                if note_names.len() == 1 { "" } else { "s" }
            );
            for n in &note_names {
                println!("  {}", n);
            }
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// M6: draft / season-advance
// ----------------------------------------------------------------------

fn draft_order(app: &mut AppState) -> Result<Vec<TeamId>> {
    let state = current_state(app)?;
    let store = app.store()?;
    let teams = store.list_teams()?;

    // Try the current season first; if its standings are empty (PreSeason
    // before any sim, or fresh-from-season-advance) fall back to the prior
    // season — that's the one the draft is tied to.
    let mut standings = Standings::new(&teams);
    let mut games = store.read_games(state.season)?;
    if games.iter().all(|g| g.is_playoffs)
        || games.is_empty()
    {
        if state.season.0 > 1 {
            games = store.read_games(SeasonId(state.season.0 - 1))?;
        }
    }
    for g in &games {
        if !g.is_playoffs {
            standings.record_game_result(g);
        }
    }

    // Reverse-record draft order. Worst record picks first; ties break on
    // point-diff (least-bad point diff picks first).
    let mut by_record: Vec<(TeamId, u16, i32, u8)> = teams
        .iter()
        .map(|t| {
            let r = standings.records.get(&t.id);
            (
                t.id,
                r.map(|r| r.wins).unwrap_or(0),
                r.map(|r| r.point_diff).unwrap_or(0),
                t.id.0,
            )
        })
        .collect();
    by_record.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });

    // NBA-style weighted lottery for the top 14 picks. The four worst teams
    // share 14% odds at #1 each, then odds taper. Picks 15-30 stay locked
    // to reverse-record order. RNG seeded from season so the lottery is
    // deterministic per save (God mode could expose this seed in M8).
    let lottery_count = by_record.len().min(14);
    let post_lottery: Vec<TeamId> = by_record
        .iter()
        .skip(lottery_count)
        .map(|(id, _, _, _)| *id)
        .collect();

    // Odds (basis points / 10000) at pick 1 for seeds 1..=14 of the
    // pre-lottery board (worst record first). Mirrors NBA 2019-present
    // smoothed lottery: 14/14/14/12.5/10.5/9/7.5/6/4.5/3/2/1.5/1/0.5%.
    const ODDS_BPS: [u32; 14] = [
        1400, 1400, 1400, 1250, 1050, 900, 750, 600, 450, 300, 200, 150, 100, 50,
    ];
    let mut pool: Vec<(TeamId, u32)> = by_record
        .iter()
        .take(lottery_count)
        .enumerate()
        .map(|(i, (id, _, _, _))| (*id, ODDS_BPS.get(i).copied().unwrap_or(0)))
        .collect();

    let mut rng = ChaCha8Rng::seed_from_u64(
        (state.season.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
    );
    let mut lottery_order: Vec<TeamId> = Vec::with_capacity(lottery_count);
    // Top 4 picks drawn weighted; rest fall back to reverse-record among
    // the un-drawn teams (NBA-style).
    let weighted_draws = lottery_count.min(4);
    for _ in 0..weighted_draws {
        let total: u32 = pool.iter().map(|(_, w)| *w).sum();
        if total == 0 {
            break;
        }
        let mut roll = rng.gen_range(0..total);
        let mut picked = 0usize;
        for (i, (_, w)) in pool.iter().enumerate() {
            if roll < *w {
                picked = i;
                break;
            }
            roll -= *w;
        }
        lottery_order.push(pool[picked].0);
        pool.remove(picked);
    }
    // Remaining lottery slots: by original record order among un-drawn.
    let drawn: std::collections::HashSet<TeamId> = lottery_order.iter().copied().collect();
    for (id, _, _, _) in by_record.iter().take(lottery_count) {
        if !drawn.contains(id) {
            lottery_order.push(*id);
        }
    }

    let mut final_order = lottery_order;
    final_order.extend(post_lottery);
    Ok(final_order)
}

fn cmd_draft_board(app: &mut AppState, as_json: bool) -> Result<()> {
    let prospects = app.store()?.list_prospects_visible()?;
    let total = prospects.len();
    let scouted_count = prospects.iter().filter(|(_, s)| *s).count();
    if as_json {
        let board: Vec<_> = prospects
            .iter()
            .take(60)
            .enumerate()
            .map(|(i, (p, scouted))| {
                if *scouted {
                    json!({
                        "mock_rank": i + 1,
                        "player_id": p.id.0,
                        "name": p.name,
                        "position": p.primary_position.to_string(),
                        "age": p.age,
                        "scouted": true,
                        "overall": p.overall,
                        "potential": p.potential,
                        "ratings": p.ratings,
                    })
                } else {
                    json!({
                        "mock_rank": i + 1,
                        "player_id": p.id.0,
                        "name": p.name,
                        "position": p.primary_position.to_string(),
                        "age": p.age,
                        "scouted": false,
                        "overall": "???",
                        "potential": "???",
                        "ratings": "???",
                    })
                }
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&board)?);
    } else {
        println!(
            "Draft board (top 60, {} of {} scouted):",
            scouted_count, total
        );
        for (i, (p, scouted)) in prospects.iter().take(60).enumerate() {
            if *scouted {
                println!(
                    "  {:>2}. {:<24} {} age={} ovr={} pot={}",
                    i + 1, p.name, p.primary_position, p.age, p.overall, p.potential
                );
            } else {
                println!(
                    "  {:>2}. {:<24} {} age={} ovr=??? pot=???",
                    i + 1, p.name, p.primary_position, p.age
                );
            }
        }
    }
    Ok(())
}

fn cmd_draft_order(app: &mut AppState, as_json: bool) -> Result<()> {
    let order = draft_order(app)?;
    let teams = app.store()?.list_teams()?;
    let abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();
    if as_json {
        let arr: Vec<_> = order
            .iter()
            .enumerate()
            .map(|(i, t)| {
                json!({
                    "pick": i + 1,
                    "team": abbrev.get(t).cloned().unwrap_or_default(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else {
        println!("Draft order:");
        for (i, t) in order.iter().enumerate() {
            println!(
                "  Pick {:>2}: {}",
                i + 1,
                abbrev.get(t).cloned().unwrap_or_default()
            );
        }
    }
    Ok(())
}

fn cmd_draft_sim(app: &mut AppState, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    if !matches!(state.phase, SeasonPhase::OffSeason | SeasonPhase::Playoffs) {
        bail!(
            "draft sim requires phase OffSeason (or Playoffs after finals); current phase is {:?}",
            state.phase
        );
    }
    let order = draft_order(app)?;
    let mut prospects = app.store()?.list_prospects()?;
    if prospects.is_empty() {
        bail!("no prospects available — draft already run for this class");
    }
    let abbrev: HashMap<TeamId, String> = app
        .store()?
        .list_teams()?
        .iter()
        .map(|t| (t.id, t.abbrev.clone()))
        .collect();

    let mut picks: Vec<(usize, TeamId, Player)> = Vec::new();
    for (idx, team_id) in order.into_iter().enumerate() {
        if prospects.is_empty() {
            break;
        }
        // BPA: best-available is index 0 (Store sort: potential DESC, overall DESC).
        let player = prospects.remove(0);
        app.store()?
            .assign_player_to_team(player.id, team_id)?;
        picks.push((idx + 1, team_id, player));
    }

    if as_json {
        let arr: Vec<_> = picks
            .iter()
            .map(|(pick, team, p)| {
                json!({
                    "pick": pick,
                    "team": abbrev.get(team).cloned().unwrap_or_default(),
                    "player_id": p.id.0,
                    "name": p.name,
                    "position": p.primary_position.to_string(),
                    "age": p.age,
                    "overall": p.overall,
                    "potential": p.potential,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else {
        println!("Draft sim — {} picks made:", picks.len());
        for (pick, team, p) in &picks {
            println!(
                "  {:>2}. {} -> {} (ovr={}, pot={})",
                pick,
                abbrev.get(team).cloned().unwrap_or_default(),
                p.name,
                p.overall,
                p.potential
            );
        }
    }
    Ok(())
}

fn cmd_draft_pick(app: &mut AppState, query: &str) -> Result<()> {
    let state = current_state(app)?;
    if !matches!(state.phase, SeasonPhase::OffSeason | SeasonPhase::Playoffs) {
        bail!(
            "draft pick requires phase OffSeason (or Playoffs after finals); current phase is {:?}",
            state.phase
        );
    }
    let needle = query.to_ascii_lowercase();
    let prospects = app.store()?.list_prospects()?;
    let player = prospects
        .into_iter()
        .find(|p| p.name.to_ascii_lowercase().contains(&needle))
        .ok_or_else(|| anyhow!("no prospect matches '{}'", query))?;
    // Resolve user team via a saved hint. The save embeds the user team
    // abbrev in `meta.user_team` — fall back to the lowest team id otherwise.
    let user_team_abbrev = app.store()?.get_meta("user_team")?.unwrap_or_default();
    let team_id = if user_team_abbrev.is_empty() {
        app.store()?
            .list_teams()?
            .first()
            .map(|t| t.id)
            .ok_or_else(|| anyhow!("no teams in save"))?
    } else {
        resolve_team(app.store()?, &user_team_abbrev)?
    };
    let _ = state;
    app.store()?.assign_player_to_team(player.id, team_id)?;
    println!(
        "drafted {} (ovr={}, pot={}) to team {}",
        player.name, player.overall, player.potential, team_id.0
    );
    Ok(())
}

fn cmd_season_advance(app: &mut AppState, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    if !matches!(state.phase, SeasonPhase::Playoffs | SeasonPhase::OffSeason) {
        bail!(
            "season advance requires phase Playoffs or OffSeason; current phase is {:?}",
            state.phase
        );
    }

    // Freeze the just-finished season's standings before state.season rolls
    // forward so `standings --season N` still recalls them next year.
    rebuild_standings(app, state.season)?;

    // 1) Progression pass — apply growth/decline to every active player.
    let games = app.store()?.read_games(state.season)?;
    let minutes = nba3k_season::aggregate_season_minutes(&games);
    let mut players = app.store()?.all_active_players()?;
    let mut devs: Vec<nba3k_models::progression::PlayerDevelopment> =
        Vec::with_capacity(players.len());
    for p in &players {
        let dev = app
            .store()?
            .read_player_dev(p.id, state.season)?
            .unwrap_or(nba3k_models::progression::PlayerDevelopment {
                player_id: p.id,
                peak_start_age: 25,
                peak_end_age: 30,
                dynamic_potential: p.potential,
                work_ethic: 70,
                last_progressed_season: state.season,
            });
        devs.push(dev);
    }
    let next_season = SeasonId(state.season.0 + 1);
    let summary = nba3k_season::run_progression_pass(
        &mut players,
        &mut devs,
        &minutes,
        next_season,
    );
    // Persist mutated players + devs.
    for p in &players {
        app.store()?.upsert_player(p)?;
    }
    app.store()?.bulk_upsert_player_dev(&devs)?;

    // 1b) Retirement pass — aging + low-minutes players hang it up before
    // the draft fills their roster slots. Walks post-progression `players`
    // so age increments from this season's growth-step are visible.
    let mut retirees = 0u32;
    for p in &players {
        let mins = minutes.get(&p.id).copied().unwrap_or(0);
        if nba3k_models::retirement::should_retire(p, mins) {
            app.store()?.set_player_retired(p.id)?;
            retirees += 1;
        }
    }

    // 2) Draft auto-sim (no user prompt — `draft pick` is the user-facing slot).
    let order = draft_order(app)?;
    let mut prospects = app.store()?.list_prospects()?;
    let mut draftees = 0u32;
    for team_id in order {
        if prospects.is_empty() {
            break;
        }
        let player = prospects.remove(0);
        app.store()?
            .assign_player_to_team(player.id, team_id)?;
        draftees += 1;
    }
    if draftees > 0 {
        let headline = format!("{} draft: {} picks made", state.season.0, draftees);
        app.store()?
            .record_news(state.season, state.day, "draft", &headline, None)?;
    }

    // 3) AI free-agent market — non-user teams sign top FAs in cap-room
    //    order until each is full or affordable FAs run out. The user team
    //    is skipped on purpose: their FA decisions stay manual via `fa sign`.
    let fa_signed = run_ai_free_agency(app, state.season, state.user_team)?;

    // 4) Roll season state forward + regenerate schedule for the new year.
    let new_state = SeasonState {
        season: next_season,
        phase: SeasonPhase::PreSeason,
        day: 0,
        user_team: state.user_team,
        mode: state.mode,
        rng_seed: state.rng_seed.wrapping_add(1),
    };
    app.store()?.save_season_state(&new_state)?;
    // Drop the OLD season's schedule rows (history stays in `games`),
    // then generate fresh games for the new year.
    app.store()?.clear_schedule_for_season(state.season)?;
    generate_and_store_schedule(app, next_season, new_state.rng_seed)?;

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "next_season": next_season.0,
                "progression": {
                    "processed": summary.processed,
                    "total_signed_delta": summary.total_signed_delta,
                    "potential_revisions": summary.potential_revisions,
                },
                "draft": { "picks_made": draftees },
                "retirements": retirees,
                "free_agency": { "signed": fa_signed },
            }))?
        );
    } else {
        println!(
            "advanced to season {} — progressed {} players (Δsum={}), {} drafted, {} retired, {} FAs signed",
            next_season.0, summary.processed, summary.total_signed_delta, draftees, retirees, fa_signed
        );
    }
    Ok(())
}

/// AI free-agent pass run as part of `season-advance`. Iterates teams in
/// cap-room order; each team greedily signs the top remaining FA whose
/// first-year salary fits within `cap × 1.30` (apron-ish soft ceiling).
/// User team is skipped — their FA flow stays manual.
///
/// Returns the count of FAs signed.
fn run_ai_free_agency(
    app: &mut AppState,
    season: SeasonId,
    user_team: TeamId,
) -> Result<u32> {
    let league_year = LeagueYear::for_season(season);
    let cap = league_year.map(|ly| ly.cap).unwrap_or(Cents::ZERO);
    // Soft ceiling = cap × 1.30. Saturating math avoids any i64 overflow on
    // pathological inputs; for real cap values this is just `cap * 13 / 10`.
    let soft_ceiling = Cents(cap.0.saturating_mul(13) / 10);

    let teams = app.store()?.list_teams()?;
    // (team_id, current_salary, current_roster_size). Skip the user team —
    // their FA decisions stay manual.
    let mut team_state: Vec<(TeamId, Cents, usize)> = Vec::with_capacity(teams.len());
    for t in &teams {
        if t.id == user_team {
            continue;
        }
        let salary = app.store()?.team_salary(t.id, season)?;
        let roster = app.store()?.roster_for_team(t.id)?.len();
        team_state.push((t.id, salary, roster));
    }

    let mut pool = app.store()?.list_free_agents()?;
    let mut signed = 0u32;

    // Outer loop: keep iterating while at least one team signed someone last
    // sweep. A single sweep visits teams in cap-room order; after a sweep
    // we re-sort because each signing changes cap room.
    loop {
        if pool.is_empty() {
            break;
        }
        // Sort by cap room desc — most-room teams pick first.
        team_state.sort_by(|a, b| (cap.0 - b.1.0).cmp(&(cap.0 - a.1.0)));

        let mut signed_this_sweep = false;
        for (team_id, team_salary, roster_size) in team_state.iter_mut() {
            if *roster_size >= AI_FA_ROSTER_CAP || pool.is_empty() {
                continue;
            }
            // Find the highest-OVR FA whose first-year salary fits the
            // team's soft ceiling (cap × 1.30 — list_free_agents is already
            // ordered by overall desc, so we just walk it).
            let pick_idx = pool.iter().position(|fa| {
                let proposed = nba3k_models::contract_gen::generate_contract(fa, season);
                let first_year = proposed
                    .years
                    .first()
                    .map(|y| y.salary)
                    .unwrap_or(Cents::ZERO);
                team_salary.0.saturating_add(first_year.0) <= soft_ceiling.0
            });
            let Some(idx) = pick_idx else { continue };

            let mut player = pool.remove(idx);
            let contract = nba3k_models::contract_gen::generate_contract(&player, season);
            let first_year = contract
                .years
                .first()
                .map(|y| y.salary)
                .unwrap_or(Cents::ZERO);
            player.team = Some(*team_id);
            player.role = PlayerRole::RolePlayer;
            player.morale = 0.5;
            player.contract = Some(contract);

            let store = app.store()?;
            store.upsert_player(&player)?;
            store.assign_player_to_team(player.id, *team_id)?;

            *team_salary += first_year;
            *roster_size += 1;
            signed += 1;
            signed_this_sweep = true;
        }
        if !signed_this_sweep {
            break;
        }
    }

    Ok(signed)
}

/// Min roster size AI teams aim for during the FA pass. Below this, teams
/// keep signing affordable FAs. Charter target is < 16, so we stop at 16.
const AI_FA_ROSTER_CAP: usize = 16;

fn cmd_season_summary(app: &mut AppState, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    let store = app.store()?;
    let teams = store.list_teams()?;
    let team_abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();
    let players = store.all_active_players()?;
    let player_name: HashMap<PlayerId, String> =
        players.iter().map(|p| (p.id, p.name.clone())).collect();

    let games = store.read_games(state.season)?;
    let regular: Vec<_> = games.iter().filter(|g| !g.is_playoffs).cloned().collect();
    let aggregate = nba3k_season::aggregate_season(&regular);
    let mut standings = Standings::new(&teams);
    for g in &regular {
        standings.record_game_result(g);
    }
    let position_of: HashMap<PlayerId, Position> =
        players.iter().map(|p| (p.id, p.primary_position)).collect();
    let bundle = nba3k_season::compute_all_awards(
        &aggregate,
        &standings,
        state.season,
        None,
        None,
        |_| false,
        |pid| position_of.get(&pid).copied(),
    );

    let saved_awards = store.read_awards(state.season)?;
    let award_lookup: HashMap<String, PlayerId> = saved_awards.into_iter().collect();
    let series_rows = store.read_series(state.season)?;
    let finals = series_rows
        .iter()
        .find(|s| s.round == nba3k_season::PlayoffRound::Finals as u8);
    let champion = finals.map(|s| if s.home_wins == 4 { s.home_team } else { s.away_team });

    if as_json {
        let award = |key: &str, fallback: Option<PlayerId>| {
            let pid = award_lookup.get(key).copied().or(fallback);
            pid.map(|p| {
                json!({
                    "player_id": p.0,
                    "name": player_name.get(&p).cloned().unwrap_or_default(),
                })
            })
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "season": state.season.0,
                "champion": champion.and_then(|t| team_abbrev.get(&t).cloned()),
                "awards": {
                    "mvp": award("MVP", bundle.mvp.winner),
                    "dpoy": award("DPOY", bundle.dpoy.winner),
                    "roy": award("ROY", bundle.roy.winner),
                    "sixth_man": award("SIXTH_MAN", bundle.sixth_man.winner),
                    "mip": award("MIP", bundle.mip.winner),
                },
            }))?
        );
    } else {
        println!("Season {} summary:", state.season.0);
        if let Some(c) = champion {
            println!(
                "  champion : {}",
                team_abbrev.get(&c).cloned().unwrap_or_default()
            );
        }
        let line = |k: &str, fallback: Option<PlayerId>| {
            let pid = award_lookup.get(k).copied().or(fallback);
            let name = pid
                .and_then(|p| player_name.get(&p).cloned())
                .unwrap_or_else(|| "—".to_string());
            println!("  {:<10} {}", k, name);
        };
        line("MVP", bundle.mvp.winner);
        line("DPOY", bundle.dpoy.winner);
        line("ROY", bundle.roy.winner);
        line("Sixth Man", bundle.sixth_man.winner);
        line("MIP", bundle.mip.winner);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// M10 — pre-locked stubs. Workers fill the bodies in their owned crates.
// ----------------------------------------------------------------------

fn cmd_career(app: &mut AppState, name: &str, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let p = store
        .find_player_by_name(name)?
        .ok_or_else(|| anyhow!("unknown player '{}'", name))?;
    let seasons = store.read_career_stats(p.id)?;

    let mut team_abbrev: HashMap<TeamId, String> = HashMap::new();
    for r in &seasons {
        if let Some(t) = r.team {
            if !team_abbrev.contains_key(&t) {
                let ab = store.team_abbrev(t)?.unwrap_or_else(|| "???".into());
                team_abbrev.insert(t, ab);
            }
        }
    }

    let career = nba3k_season::career::career_totals(&seasons);
    let display_name = clean_name(&p.name);

    if as_json {
        let seasons_json: Vec<_> = seasons
            .iter()
            .map(|r| {
                json!({
                    "season": r.season.0,
                    "team": r.team.and_then(|t| team_abbrev.get(&t).cloned()),
                    "gp": r.gp,
                    "ppg": round1(r.ppg()),
                    "rpg": round1(r.rpg()),
                    "apg": round1(r.apg()),
                    "spg": round1(r.spg()),
                    "bpg": round1(r.bpg()),
                    "fg_pct": round3(r.fg_pct()),
                    "three_pct": round3(r.three_pct()),
                    "ft_pct": round3(r.ft_pct()),
                    "minutes": r.minutes,
                    "pts_total": r.pts,
                    "reb_total": r.reb,
                    "ast_total": r.ast,
                })
            })
            .collect();
        let out = json!({
            "player": display_name,
            "player_id": p.id.0,
            "seasons": seasons_json,
            "career": {
                "gp": career.gp,
                "ppg": round1(career.ppg()),
                "rpg": round1(career.rpg()),
                "apg": round1(career.apg()),
                "spg": round1(career.spg()),
                "bpg": round1(career.bpg()),
                "fg_pct": round3(career.fg_pct()),
                "three_pct": round3(career.three_pct()),
                "ft_pct": round3(career.ft_pct()),
            },
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("{} — career", display_name);
        println!(
            "{:<8} {:<5} {:>3}  {:>4} {:>4} {:>4} {:>4} {:>4}  {:<5} {:<5} {:<5}",
            "SEASON", "TM", "GP", "PPG", "RPG", "APG", "SPG", "BPG", "FG%", "3P%", "FT%"
        );
        for r in &seasons {
            let label = format_season_label(r.season);
            let tm = r
                .team
                .and_then(|t| team_abbrev.get(&t).cloned())
                .unwrap_or_else(|| "—".into());
            println!(
                "{:<8} {:<5} {:>3}  {:>4.1} {:>4.1} {:>4.1} {:>4.1} {:>4.1}  {} {} {}",
                label,
                tm,
                r.gp,
                r.ppg(),
                r.rpg(),
                r.apg(),
                r.spg(),
                r.bpg(),
                fmt_pct(r.fg_pct()),
                fmt_pct(r.three_pct()),
                fmt_pct(r.ft_pct()),
            );
        }
        println!(
            "{:<8} {:<5} {:>3}  {:>4.1} {:>4.1} {:>4.1} {:>4.1} {:>4.1}  {} {} {}",
            "career",
            "",
            career.gp,
            career.ppg(),
            career.rpg(),
            career.apg(),
            career.spg(),
            career.bpg(),
            fmt_pct(career.fg_pct()),
            fmt_pct(career.three_pct()),
            fmt_pct(career.ft_pct()),
        );
    }
    Ok(())
}

/// `SeasonId(2026)` represents the 2025-26 league year.
fn format_season_label(s: SeasonId) -> String {
    let end_full = s.0;
    if end_full == 0 {
        return "0000-00".into();
    }
    let end_short = end_full % 100;
    format!("{}-{:02}", end_full - 1, end_short)
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// Render a 0..1 percentage as `.XYZ` (NBA convention — no leading 0).
fn fmt_pct(v: f32) -> String {
    if v <= 0.0 {
        return ".000".into();
    }
    let scaled = (v * 1000.0).round() as i32;
    if scaled >= 1000 {
        return "1.000".into();
    }
    format!(".{:03}", scaled)
}

/// Roster cap. Mirrors the NBA's hard cap of 15 standard contracts + 3
/// two-way slots (= 18); refused at this boundary, no soft-warning mode.
const FA_ROSTER_CAP: usize = 18;
/// How many free agents to surface in the default `fa list` view.
const FA_LIST_TOP_N: usize = 30;

fn cmd_fa_list(app: &mut AppState, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let mut pool = store.list_free_agents()?;
    let total = pool.len();
    pool.truncate(FA_LIST_TOP_N);

    if as_json {
        let arr: Vec<_> = pool
            .iter()
            .map(|p| {
                json!({
                    "id": p.id.0,
                    "name": clean_name(&p.name),
                    "position": p.primary_position.to_string(),
                    "age": p.age,
                    "overall": p.overall,
                    "potential": p.potential,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else if pool.is_empty() {
        println!("Free agents: none.");
    } else {
        println!("Free agents — top {} of {}", pool.len(), total);
        println!(
            "{:<28} {:>3} {:>3} {:>4} {:>4}",
            "NAME", "POS", "AGE", "OVR", "POT"
        );
        for p in &pool {
            println!(
                "{:<28} {:>3} {:>3} {:>4} {:>4}",
                clean_name(&p.name),
                p.primary_position.to_string(),
                p.age,
                p.overall,
                p.potential,
            );
        }
    }
    Ok(())
}

fn cmd_fa_sign(app: &mut AppState, query: &str) -> Result<()> {
    let state = current_state(app)?;
    let user_team = state.user_team;
    let store = app.store()?;

    let roster_size = store.roster_for_team(user_team)?.len();
    if roster_size >= FA_ROSTER_CAP {
        bail!(
            "roster full: {} players already on user team (cap {}). Cut a player first.",
            roster_size,
            FA_ROSTER_CAP
        );
    }

    let pool = store.list_free_agents()?;
    let needle = query.trim().to_ascii_lowercase();
    let mut hits: Vec<Player> = pool
        .into_iter()
        .filter(|p| p.name.to_ascii_lowercase().contains(&needle))
        .collect();
    if hits.is_empty() {
        bail!("'{}' is not a free agent (or no such player)", query);
    }
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|p| p.name.as_str()).take(5).collect();
        bail!(
            "ambiguous match for '{}': {} candidates (e.g. {:?}). Refine the query.",
            query,
            hits.len(),
            names
        );
    }
    let mut player = hits.remove(0);
    let abbrev = store
        .team_abbrev(user_team)?
        .unwrap_or_else(|| format!("team {}", user_team.0));
    let display_name = clean_name(&player.name);
    let overall = player.overall;

    // Sign defaults: role-player slot, neutral morale. The team flip is
    // routed through `assign_player_to_team` so `is_free_agent` is cleared
    // alongside `team_id`. Generate a contract scaled to OVR — without this,
    // FA signings show $0 against the cap.
    player.team = Some(user_team);
    player.role = PlayerRole::RolePlayer;
    player.morale = 0.5;
    player.contract = Some(nba3k_models::contract_gen::generate_contract(
        &player,
        state.season,
    ));
    store.upsert_player(&player)?;
    store.assign_player_to_team(player.id, user_team)?;

    let headline = format!("{} signs {} (OVR {})", abbrev, display_name, overall);
    store.record_news(state.season, state.day, "signing", &headline, None)?;

    println!("signed {} (OVR {}) to {}", display_name, overall, abbrev);
    Ok(())
}

fn cmd_fa_cut(app: &mut AppState, query: &str) -> Result<()> {
    let state = current_state(app)?;
    let user_team = state.user_team;
    let store = app.store()?;

    let roster = store.roster_for_team(user_team)?;
    let needle = query.trim().to_ascii_lowercase();
    let mut hits: Vec<Player> = roster
        .into_iter()
        .filter(|p| p.name.to_ascii_lowercase().contains(&needle))
        .collect();
    if hits.is_empty() {
        bail!(
            "'{}' is not on your roster (cut only works on user-team players)",
            query
        );
    }
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|p| p.name.as_str()).take(5).collect();
        bail!(
            "ambiguous match for '{}': {} candidates (e.g. {:?}). Refine the query.",
            query,
            hits.len(),
            names
        );
    }
    let player = hits.remove(0);
    let display_name = clean_name(&player.name);
    let abbrev = store
        .team_abbrev(user_team)?
        .unwrap_or_else(|| format!("team {}", user_team.0));
    store.cut_player(player.id)?;

    let headline = format!("{} cuts {} — now a free agent", abbrev, display_name);
    store.record_news(state.season, state.day, "cut", &headline, None)?;

    println!("cut {} — now a free agent", display_name);
    Ok(())
}

fn cmd_training(app: &mut AppState, player_query: &str, focus_str: &str) -> Result<()> {
    let focus = nba3k_models::training::TrainingFocus::parse_str(focus_str).ok_or_else(|| {
        anyhow!(
            "unknown training focus '{}': use shoot|inside|def|reb|ath|handle",
            focus_str
        )
    })?;

    // Resolve the user's team. The save embeds the abbrev in `meta.user_team`.
    let user_team_abbrev = app
        .store()?
        .get_meta("user_team")?
        .ok_or_else(|| anyhow!("no user team set in save (run `new --team <ABBR>` first)"))?;
    let user_team_id = resolve_team(app.store()?, &user_team_abbrev)?;

    // One-shot per season per team: gate on `meta.training_used:<season>:<team_id>`.
    let season = current_state(app)?.season;
    let meta_key = format!("training_used:{}:{}", season.0, user_team_id.0);
    if app.store()?.get_meta(&meta_key)?.is_some() {
        bail!(
            "training already used this season for team {} (season {})",
            user_team_abbrev,
            season.0
        );
    }

    // Resolve player by name and require they're on the user's team.
    let mut player = app
        .store()?
        .find_player_by_name(player_query)?
        .ok_or_else(|| anyhow!("unknown player '{}'", player_query))?;
    if player.team != Some(user_team_id) {
        bail!(
            "{} is not on your team ({})",
            clean_name(&player.name),
            user_team_abbrev
        );
    }

    let delta = nba3k_models::training::apply_training_focus(&mut player, focus);
    let new_overall = delta.new_overall;
    let player_name = player.name.clone();
    app.store()?.upsert_player(&player)?;
    app.store()?.set_meta(&meta_key, "1")?;

    let bumps: Vec<String> = delta
        .attributes_changed
        .iter()
        .map(|(name, d)| format!("+{} {}", d, name))
        .collect();
    let display_name = clean_name(&player_name)
        .split_whitespace()
        .last()
        .unwrap_or(&player_name)
        .to_string();
    println!(
        "{}: {} training applied ({}). New OVR: {}.",
        display_name,
        focus.label(),
        bumps.join(", "),
        new_overall
    );
    Ok(())
}

fn cmd_trade_propose3(app: &mut AppState, legs: &[String], as_json: bool) -> Result<()> {
    if legs.len() != 3 {
        bail!(
            "3-team trade requires exactly 3 --leg args (got {}). \
             Usage: --leg \"BOS:Player A\" --leg \"LAL:Player B\" --leg \"DAL:Player C\"",
            legs.len()
        );
    }

    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;

    // Parse each leg into (team_id, [player_id]).
    let mut parsed: Vec<(TeamId, Vec<PlayerId>)> = Vec::with_capacity(3);
    for leg in legs {
        let (abbr, players_part) = leg
            .split_once(':')
            .ok_or_else(|| anyhow!("--leg '{}' missing ':' separator", leg))?;
        let team_id = resolve_team(store, abbr.trim())?;
        let player_ids: Vec<PlayerId> = players_part
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|name| resolve_player(store, team_id, name))
            .collect::<Result<_>>()?;
        if player_ids.is_empty() {
            bail!("--leg '{}' must list at least one player", leg);
        }
        parsed.push((team_id, player_ids));
    }
    if parsed[0].0 == parsed[1].0 || parsed[0].0 == parsed[2].0 || parsed[1].0 == parsed[2].0 {
        bail!("3-team trade requires three distinct teams");
    }

    let initiator = parsed[0].0;
    let mut assets_by_team: IndexMap<TeamId, TradeAssets> = IndexMap::new();
    for (team_id, players) in &parsed {
        assets_by_team.insert(
            *team_id,
            TradeAssets { players_out: players.clone(), picks_out: vec![], cash_out: Cents::ZERO },
        );
    }

    let offer = TradeOffer {
        id: TradeId(0),
        initiator,
        assets_by_team,
        round: 1,
        parent: None,
    };

    let god_active = state.mode == GameMode::God || app.force_god;
    let team_ids: Vec<TeamId> = parsed.iter().map(|(t, _)| *t).collect();

    // Build snapshot once and run unanimous-Accept evaluation.
    let snap_owned = build_league_snapshot(app)?;
    let snapshot = snap_owned.view();

    let final_state = if god_active {
        NegotiationState::Accepted(offer.clone())
    } else {
        let mut rng = ChaCha8Rng::seed_from_u64(state.rng_seed.wrapping_add(state.day as u64));
        let mut all_accept = true;
        for team_id in &team_ids {
            let evaluation = evaluate_mod::evaluate(&offer, *team_id, &snapshot, &mut rng);
            if !matches!(evaluation.verdict, Verdict::Accept) {
                all_accept = false;
                break;
            }
        }
        if all_accept {
            NegotiationState::Accepted(offer.clone())
        } else {
            NegotiationState::Rejected {
                final_offer: offer.clone(),
                reason: RejectReason::Other("3-team unanimous-accept failed".into()),
            }
        }
    };

    if let NegotiationState::Accepted(accepted) = &final_state {
        apply_accepted_trade(app, accepted)?;
        let headline = trade_headline(accepted, app.store()?);
        app.store()?
            .record_news(state.season, state.day, "trade", &headline, None)?;
    }

    let store = app.store()?;
    let id = store.insert_trade_chain(state.season, state.day, &final_state)?;
    print_chain_outcome(id, &final_state, as_json, store)?;
    Ok(())
}

// ----------------------------------------------------------------------
// M11 — pre-locked stubs.
// ----------------------------------------------------------------------

fn cmd_cap(app: &mut AppState, team_arg: Option<String>, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    let store = app.store()?;

    let team_id = match team_arg.as_deref() {
        Some(abbrev) => {
            let needle = abbrev.trim().to_ascii_uppercase();
            store
                .list_teams()?
                .into_iter()
                .find(|t| t.abbrev.eq_ignore_ascii_case(&needle))
                .ok_or_else(|| anyhow!("unknown team abbrev '{}'", abbrev))?
                .id
        }
        None => state.user_team,
    };
    let abbrev = store
        .team_abbrev(team_id)?
        .unwrap_or_else(|| format!("team {}", team_id.0));

    let season = state.season;
    let label = format_season_label(season);
    let payroll = store.team_salary(team_id, season)?;
    let roster_size = store.roster_for_team(team_id)?.len();
    let ly = LeagueYear::for_season(season)
        .ok_or_else(|| anyhow!("no LeagueYear constants for season {}", season.0))?;

    if as_json {
        let signed_diff = |line: Cents| -> i64 {
            // Positive when payroll exceeds the threshold.
            payroll.as_dollars() - line.as_dollars()
        };
        let out = json!({
            "team": abbrev,
            "season": label,
            "payroll_cents": payroll.0,
            "cap_cents": ly.cap.0,
            "luxury_tax_cents": ly.tax.0,
            "first_apron_cents": ly.apron_1.0,
            "second_apron_cents": ly.apron_2.0,
            "over_cap_dollars": signed_diff(ly.cap),
            "over_tax_dollars": signed_diff(ly.tax),
            "over_first_apron_dollars": signed_diff(ly.apron_1),
            "over_second_apron_dollars": signed_diff(ly.apron_2),
            "roster_size": roster_size,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("{} salary cap ({}):", abbrev, label);
    println!("  payroll:        {}", payroll);
    println!("  cap:            {}  {}", ly.cap, fmt_delta(payroll, ly.cap));
    println!(
        "  luxury tax:     {}  {}",
        ly.tax,
        fmt_delta(payroll, ly.tax)
    );
    println!(
        "  first apron:    {}  {}",
        ly.apron_1,
        fmt_delta(payroll, ly.apron_1)
    );
    println!(
        "  second apron:   {}  {}",
        ly.apron_2,
        fmt_delta(payroll, ly.apron_2)
    );
    println!("roster size: {}", roster_size);
    Ok(())
}

/// Format `(payroll - line)` as `($X.XM over)` or `($X.XM under)`. Equal
/// payroll renders as `(at line)` so the user can tell the diff is exactly zero.
fn fmt_delta(payroll: Cents, line: Cents) -> String {
    if payroll > line {
        format!("({} over)", payroll - line)
    } else if payroll < line {
        format!("({} under)", line - payroll)
    } else {
        "(at line)".to_string()
    }
}

fn cmd_retire(app: &mut AppState, player: &str) -> Result<()> {
    let store = app.store()?;
    let p = store
        .find_player_by_name(player)?
        .ok_or_else(|| anyhow!("player '{}' not found", player))?;
    store.set_player_retired(p.id)?;

    let headline = format!(
        "{} retires (age {}, OVR {})",
        clean_name(&p.name),
        p.age,
        p.overall
    );
    let state = current_state(app)?;
    app.store()?
        .record_news(state.season, state.day, "retire", &headline, None)?;

    println!(
        "retired {} (age={}, ovr={})",
        p.name, p.age, p.overall
    );
    Ok(())
}

// ----------------------------------------------------------------------
// M12 — Hall of Fame.
// ----------------------------------------------------------------------

struct HofEntry {
    name: String,
    pos: String,
    yrs: u32,
    gp: u32,
    pts: u32,
    reb: u32,
    ast: u32,
    age_at_retirement: u8,
}

impl HofEntry {
    fn rpg(&self) -> f32 {
        if self.gp == 0 { 0.0 } else { self.reb as f32 / self.gp as f32 }
    }
    fn apg(&self) -> f32 {
        if self.gp == 0 { 0.0 } else { self.ast as f32 / self.gp as f32 }
    }
    /// Tiebreak: PTS + 2.5*AST + 1.2*REB — rewards well-rounded careers
    /// when raw scoring totals tie.
    fn tiebreak(&self) -> f64 {
        self.pts as f64 + 2.5 * self.ast as f64 + 1.2 * self.reb as f64
    }
}

fn cmd_hof(app: &mut AppState, limit: u32, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let retired = store.list_retired_players()?;

    let mut entries: Vec<HofEntry> = Vec::with_capacity(retired.len());
    for p in &retired {
        let seasons = store.read_career_stats(p.id)?;
        let mut gp = 0u32;
        let mut pts = 0u32;
        let mut reb = 0u32;
        let mut ast = 0u32;
        for s in &seasons {
            gp += s.gp;
            pts += s.pts;
            reb += s.reb;
            ast += s.ast;
        }
        entries.push(HofEntry {
            name: clean_name(&p.name),
            pos: p.primary_position.to_string(),
            yrs: seasons.len() as u32,
            gp,
            pts,
            reb,
            ast,
            age_at_retirement: p.age,
        });
    }

    entries.sort_by(|a, b| {
        b.pts
            .cmp(&a.pts)
            .then_with(|| {
                b.tiebreak()
                    .partial_cmp(&a.tiebreak())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.name.cmp(&b.name))
    });
    if entries.len() > limit as usize {
        entries.truncate(limit as usize);
    }

    if as_json {
        let arr: Vec<_> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                json!({
                    "rank": i + 1,
                    "name": e.name,
                    "pos": e.pos,
                    "yrs": e.yrs,
                    "gp": e.gp,
                    "pts": e.pts,
                    "rpg": round1(e.rpg()),
                    "apg": round1(e.apg()),
                    "age_at_retirement": e.age_at_retirement,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(arr))?
        );
        return Ok(());
    }

    if entries.is_empty() {
        println!("Hall of Fame: empty (no retired players yet).");
        return Ok(());
    }

    println!("Hall of Fame (top {}):", entries.len());
    println!(
        "{:>4}  {:<18} {:<3}  {:>3}  {:>4}  {:>5}  {:>4} {:>4}",
        "RANK", "NAME", "POS", "YRS", "GP", "PTS", "RPG", "APG"
    );
    for (i, e) in entries.iter().enumerate() {
        println!(
            "{:>4}  {:<18} {:<3}  {:>3}  {:>4}  {:>5}  {:>4.1} {:>4.1}",
            i + 1,
            hof_truncate(&e.name, 18),
            e.pos,
            e.yrs,
            e.gp,
            e.pts,
            e.rpg(),
            e.apg(),
        );
    }
    Ok(())
}

fn hof_truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba3k_core::{
        Conference, Division, GMArchetype, GMPersonality, Player, PlayerId, Position, Ratings, Team,
    };
    use tempfile::tempdir;

    /// Constructs a minimal AppState backed by a fresh temp DB for unit-style
    /// runs of `run_ai_free_agency`. Returns the dir-guard so the temp file
    /// outlives the test.
    fn ai_fa_test_setup() -> (tempfile::TempDir, AppState) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("ai_fa.db");
        let mut app = AppState::new(Some(path.clone()), false);
        app.open_path(path).expect("open");

        // Two teams: one user team, one AI team.
        let user_team = Team {
            id: TeamId(1),
            abbrev: "BOS".into(),
            city: "Boston".into(),
            name: "Celtics".into(),
            conference: Conference::East,
            division: Division::Atlantic,
            gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
            coach: nba3k_core::Coach::default_for("BOS"),
            roster: Vec::new(),
            draft_picks: Vec::new(),
        };
        let ai_team = Team {
            id: TeamId(2),
            abbrev: "LAL".into(),
            city: "Los Angeles".into(),
            name: "Lakers".into(),
            conference: Conference::West,
            division: Division::Pacific,
            gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
            coach: nba3k_core::Coach::default_for("LAL"),
            roster: Vec::new(),
            draft_picks: Vec::new(),
        };
        app.store().unwrap().upsert_team(&user_team).unwrap();
        app.store().unwrap().upsert_team(&ai_team).unwrap();
        (dir, app)
    }

    fn make_test_player(id: u32, team: Option<TeamId>, overall: u8) -> Player {
        Player {
            id: PlayerId(id),
            name: format!("Player{id}"),
            primary_position: Position::SF,
            secondary_position: None,
            age: 27,
            overall,
            potential: overall,
            ratings: Ratings::default(),
            contract: None,
            team,
            injury: None,
            no_trade_clause: false,
            trade_kicker_pct: None,
            role: nba3k_core::PlayerRole::RolePlayer,
            morale: 0.5,
        }
    }

    #[test]
    fn populate_default_starters_writes_five_slots() {
        let (_dir, mut app) = ai_fa_test_setup();
        let user = TeamId(1);
        for (idx, pos) in Position::all().into_iter().enumerate() {
            let mut player = make_test_player(100 + idx as u32, Some(user), 80 + idx as u8);
            player.primary_position = pos;
            app.store().unwrap().upsert_player(&player).unwrap();
        }

        let wrote = populate_default_starters(app.store().unwrap(), user).unwrap();
        assert!(wrote);

        let starters = app.store().unwrap().read_starters(user).unwrap();
        assert!(starters.is_complete());
        for (_, pid) in starters.iter_assigned() {
            assert!(
                app.store()
                    .unwrap()
                    .roster_for_team(user)
                    .unwrap()
                    .iter()
                    .any(|p| p.id == pid),
                "starter {pid:?} must come from the user roster"
            );
        }
    }

    #[test]
    fn ai_fa_pass_signs_available_free_agent() {
        let (_dir, mut app) = ai_fa_test_setup();
        let user = TeamId(1);
        let ai = TeamId(2);

        // Seed the AI team with one cheap player so it has a non-empty
        // roster + nonzero salary, but tons of room (well under cap).
        let mut anchor = make_test_player(1, Some(ai), 60);
        anchor.contract = Some(nba3k_models::contract_gen::generate_contract(
            &anchor,
            SeasonId(2026),
        ));
        app.store().unwrap().upsert_player(&anchor).unwrap();

        // Create one free agent: rostered then cut, so the FA flag is set.
        let fa = make_test_player(2, Some(user), 78);
        app.store().unwrap().upsert_player(&fa).unwrap();
        app.store().unwrap().cut_player(fa.id).unwrap();
        assert_eq!(
            app.store().unwrap().list_free_agents().unwrap().len(),
            1,
            "FA pool should have the one cut player"
        );

        let signed = run_ai_free_agency(&mut app, SeasonId(2026), user).unwrap();
        assert_eq!(signed, 1, "AI team should sign the lone affordable FA");

        // FA pool drained, the AI roster grew, the player has a contract.
        assert!(app.store().unwrap().list_free_agents().unwrap().is_empty());
        let ai_roster = app.store().unwrap().roster_for_team(ai).unwrap();
        assert!(
            ai_roster.iter().any(|p| p.id == fa.id),
            "signed FA should be on AI team roster"
        );
        let signed_player = ai_roster.iter().find(|p| p.id == fa.id).unwrap();
        assert!(
            signed_player.contract.is_some(),
            "signed FA must have a contract for cap accounting"
        );
    }

    #[test]
    fn ai_fa_pass_skips_user_team() {
        let (_dir, mut app) = ai_fa_test_setup();
        let user = TeamId(1);
        let ai = TeamId(2);

        // Make the AI team a non-actor: fill it past the AI roster target
        // so its slot won't trigger a signing.
        for i in 100..(100 + AI_FA_ROSTER_CAP as u32) {
            let mut p = make_test_player(i, Some(ai), 60);
            p.contract = Some(nba3k_models::contract_gen::generate_contract(
                &p,
                SeasonId(2026),
            ));
            app.store().unwrap().upsert_player(&p).unwrap();
        }

        // One free agent in the pool.
        let fa = make_test_player(2, Some(user), 78);
        app.store().unwrap().upsert_player(&fa).unwrap();
        app.store().unwrap().cut_player(fa.id).unwrap();

        let signed = run_ai_free_agency(&mut app, SeasonId(2026), user).unwrap();
        assert_eq!(signed, 0, "user team must not sign FAs in the AI pass");
        assert_eq!(
            app.store().unwrap().list_free_agents().unwrap().len(),
            1,
            "FA pool unchanged when only AI team is full"
        );
    }
}

// ----------------------------------------------------------------------
// M13 — pre-locked stubs.
// ----------------------------------------------------------------------

fn cmd_awards_race(app: &mut AppState, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    let store = app.store()?;

    let games = store.read_games(state.season)?;
    let regular: Vec<_> = games.into_iter().filter(|g| !g.is_playoffs).collect();
    let played = regular.len();

    if played < 10 {
        if as_json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "season": state.season.0,
                    "day": state.day,
                    "games_played": played,
                    "insufficient_games": true,
                }))?
            );
        } else {
            println!(
                "Award race: insufficient games (need 10+); only {} played.",
                played
            );
        }
        return Ok(());
    }

    let teams = store.list_teams()?;
    let players = store.all_active_players()?;

    let team_abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();
    let player_name: HashMap<PlayerId, String> =
        players.iter().map(|p| (p.id, p.name.clone())).collect();
    let player_team: HashMap<PlayerId, TeamId> =
        players.iter().filter_map(|p| p.team.map(|t| (p.id, t))).collect();

    let mut standings = Standings::new(&teams);
    for g in &regular {
        standings.record_game_result(g);
    }
    let aggregate = nba3k_season::aggregate_season(&regular);

    // Mid-season floor: scale to ~25% of games-so-far (clamped 3..=20). Keeps
    // a 9-day start from over-weighting 2-game samples while still letting
    // mid-season races include genuine rotation players.
    let max_team_games = standings
        .records
        .values()
        .map(|r| r.games_played())
        .max()
        .unwrap_or(0);
    let min_games: u16 = ((max_team_games / 4).max(3)).min(20);

    let mvp = nba3k_season::compute_mvp_race(&aggregate, &standings, state.season, min_games);
    let dpoy = nba3k_season::compute_dpoy_race(&aggregate, state.season, min_games);
    let roy = nba3k_season::compute_roy_race(&aggregate, state.season, min_games, |_pid| false);
    let sixth_man = nba3k_season::compute_sixth_man_race(&aggregate, state.season, min_games);
    let mip = nba3k_season::compute_mip_race(&aggregate, None, state.season, min_games);

    let team_record_str = |tid: TeamId| -> String {
        match standings.records.get(&tid) {
            Some(r) => format!("{}-{}", r.wins, r.losses),
            None => "—".into(),
        }
    };

    let entry_json = |rank: usize, pid: PlayerId, share: f32| -> serde_json::Value {
        let team = player_team.get(&pid).copied();
        let abbrev = team
            .and_then(|t| team_abbrev.get(&t).cloned())
            .unwrap_or_default();
        let ppg = aggregate
            .by_player
            .get(&pid)
            .map(|p| p.ppg())
            .unwrap_or(0.0);
        json!({
            "rank": rank,
            "player_id": pid.0,
            "name": player_name.get(&pid).cloned().unwrap_or_default(),
            "team": abbrev,
            "ppg": (ppg * 10.0).round() / 10.0,
            "share": (share * 1000.0).round() / 1000.0,
        })
    };

    let top_n = |award: &nba3k_season::AwardResult, n: usize| -> Vec<serde_json::Value> {
        award
            .ballot
            .iter()
            .take(n)
            .enumerate()
            .map(|(i, (pid, share))| entry_json(i + 1, *pid, *share))
            .collect()
    };

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "season": state.season.0,
                "day": state.day,
                "games_played": played,
                "min_games": min_games,
                "mvp": top_n(&mvp, 5),
                "dpoy": top_n(&dpoy, 5),
                "roy": top_n(&roy, 5),
                "sixth_man": top_n(&sixth_man, 5),
                "mip": top_n(&mip, 5),
            }))?
        );
        return Ok(());
    }

    println!(
        "Award race — through day {} of {} (mid-season check):",
        state.day,
        format_season_label(state.season)
    );

    let render = |label: &str, award: &nba3k_season::AwardResult| {
        if award.ballot.is_empty() {
            println!("  {:<12}    — (insufficient data)", label);
            return;
        }
        for (i, (pid, share)) in award.ballot.iter().take(5).enumerate() {
            let name = player_name
                .get(pid)
                .cloned()
                .unwrap_or_else(|| format!("#{}", pid.0));
            let team_str = player_team
                .get(pid)
                .and_then(|t| team_abbrev.get(t).cloned())
                .unwrap_or_else(|| "—".into());
            let record = player_team
                .get(pid)
                .map(|t| team_record_str(*t))
                .unwrap_or_else(|| "—".into());
            let ppg = aggregate
                .by_player
                .get(pid)
                .map(|p| p.ppg())
                .unwrap_or(0.0);
            let detail = format!("{} ({}, {:.1} PPG, {})", name, team_str, ppg, record);
            let prefix = if i == 0 {
                format!("  {:<12}", label)
            } else {
                "              ".to_string()
            };
            println!(
                "{}    {}. {:<40} {:>3}%",
                prefix,
                i + 1,
                detail,
                (share * 100.0).round() as i32,
            );
        }
    };

    render("MVP", &mvp);
    render("DPOY", &dpoy);
    render("ROY", &roy);
    render("Sixth Man", &sixth_man);
    render("MIP", &mip);

    Ok(())
}

fn cmd_news(app: &mut AppState, limit: u32, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let rows = store.recent_news(limit)?;

    if as_json {
        let arr: Vec<_> = rows
            .iter()
            .map(|r| {
                json!({
                    "season": r.season.0,
                    "day": r.day,
                    "kind": r.kind,
                    "headline": r.headline,
                    "body": r.body,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else if rows.is_empty() {
        println!("No league news yet.");
    } else {
        println!("Recent league news (last {}):", rows.len());
        for r in &rows {
            println!(
                "  S{} D{:<3} [{:<8}] {}",
                r.season.0, r.day, r.kind, r.headline
            );
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// M14 — pre-locked stubs.
// ----------------------------------------------------------------------

fn cmd_coach_show(app: &mut AppState, team: Option<String>, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;
    let team_id = match team {
        Some(abbrev) => resolve_team(store, &abbrev)?,
        None => state.user_team,
    };
    let team = store
        .list_teams()?
        .into_iter()
        .find(|t| t.id == team_id)
        .ok_or_else(|| anyhow!("team {} not found", team_id.0))?;
    let coach = &team.coach;
    let overall = coach.overall();
    let hot = coach.on_hot_seat();

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "team": team.abbrev,
                "name": coach.name,
                "overall": overall,
                "hot_seat": hot,
                "scheme_offense": coach.scheme_offense.to_string(),
                "scheme_defense": coach.scheme_defense.to_string(),
                "axes": {
                    "strategy": coach.axes.strategy,
                    "leadership": coach.axes.leadership,
                    "mentorship": coach.axes.mentorship,
                    "knowledge": coach.axes.knowledge,
                    "team_management": coach.axes.team_management,
                },
            }))?
        );
    } else {
        let suffix = if hot { " (hot seat)" } else { "" };
        println!(
            "{} coach: {} (overall {}){}",
            team.abbrev, coach.name, overall, suffix
        );
        println!(
            "  schemes: {} / {}",
            coach.scheme_offense, coach.scheme_defense
        );
        println!(
            "  axes: strategy {} / leadership {} / mentorship {} / knowledge {} / team_management {}",
            coach.axes.strategy.round() as i32,
            coach.axes.leadership.round() as i32,
            coach.axes.mentorship.round() as i32,
            coach.axes.knowledge.round() as i32,
            coach.axes.team_management.round() as i32,
        );
    }
    Ok(())
}

fn cmd_coach_fire(app: &mut AppState, team: Option<String>) -> Result<()> {
    let (team_id, season, day) = {
        let store = app.store()?;
        let state = store
            .load_season_state()?
            .ok_or_else(|| anyhow!("no season_state"))?;
        let team_id = match team {
            Some(ref abbrev) => resolve_team(store, abbrev)?,
            None => state.user_team,
        };
        (team_id, state.season, state.day)
    };

    let store = app.store()?;
    let mut team = store
        .list_teams()?
        .into_iter()
        .find(|t| t.id == team_id)
        .ok_or_else(|| anyhow!("team {} not found", team_id.0))?;

    let fired_name = team.coach.name.clone();
    // Mix season + day into the key so successive fires within the same
    // save produce different replacements.
    let key = (season.0 as u64) << 32 | ((day as u64) << 8) | team.id.0 as u64;
    let new_coach = Coach::generated(&team.abbrev, key);
    let hired_overall = new_coach.overall();
    let hired_name = new_coach.name.clone();
    team.coach = new_coach;

    store.upsert_team(&team)?;

    let headline = format!(
        "{} fired {}; hired {} (overall {})",
        team.abbrev, fired_name, hired_name, hired_overall
    );
    store.record_news(season, day, "coach", &headline, None)?;

    println!("fired {}; hired {} (overall {})", fired_name, hired_name, hired_overall);
    Ok(())
}

fn cmd_coach_pool(app: &mut AppState, limit: u32, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;

    let n = if limit == 0 { 12 } else { limit.min(50) };
    let mut pool: Vec<Coach> = (0..n)
        .map(|i| {
            let key = (state.season.0 as u64) << 32 | (i as u64).wrapping_mul(0x9E37_79B9);
            // Use a generic abbrev "POOL" so candidates aren't tied to a
            // specific team — they're free-agent coaches.
            Coach::generated("POOL", key)
        })
        .collect();
    pool.sort_by_key(|c| std::cmp::Reverse(c.overall()));

    if as_json {
        let arr: Vec<_> = pool
            .iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "overall": c.overall(),
                    "scheme_offense": c.scheme_offense.to_string(),
                    "scheme_defense": c.scheme_defense.to_string(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else {
        println!("Coach pool ({} candidates):", pool.len());
        println!("  {:<3}  {:<24}  {:>3}  {:<18}  {:<18}", "#", "NAME", "OVR", "OFFENSE", "DEFENSE");
        for (i, c) in pool.iter().enumerate() {
            println!(
                "  {:<3}  {:<24}  {:>3}  {:<18}  {:<18}",
                i + 1,
                c.name,
                c.overall(),
                c.scheme_offense.to_string(),
                c.scheme_defense.to_string(),
            );
        }
    }
    Ok(())
}

/// Scout caps reveals at 5 prospects per season; tracked via `meta`
/// key `scouts_used:<season>`.
const SCOUTS_PER_SEASON: u32 = 5;

fn cmd_scout(app: &mut AppState, query: &str) -> Result<()> {
    let state = current_state(app)?;
    let season = state.season;
    let day = state.day;

    // Resolve prospect by name: must be unsigned (team_id IS NULL),
    // not retired, and not a free agent. We narrow the candidate set to
    // prospects so a name collision with an active player (e.g. a "Cooper"
    // already on a roster) doesn't get selected by mistake.
    let candidates = app.store()?.list_prospects_visible()?;
    let needle = query.trim().to_ascii_lowercase();
    let mut hits: Vec<(Player, bool)> = candidates
        .into_iter()
        .filter(|(p, _)| p.name.to_ascii_lowercase().contains(&needle))
        .collect();
    if hits.is_empty() {
        bail!("'{}' is not a draft prospect (or no such player)", query);
    }
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|(p, _)| p.name.as_str()).take(5).collect();
        bail!(
            "ambiguous match for '{}': {} candidates (e.g. {:?}). Refine the query.",
            query,
            hits.len(),
            names
        );
    }
    let (player, already_scouted) = hits.remove(0);

    if already_scouted {
        let display_name = clean_name(&player.name);
        println!(
            "{} already scouted — OVR {}, POT {}",
            display_name, player.overall, player.potential
        );
        return Ok(());
    }

    // Per-season scout cap: gate before flipping the flag so the count
    // only advances on real reveals.
    let meta_key = format!("scouts_used:{}", season.0);
    let used: u32 = app
        .store()?
        .get_meta(&meta_key)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if used >= SCOUTS_PER_SEASON {
        bail!(
            "scout budget exhausted for season {}: {}/{} used. Wait until next season.",
            season.0,
            used,
            SCOUTS_PER_SEASON
        );
    }

    app.store()?.set_player_scouted(player.id, true)?;
    app.store()?.set_meta(&meta_key, &(used + 1).to_string())?;

    let display_name = clean_name(&player.name);
    let headline = format!(
        "scouted {} — OVR {}, POT {}",
        display_name, player.overall, player.potential
    );
    app.store()?
        .record_news(season, day, "scouting", &headline, None)?;

    println!(
        "scouted {} — OVR {}, POT {}",
        display_name, player.overall, player.potential
    );
    Ok(())
}

fn cmd_records(
    app: &mut AppState,
    scope: &str,
    stat: &str,
    limit: u32,
    as_json: bool,
) -> Result<()> {
    let scope_norm = scope.to_ascii_lowercase();
    let stat_norm = stat.to_ascii_lowercase();

    let stat_kind = parse_records_stat(&stat_norm)?;
    let scope_kind = parse_records_scope(&scope_norm)?;
    let limit = limit.max(1) as usize;

    let state = current_state(app)?;
    let store = app.store()?;

    // Aggregate per-player season totals across the requested scope.
    let aggregate = match scope_kind {
        RecordScope::Season => {
            let games = store.read_games(state.season)?;
            nba3k_season::aggregate_season(&games)
        }
        RecordScope::Career => {
            let mut combined: HashMap<PlayerId, nba3k_season::PlayerSeason> = HashMap::new();
            for season in store.distinct_game_seasons()? {
                let games = store.read_games(season)?;
                let agg = nba3k_season::aggregate_season(&games);
                for (pid, ps) in agg.by_player {
                    let entry = combined
                        .entry(pid)
                        .or_insert_with(|| nba3k_season::PlayerSeason {
                            player: pid,
                            team: None,
                            games: 0,
                            minutes: 0,
                            pts: 0,
                            reb: 0,
                            ast: 0,
                            stl: 0,
                            blk: 0,
                            tov: 0,
                            fg_made: 0,
                            fg_att: 0,
                            three_made: 0,
                            three_att: 0,
                            ft_made: 0,
                            ft_att: 0,
                        });
                    // Career career-end team: keep the most recent non-None team.
                    if ps.team.is_some() {
                        entry.team = ps.team;
                    }
                    entry.games = entry.games.saturating_add(ps.games);
                    entry.minutes += ps.minutes;
                    entry.pts += ps.pts;
                    entry.reb += ps.reb;
                    entry.ast += ps.ast;
                    entry.stl += ps.stl;
                    entry.blk += ps.blk;
                    entry.tov += ps.tov;
                    entry.fg_made += ps.fg_made;
                    entry.fg_att += ps.fg_att;
                    entry.three_made += ps.three_made;
                    entry.three_att += ps.three_att;
                    entry.ft_made += ps.ft_made;
                    entry.ft_att += ps.ft_att;
                }
            }
            nba3k_season::SeasonAggregate {
                by_player: combined,
                team_drtg: HashMap::new(),
            }
        }
    };

    let min_gp: u16 = match scope_kind {
        RecordScope::Season => 20,
        RecordScope::Career => 100,
    };

    // Filter + score.
    let mut scored: Vec<(PlayerId, &nba3k_season::PlayerSeason, f32)> = aggregate
        .by_player
        .iter()
        .filter(|(_, ps)| ps.games >= min_gp)
        .filter(|(_, ps)| {
            // For fg_pct we additionally require non-zero attempts to avoid div-by-zero
            // and meaningless leaders. For three_made the count is on totals so no extra gate.
            match stat_kind {
                RecordStat::FgPct => ps.fg_att > 0,
                _ => true,
            }
        })
        .map(|(pid, ps)| {
            let v = match stat_kind {
                RecordStat::Ppg => ps.ppg(),
                RecordStat::Rpg => ps.rpg(),
                RecordStat::Apg => ps.apg(),
                RecordStat::Spg => ps.spg(),
                RecordStat::Bpg => ps.bpg(),
                RecordStat::ThreeMade => ps.three_made_per_game(),
                RecordStat::FgPct => ps.fg_pct(),
            };
            (*pid, ps, v)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0 .0.cmp(&b.0 .0))
    });
    scored.truncate(limit);

    // Resolve names + team abbrevs only for the top-N (cheap on a fresh save,
    // cheap even on a 30-year career — we never iterate the full player table).
    let mut team_abbrev_cache: HashMap<TeamId, String> = HashMap::new();

    let stat_label = stat_kind.label();
    let scope_label = match scope_kind {
        RecordScope::Season => format!("{} season", format_season_label(state.season)),
        RecordScope::Career => "career".to_string(),
    };

    if as_json {
        let mut rows = Vec::with_capacity(scored.len());
        for (rank, (pid, ps, val)) in scored.iter().enumerate() {
            let name = store.player_name(*pid)?.unwrap_or_else(|| format!("#{}", pid.0));
            let abbrev = match ps.team {
                Some(t) => {
                    if let Some(s) = team_abbrev_cache.get(&t) {
                        s.clone()
                    } else {
                        let s = store.team_abbrev(t)?.unwrap_or_default();
                        team_abbrev_cache.insert(t, s.clone());
                        s
                    }
                }
                None => String::new(),
            };
            rows.push(json!({
                "rank": rank + 1,
                "player_id": pid.0,
                "name": name,
                "team": abbrev,
                "games": ps.games,
                "stat": stat_kind.json_key(),
                "value": format_records_value(stat_kind, *val),
            }));
        }
        println!("{}", serde_json::to_string_pretty(&json!({
            "scope": match scope_kind {
                RecordScope::Season => "season",
                RecordScope::Career => "career",
            },
            "season": state.season.0,
            "stat": stat_kind.json_key(),
            "min_gp": min_gp,
            "limit": limit,
            "rows": rows,
        }))?);
        return Ok(());
    }

    if scored.is_empty() {
        match scope_kind {
            RecordScope::Season => println!(
                "No qualifying players (need {} GP for season scope).",
                min_gp
            ),
            RecordScope::Career => println!(
                "No qualifying players (need {} GP for career scope).",
                min_gp
            ),
        }
        return Ok(());
    }

    println!(
        "Records — {}, top {} {} (min {} GP):",
        scope_label,
        scored.len(),
        stat_label,
        min_gp,
    );
    println!("RANK  NAME                  TM    GP  {}", stat_label);
    for (rank, (pid, ps, val)) in scored.iter().enumerate() {
        let name = store.player_name(*pid)?.unwrap_or_else(|| format!("#{}", pid.0));
        let abbrev = match ps.team {
            Some(t) => {
                if let Some(s) = team_abbrev_cache.get(&t) {
                    s.clone()
                } else {
                    let s = store.team_abbrev(t)?.unwrap_or_default();
                    team_abbrev_cache.insert(t, s.clone());
                    s
                }
            }
            None => "—".into(),
        };
        let value_str = match stat_kind {
            RecordStat::FgPct => fmt_pct(*val),
            _ => format!("{:.1}", val),
        };
        println!(
            "{:>4}  {:<20}  {:<3}  {:>3}  {}",
            rank + 1,
            truncate_str(&name, 20),
            truncate_str(&abbrev, 3),
            ps.games,
            value_str,
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum RecordScope {
    Season,
    Career,
}

#[derive(Debug, Clone, Copy)]
enum RecordStat {
    Ppg,
    Rpg,
    Apg,
    Spg,
    Bpg,
    ThreeMade,
    FgPct,
}

impl RecordStat {
    fn label(self) -> &'static str {
        match self {
            Self::Ppg => "PPG",
            Self::Rpg => "RPG",
            Self::Apg => "APG",
            Self::Spg => "SPG",
            Self::Bpg => "BPG",
            Self::ThreeMade => "3PM",
            Self::FgPct => "FG%",
        }
    }
    fn json_key(self) -> &'static str {
        match self {
            Self::Ppg => "ppg",
            Self::Rpg => "rpg",
            Self::Apg => "apg",
            Self::Spg => "spg",
            Self::Bpg => "bpg",
            Self::ThreeMade => "three_made",
            Self::FgPct => "fg_pct",
        }
    }
}

fn parse_records_stat(s: &str) -> Result<RecordStat> {
    match s {
        "ppg" => Ok(RecordStat::Ppg),
        "rpg" => Ok(RecordStat::Rpg),
        "apg" => Ok(RecordStat::Apg),
        "spg" => Ok(RecordStat::Spg),
        "bpg" => Ok(RecordStat::Bpg),
        "three_made" | "3pm" | "threes" => Ok(RecordStat::ThreeMade),
        "fg_pct" | "fgpct" | "fg%" => Ok(RecordStat::FgPct),
        other => bail!(
            "unknown stat `{}` — supported: ppg, rpg, apg, spg, bpg, three_made, fg_pct",
            other
        ),
    }
}

fn parse_records_scope(s: &str) -> Result<RecordScope> {
    match s {
        "season" => Ok(RecordScope::Season),
        "career" => Ok(RecordScope::Career),
        other => bail!(
            "unknown scope `{}` — supported: season, career",
            other
        ),
    }
}

fn format_records_value(stat: RecordStat, v: f32) -> serde_json::Value {
    // Round in f64 to avoid f32→JSON float widening artifacts (40.4 → 40.400001…).
    let v = v as f64;
    match stat {
        RecordStat::FgPct => json!((v * 1000.0).round() / 1000.0),
        _ => json!((v * 10.0).round() / 10.0),
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

// ----------------------------------------------------------------------
// M15 — pre-locked stubs.
// ----------------------------------------------------------------------

fn cmd_all_star(app: &mut AppState, season_arg: Option<u16>, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;
    let season = season_arg.map(SeasonId).unwrap_or(state.season);

    let rows = store.read_all_star(season)?;
    if rows.is_empty() {
        if as_json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "season": season.0,
                    "east_starters": [],
                    "east_reserves": [],
                    "west_starters": [],
                    "west_reserves": [],
                }))?
            );
        } else {
            println!(
                "No All-Star roster recorded for season {} yet (sim past day {} to trigger).",
                season.0, ALL_STAR_DAY
            );
        }
        return Ok(());
    }

    let players = store.all_active_players()?;
    let teams = store.list_teams()?;
    let player_name: HashMap<PlayerId, String> =
        players.iter().map(|p| (p.id, p.name.clone())).collect();
    let player_pos: HashMap<PlayerId, Position> =
        players.iter().map(|p| (p.id, p.primary_position)).collect();
    let player_team: HashMap<PlayerId, TeamId> = players
        .iter()
        .filter_map(|p| p.team.map(|t| (p.id, t)))
        .collect();
    let team_abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();

    let mut east_starters: Vec<PlayerId> = Vec::new();
    let mut east_reserves: Vec<PlayerId> = Vec::new();
    let mut west_starters: Vec<PlayerId> = Vec::new();
    let mut west_reserves: Vec<PlayerId> = Vec::new();
    for (conf, role, pid) in rows {
        match (conf, role.as_str()) {
            (Conference::East, "starter") => east_starters.push(pid),
            (Conference::East, _) => east_reserves.push(pid),
            (Conference::West, "starter") => west_starters.push(pid),
            (Conference::West, _) => west_reserves.push(pid),
        }
    }

    let render_player = |pid: PlayerId| -> serde_json::Value {
        let name = player_name
            .get(&pid)
            .cloned()
            .map(|n| clean_name(&n))
            .unwrap_or_else(|| format!("#{}", pid.0));
        let pos = player_pos
            .get(&pid)
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".into());
        let team = player_team
            .get(&pid)
            .and_then(|t| team_abbrev.get(t).cloned())
            .unwrap_or_else(|| "FA".into());
        json!({
            "player_id": pid.0,
            "name": name,
            "position": pos,
            "team": team,
        })
    };
    let render_group = |ids: &[PlayerId]| -> Vec<serde_json::Value> {
        ids.iter().copied().map(render_player).collect()
    };

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "season": season.0,
                "east_starters": render_group(&east_starters),
                "east_reserves": render_group(&east_reserves),
                "west_starters": render_group(&west_starters),
                "west_reserves": render_group(&west_reserves),
            }))?
        );
    } else {
        let print_row = |pid: PlayerId| {
            let pos = player_pos
                .get(&pid)
                .map(|p| p.to_string())
                .unwrap_or_else(|| "?".into());
            let name = player_name
                .get(&pid)
                .cloned()
                .map(|n| clean_name(&n))
                .unwrap_or_else(|| format!("#{}", pid.0));
            let team = player_team
                .get(&pid)
                .and_then(|t| team_abbrev.get(t).cloned())
                .unwrap_or_else(|| "FA".into());
            println!("  {:<3} {} ({})", pos, name, team);
        };
        // Display year is `season - 1`-`season` (e.g. SeasonId(2026) → "2025-26").
        let yy_end = season.0 % 100;
        println!(
            "{}-{:02} All-Star — East starters",
            season.0.saturating_sub(1),
            yy_end
        );
        for pid in &east_starters { print_row(*pid); }
        println!("East reserves");
        for pid in &east_reserves { print_row(*pid); }
        println!("West starters");
        for pid in &west_starters { print_row(*pid); }
        println!("West reserves");
        for pid in &west_reserves { print_row(*pid); }
    }
    Ok(())
}

#[derive(Serialize)]
struct SaveListRow {
    path: String,
    team: Option<String>,
    season: Option<u16>,
    created: Option<String>,
    app_version: Option<String>,
    error: Option<String>,
}

fn scan_db_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("db") {
            out.push(p);
        }
    }
}

fn cmd_saves_list(_app: &mut AppState, dir: Option<PathBuf>, as_json: bool) -> Result<()> {
    let scan_dirs: Vec<PathBuf> = match dir {
        Some(d) => vec![d],
        None => {
            let mut v = Vec::new();
            if let Ok(cwd) = std::env::current_dir() {
                v.push(cwd);
            }
            let tmp = PathBuf::from("/tmp");
            if cfg!(unix) && tmp.is_dir() {
                v.push(tmp);
            }
            v
        }
    };

    let mut paths: Vec<PathBuf> = Vec::new();
    for d in &scan_dirs {
        scan_db_files(d, &mut paths);
    }
    paths.sort();
    paths.dedup();

    let mut rows: Vec<SaveListRow> = Vec::with_capacity(paths.len());
    for p in &paths {
        let row = match nba3k_store::Store::open(p) {
            Ok(store) => {
                let team = store.get_meta("user_team").ok().flatten();
                let season = store
                    .get_meta("season")
                    .ok()
                    .flatten()
                    .and_then(|s| s.parse::<u16>().ok());
                let created = store.get_meta("created_at").ok().flatten();
                let app_version = store.get_meta("app_version").ok().flatten();
                drop(store);
                SaveListRow {
                    path: p.display().to_string(),
                    team,
                    season,
                    created,
                    app_version,
                    error: None,
                }
            }
            Err(e) => SaveListRow {
                path: p.display().to_string(),
                team: None,
                season: None,
                created: None,
                app_version: None,
                error: Some(e.to_string()),
            },
        };
        rows.push(row);
    }

    if as_json {
        println!("{}", serde_json::to_string_pretty(&json!(rows))?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("Save files: (none found)");
        return Ok(());
    }
    println!("Save files:");
    for r in &rows {
        if let Some(err) = &r.error {
            println!("  {}    [unreadable: {}]", r.path, err);
        } else {
            let team = r.team.as_deref().unwrap_or("???");
            let season = r
                .season
                .map(|s| s.to_string())
                .unwrap_or_else(|| "???".into());
            let created = r.created.as_deref().unwrap_or("???");
            println!("  {}    team={} season={} created={}", r.path, team, season, created);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct SaveShowReport {
    path: String,
    team: Option<String>,
    season: Option<u16>,
    phase: Option<String>,
    day: Option<u32>,
    schedule_total: u32,
    schedule_unplayed: u32,
    teams_count: u32,
    players_count: u32,
    app_version: Option<String>,
    created_at: Option<String>,
}

fn cmd_saves_show(_app: &mut AppState, path: PathBuf, as_json: bool) -> Result<()> {
    if !path.exists() {
        bail!("no such save: {}", path.display());
    }
    let store = nba3k_store::Store::open(&path)
        .with_context(|| format!("opening save {}", path.display()))?;

    let state = store.load_season_state()?;
    let team_abbrev = match &state {
        Some(s) => store.team_abbrev(s.user_team)?,
        None => store.get_meta("user_team")?,
    };
    let app_version = store.get_meta("app_version")?;
    let created_at = store.get_meta("created_at")?;
    let teams_count = store.count_teams().unwrap_or(0);
    let players_count = store.count_players().unwrap_or(0);
    let schedule_total = store.count_schedule().unwrap_or(0);
    let schedule_unplayed = store.count_unplayed().unwrap_or(0);
    let season = state.as_ref().map(|s| s.season.0).or_else(|| {
        store
            .get_meta("season")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u16>().ok())
    });

    let report = SaveShowReport {
        path: path.display().to_string(),
        team: team_abbrev,
        season,
        phase: state.as_ref().map(|s| format!("{:?}", s.phase)),
        day: state.as_ref().map(|s| s.day),
        schedule_total,
        schedule_unplayed,
        teams_count,
        players_count,
        app_version,
        created_at,
    };

    if as_json {
        println!("{}", serde_json::to_string_pretty(&json!(report))?);
        return Ok(());
    }

    println!("save:     {}", report.path);
    println!(
        "season:   {} ({})",
        report
            .season
            .map(|s| s.to_string())
            .unwrap_or_else(|| "???".into()),
        report.phase.as_deref().unwrap_or("???")
    );
    println!(
        "day:      {}",
        report.day.map(|d| d.to_string()).unwrap_or_else(|| "???".into())
    );
    println!("team:     {}", report.team.as_deref().unwrap_or("???"));
    println!(
        "teams:    {} | players: {}",
        report.teams_count, report.players_count
    );
    println!(
        "schedule: {} games ({} unplayed)",
        report.schedule_total, report.schedule_unplayed
    );
    if let Some(v) = &report.app_version {
        println!("version:  {}", v);
    }
    if let Some(c) = &report.created_at {
        println!("created:  {}", c);
    }
    Ok(())
}

fn cmd_saves_delete(app: &mut AppState, path: PathBuf, yes: bool) -> Result<()> {
    if !yes {
        bail!("refusing to delete {} without --yes", path.display());
    }
    if !path.exists() {
        bail!("no such save: {}", path.display());
    }
    if let Some(open) = &app.save_path {
        let same = match (std::fs::canonicalize(open), std::fs::canonicalize(&path)) {
            (Ok(a), Ok(b)) => a == b,
            _ => open == &path,
        };
        if same {
            bail!(
                "refusing to delete currently-open save {}",
                path.display()
            );
        }
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("deleting {}", path.display()))?;
    println!("deleted {}", path.display());
    Ok(())
}

// ----------------------------------------------------------------------
// M16 — pre-locked stubs.
// ----------------------------------------------------------------------

fn cmd_cup(app: &mut AppState, season_arg: Option<u16>, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state"))?;
    let season = season_arg.map(SeasonId).unwrap_or(state.season);
    let rows = store.read_cup_matches(season)?;

    let teams = store.list_teams()?;
    let team_abbrev: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();

    let abbrev_of = |t: TeamId| -> String {
        team_abbrev
            .get(&t)
            .cloned()
            .unwrap_or_else(|| format!("#{}", t))
    };

    if rows.is_empty() {
        if as_json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "season": season.0,
                    "groups": [],
                    "knockout": {"qf": [], "sf": [], "final": []},
                    "champion": null,
                }))?
            );
        } else {
            println!(
                "No NBA Cup recorded for season {} yet (sim past day {} to start group stage).",
                season.0, CUP_GROUP_DAY
            );
        }
        return Ok(());
    }

    // Group standings (wins, point diff) for the bracket render.
    let standings = cup_group_standings(&rows);
    let mut by_group: HashMap<String, Vec<CupGroupStanding>> = HashMap::new();
    for s in &standings {
        by_group.entry(s.group_id.clone()).or_default().push(s.clone());
    }
    let mut group_ids: Vec<String> = by_group.keys().cloned().collect();
    group_ids.sort();
    let sort_group = |g: &mut Vec<CupGroupStanding>| {
        g.sort_by(|a, b| {
            b.wins
                .cmp(&a.wins)
                .then(b.point_diff.cmp(&a.point_diff))
                .then(a.team.0.cmp(&b.team.0))
        });
    };

    // KO ladder grouped by round.
    let ko_round = |label: &str| -> Vec<&nba3k_store::CupMatchRow> {
        rows.iter().filter(|r| r.round == label).collect()
    };
    let qf_rows = ko_round("qf");
    let sf_rows = ko_round("sf");
    let final_rows = ko_round("final");
    let champion: Option<TeamId> = final_rows.first().map(|r| {
        if r.home_score >= r.away_score {
            r.home_team
        } else {
            r.away_team
        }
    });

    if as_json {
        let mut groups_json = Vec::new();
        for gid in &group_ids {
            let mut g = by_group.get(gid).cloned().unwrap_or_default();
            sort_group(&mut g);
            let standings_json: Vec<_> = g
                .iter()
                .map(|s| {
                    json!({
                        "team": abbrev_of(s.team),
                        "team_id": s.team.0,
                        "wins": s.wins,
                        "point_diff": s.point_diff,
                    })
                })
                .collect();
            groups_json.push(json!({
                "group_id": gid,
                "standings": standings_json,
            }));
        }
        let render_match = |r: &nba3k_store::CupMatchRow| {
            json!({
                "home": abbrev_of(r.home_team),
                "away": abbrev_of(r.away_team),
                "home_score": r.home_score,
                "away_score": r.away_score,
                "day": r.day,
            })
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "season": season.0,
                "groups": groups_json,
                "knockout": {
                    "qf": qf_rows.iter().map(|r| render_match(r)).collect::<Vec<_>>(),
                    "sf": sf_rows.iter().map(|r| render_match(r)).collect::<Vec<_>>(),
                    "final": final_rows.iter().map(|r| render_match(r)).collect::<Vec<_>>(),
                },
                "champion": champion.map(abbrev_of),
            }))?
        );
        return Ok(());
    }

    // Text render.
    println!("NBA Cup {}", season.0);
    println!();
    println!("Group stage:");
    for gid in &group_ids {
        let mut g = by_group.get(gid).cloned().unwrap_or_default();
        sort_group(&mut g);
        println!("  {}", gid);
        println!("    {:<6}  {:>2}W  {:>+5}", "TEAM", "", "DIFF");
        for s in &g {
            println!(
                "    {:<6}  {:>2}W  {:>+5}",
                abbrev_of(s.team),
                s.wins,
                s.point_diff,
            );
        }
    }

    let print_round = |label: &str, rs: &[&nba3k_store::CupMatchRow]| {
        if rs.is_empty() {
            return;
        }
        println!();
        println!("{}:", label);
        for r in rs {
            println!(
                "  {:<4} {:>3}  -  {:>3} {:<4}",
                abbrev_of(r.home_team),
                r.home_score,
                r.away_score,
                abbrev_of(r.away_team),
            );
        }
    };
    print_round("Quarterfinals", &qf_rows);
    print_round("Semifinals", &sf_rows);
    print_round("Final", &final_rows);

    println!();
    match champion {
        Some(t) => println!("Champion: {}", abbrev_of(t)),
        None => println!("Champion: TBD (sim past day {})", CUP_FINAL_DAY),
    }
    Ok(())
}

fn cmd_rumors(app: &mut AppState, limit: u32, as_json: bool) -> Result<()> {
    use nba3k_models::stat_projection::infer_archetype;
    use std::collections::HashSet;

    let state = current_state(app)?;
    let season = state.season;
    let store = app.store()?;

    let teams = store.list_teams()?;
    let players = store.all_active_players()?;

    let ly = LeagueYear::for_season(season)
        .ok_or_else(|| anyhow!("no LeagueYear constants for season {}", season.0))?;

    // Per-team data: abbrev, top-8 archetype set, top-8 position counts, cap room.
    struct TeamCtx {
        id: TeamId,
        abbrev: String,
        archetypes: HashSet<String>,
        position_counts: HashMap<Position, u32>,
        cap_room_cents: i64,
    }

    let mut team_ctx: HashMap<TeamId, TeamCtx> = HashMap::new();
    for t in &teams {
        let mut roster = store.roster_for_team(t.id)?;
        // Top-8 by overall, descending. `roster_for_team` returns OVR-sorted.
        roster.truncate(8);
        let mut archetypes: HashSet<String> = HashSet::new();
        let mut position_counts: HashMap<Position, u32> = HashMap::new();
        for p in &roster {
            archetypes.insert(infer_archetype(p));
            *position_counts.entry(p.primary_position).or_insert(0) += 1;
        }
        let payroll = store.team_salary(t.id, season)?;
        // First-apron headroom: above the apron the hard-cap blocks salary
        // intake, so it's a tighter ceiling than the cap line for "can this
        // team realistically take a contract on".
        let cap_room_cents = ly.apron_1.0 - payroll.0;
        team_ctx.insert(
            t.id,
            TeamCtx {
                id: t.id,
                abbrev: t.abbrev.clone(),
                archetypes,
                position_counts,
                cap_room_cents,
            },
        );
    }

    #[derive(Clone)]
    struct RumorRow {
        player_name: String,
        team_abbrev: String,
        ovr: u8,
        role: PlayerRole,
        interest: u32,
        suitors: Vec<String>,
    }

    let mut rumors: Vec<RumorRow> = Vec::new();
    for p in &players {
        let Some(player_team) = p.team else { continue };
        let archetype = infer_archetype(p);
        let first_year_cents = p
            .contract
            .as_ref()
            .map(|c| c.salary_for(season).0)
            .unwrap_or(0);
        let needed_room = first_year_cents / 2; // 0.5x first-year salary.

        let mut suitors: Vec<(String, f32)> = Vec::new();
        for ctx in team_ctx.values() {
            if ctx.id == player_team {
                continue;
            }
            if ctx.cap_room_cents < needed_room {
                continue;
            }
            let score = if !ctx.archetypes.contains(&archetype) {
                1.0
            } else {
                let pos_count = ctx
                    .position_counts
                    .get(&p.primary_position)
                    .copied()
                    .unwrap_or(0);
                if pos_count <= 1 {
                    0.5
                } else {
                    0.0
                }
            };
            if score >= 0.5 {
                suitors.push((ctx.abbrev.clone(), score));
            }
        }

        if suitors.is_empty() {
            continue;
        }
        suitors.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let interest = suitors.len() as u32;
        let team_abbrev = store
            .team_abbrev(player_team)?
            .unwrap_or_else(|| format!("T{}", player_team.0));

        rumors.push(RumorRow {
            player_name: clean_name(&p.name),
            team_abbrev,
            ovr: p.overall,
            role: p.role,
            interest,
            suitors: suitors.into_iter().map(|(a, _)| a).collect(),
        });
    }

    rumors.sort_by(|a, b| {
        b.interest
            .cmp(&a.interest)
            .then(b.ovr.cmp(&a.ovr))
            .then(a.player_name.cmp(&b.player_name))
    });
    let take = (limit as usize).min(rumors.len());
    let top = &rumors[..take];

    if as_json {
        let arr: Vec<_> = top
            .iter()
            .enumerate()
            .map(|(i, r)| {
                json!({
                    "rank": i + 1,
                    "player": r.player_name,
                    "team": r.team_abbrev,
                    "ovr": r.ovr,
                    "role": r.role.to_string(),
                    "interest": r.interest,
                    "suitors": r.suitors,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if top.is_empty() {
        println!("No trade rumors right now.");
        return Ok(());
    }

    println!("Trade rumors (top {}):", top.len());
    println!(
        "RANK  {:<18} {:<4} {:>3}  {:<8} {:>8}  TOP-3 SUITORS",
        "PLAYER", "TM", "OVR", "ROLE", "INTEREST"
    );
    for (i, r) in top.iter().enumerate() {
        let top3 = r
            .suitors
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let mut name = r.player_name.clone();
        if name.chars().count() > 18 {
            name = name.chars().take(18).collect();
        }
        println!(
            "{:>4}  {:<18} {:<4} {:>3}  {:<8} {:>8}  {}",
            i + 1,
            name,
            r.team_abbrev,
            r.ovr,
            short_role(r.role),
            r.interest,
            top3,
        );
    }
    Ok(())
}

fn cmd_compare(app: &mut AppState, a: &str, b: &str, as_json: bool) -> Result<()> {
    // Resolve both teams up front; refuse self-compare.
    let (id_a, id_b, abbrev_a, abbrev_b) = {
        let store = app.store()?;
        let id_a = resolve_team(store, a)?;
        let id_b = resolve_team(store, b)?;
        if id_a == id_b {
            bail!("cannot compare a team to itself");
        }
        let abbrev_a = store.team_abbrev(id_a)?.unwrap_or_else(|| a.to_uppercase());
        let abbrev_b = store.team_abbrev(id_b)?.unwrap_or_else(|| b.to_uppercase());
        (id_a, id_b, abbrev_a, abbrev_b)
    };

    let state = current_state(app)?;
    let season = state.season;

    // Build the league snapshot once for chemistry on both teams.
    let owned = build_league_snapshot(app)?;
    let snap = owned.view();
    let chem_a = nba3k_models::team_chemistry::team_chemistry(&snap, id_a);
    let chem_b = nba3k_models::team_chemistry::team_chemistry(&snap, id_b);

    let store = app.store()?;
    let breakdown_a = compare_team_breakdown(store, id_a, season)?;
    let breakdown_b = compare_team_breakdown(store, id_b, season)?;
    let payroll_a = store.team_salary(id_a, season)?;
    let payroll_b = store.team_salary(id_b, season)?;

    if as_json {
        let render_top8 = |rows: &[(Position, PlayerId, String, u8)]| -> serde_json::Value {
            let arr: Vec<_> = rows
                .iter()
                .map(|(pos, pid, name, ovr)| {
                    json!({
                        "position": pos.to_string(),
                        "player_id": pid.0,
                        "name": clean_name(name),
                        "overall": ovr,
                    })
                })
                .collect();
            json!(arr)
        };
        let team_obj = |abbrev: &str,
                        bd: &CompareBreakdown,
                        chem: &nba3k_models::Score,
                        payroll: Cents|
         -> serde_json::Value {
            json!({
                "team": abbrev,
                "roster_size": bd.roster_size,
                "top8_avg_overall": bd.top8_avg,
                "payroll_cents": payroll.0,
                "payroll_dollars": payroll.as_dollars(),
                "chemistry": chem.value,
                "top8": render_top8(&bd.top8),
            })
        };
        let payroll_delta = payroll_a.as_dollars() - payroll_b.as_dollars();
        let out = json!({
            "team_a": team_obj(&abbrev_a, &breakdown_a, &chem_a, payroll_a),
            "team_b": team_obj(&abbrev_b, &breakdown_b, &chem_b, payroll_b),
            "deltas": {
                "top8_avg_overall": breakdown_a.top8_avg - breakdown_b.top8_avg,
                "payroll_dollars": payroll_delta,
                "chemistry": chem_a.value - chem_b.value,
            },
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    // Side-by-side text render. Column widths chosen to match charter sample.
    const LABEL_W: usize = 22;
    const COL_W: usize = 16;

    let payroll_delta_cents = Cents(payroll_a.0.saturating_sub(payroll_b.0));
    let payroll_delta_str = if payroll_a.0 == payroll_b.0 {
        "(equal)".to_string()
    } else if payroll_a.0 > payroll_b.0 {
        format!("(Δ +{})", payroll_delta_cents)
    } else {
        // Show the magnitude with a leading minus sign.
        format!("(Δ -{})", payroll_b - payroll_a)
    };

    println!(
        "{:<lw$}{:<cw$} {:<cw$}",
        "",
        abbrev_a,
        abbrev_b,
        lw = LABEL_W,
        cw = COL_W
    );
    println!(
        "{:<lw$}{:<cw$} {:<cw$}",
        "roster size",
        breakdown_a.roster_size,
        breakdown_b.roster_size,
        lw = LABEL_W,
        cw = COL_W
    );
    println!(
        "{:<lw$}{:<cw$} {:<cw$}",
        "top-8 OVR (avg)",
        format!("{:.1}", breakdown_a.top8_avg),
        format!("{:.1}", breakdown_b.top8_avg),
        lw = LABEL_W,
        cw = COL_W
    );
    println!(
        "{:<lw$}{:<cw$} {:<cw$} {}",
        "payroll",
        format!("{}", payroll_a),
        format!("{}", payroll_b),
        payroll_delta_str,
        lw = LABEL_W,
        cw = COL_W
    );
    println!(
        "{:<lw$}{:<cw$} {:<cw$}",
        "chemistry",
        format!("{:.2}", chem_a.value),
        format!("{:.2}", chem_b.value),
        lw = LABEL_W,
        cw = COL_W
    );

    println!();
    println!(
        "  {:<lw$}{:<cw$} {:<cw$}",
        "TOP 8",
        abbrev_a,
        abbrev_b,
        lw = LABEL_W - 2,
        cw = COL_W
    );
    let n = breakdown_a.top8.len().max(breakdown_b.top8.len());
    for i in 0..n {
        let (label_a, cell_a) = match breakdown_a.top8.get(i) {
            Some((pos, _, name, ovr)) => (
                pos.to_string(),
                format!("{} {}", clean_name(name), ovr),
            ),
            None => (String::new(), String::from("-")),
        };
        let cell_b = match breakdown_b.top8.get(i) {
            Some((_, _, name, ovr)) => format!("{} {}", clean_name(name), ovr),
            None => String::from("-"),
        };
        // Use team A's slot label (PG/SG/...) as the row label; team B may
        // diverge but rendering both labels would clutter the layout.
        println!(
            "  {:<lw$}{:<cw$} {:<cw$}",
            label_a,
            cell_a,
            cell_b,
            lw = LABEL_W - 2,
            cw = COL_W
        );
    }

    Ok(())
}

struct CompareBreakdown {
    roster_size: usize,
    top8_avg: f32,
    /// Slot position (best-fit), player id, name, overall.
    top8: Vec<(Position, PlayerId, String, u8)>,
}

fn compare_team_breakdown(
    store: &nba3k_store::Store,
    team_id: TeamId,
    _season: SeasonId,
) -> Result<CompareBreakdown> {
    let mut roster = store.roster_for_team(team_id)?;
    let roster_size = roster.len();
    // Drop active-injury players so the rotation snapshot mirrors `build_snapshot`.
    roster.retain(|p| {
        p.injury
            .as_ref()
            .map(|i| i.games_remaining == 0)
            .unwrap_or(true)
    });
    roster.sort_by(|a, b| b.overall.cmp(&a.overall));
    let top: Vec<_> = roster.into_iter().take(8).collect();

    let top8_avg = if top.is_empty() {
        0.0
    } else {
        top.iter().map(|p| p.overall as f32).sum::<f32>() / top.len() as f32
    };

    let top8: Vec<(Position, PlayerId, String, u8)> = top
        .into_iter()
        .map(|p| (p.primary_position, p.id, p.name.clone(), p.overall))
        .collect();

    Ok(CompareBreakdown {
        roster_size,
        top8_avg,
        top8,
    })
}

// ----------------------------------------------------------------------
// M17 — pre-locked stubs.
// ----------------------------------------------------------------------

fn cmd_offers(app: &mut AppState, limit: u32, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    let snap_owned = build_league_snapshot(app)?;
    let snap = snap_owned.view();

    let store = app.store()?;
    let user_abbrev = store
        .team_abbrev(state.user_team)?
        .unwrap_or_else(|| format!("T{}", state.user_team.0));

    let chains = store.read_open_chains_targeting(state.season, state.user_team)?;

    #[derive(Clone)]
    struct OfferRow {
        id: u64,
        from_abbrev: String,
        wants: String,
        sends: String,
        verdict: &'static str,
        commentary: String,
        probability: f32,
    }

    let mut rows: Vec<OfferRow> = Vec::with_capacity(chains.len());
    let mut rng = ChaCha8Rng::seed_from_u64(state.rng_seed ^ 0xC0FFEE);
    for (id, st) in chains {
        let NegotiationState::Open { chain } = st else { continue };
        let Some(latest) = chain.last() else { continue };

        let from = latest.initiator;
        let from_abbrev = store
            .team_abbrev(from)?
            .unwrap_or_else(|| format!("T{}", from.0));

        // "wants" = players the user team is being asked to send out.
        let wants_pids = latest
            .assets_by_team
            .get(&state.user_team)
            .map(|a| a.players_out.clone())
            .unwrap_or_default();
        // "sends" = the AI initiator's outgoing assets (what user receives).
        let sends_pids = latest
            .assets_by_team
            .get(&from)
            .map(|a| a.players_out.clone())
            .unwrap_or_default();

        let render_pids = |pids: &[PlayerId]| -> String {
            if pids.is_empty() {
                return "(nothing)".to_string();
            }
            pids.iter()
                .map(|p| {
                    store
                        .player_name(*p)
                        .ok()
                        .flatten()
                        .map(|n| clean_name(&n))
                        .unwrap_or_else(|| format!("#{}", p.0))
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let wants = render_pids(&wants_pids);
        let sends = render_pids(&sends_pids);

        // Verdict from the user team's perspective — purely advisory.
        let evaluation = evaluate_mod::evaluate(latest, state.user_team, &snap, &mut rng);
        let verdict = match &evaluation.verdict {
            Verdict::Accept => "accept",
            Verdict::Counter(_) => "counter",
            Verdict::Reject(_) => "reject",
        };

        rows.push(OfferRow {
            id: id.0,
            from_abbrev,
            wants,
            sends,
            verdict,
            commentary: evaluation.commentary,
            probability: evaluation.confidence,
        });
    }

    let take = (limit as usize).min(rows.len());
    let top = &rows[..take];

    if as_json {
        let arr: Vec<_> = top
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "from": r.from_abbrev,
                    "to": user_abbrev,
                    "wants": r.wants,
                    "sends": r.sends,
                    "verdict": r.verdict,
                    "probability": r.probability,
                    "commentary": r.commentary,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if top.is_empty() {
        println!("Incoming offers: none right now.");
        return Ok(());
    }

    println!("Incoming offers:");
    println!(
        "  {:>3}  {:<5} {:<22} {:<32} {:<8}",
        "ID", "FROM", "WANTS", "SENDS", "VERDICT"
    );
    for r in top {
        let mut wants = r.wants.clone();
        if wants.chars().count() > 22 {
            wants = wants.chars().take(21).chain(std::iter::once('…')).collect();
        }
        let mut sends = r.sends.clone();
        if sends.chars().count() > 32 {
            sends = sends.chars().take(31).chain(std::iter::once('…')).collect();
        }
        println!(
            "  {:>3}  {:<5} {:<22} {:<32} {:<8}",
            r.id, r.from_abbrev, wants, sends, r.verdict,
        );
    }
    Ok(())
}

fn cmd_extend(app: &mut AppState, player: &str, salary_m: f64, years: u8) -> Result<()> {
    use nba3k_models::contract_extension::{accept_extension, ExtensionDecision};

    let state = current_state(app)?;
    let user_team = state.user_team;
    let season = state.season;
    let store = app.store()?;

    let roster = store.roster_for_team(user_team)?;
    let needle = player.trim().to_ascii_lowercase();
    let mut hits: Vec<Player> = roster
        .into_iter()
        .filter(|p| p.name.to_ascii_lowercase().contains(&needle))
        .collect();
    if hits.is_empty() {
        bail!(
            "'{}' is not on your roster — extensions only work on user-team players",
            player
        );
    }
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|p| p.name.as_str()).take(5).collect();
        bail!(
            "ambiguous match for '{}': {} candidates (e.g. {:?}). Refine the query.",
            player,
            hits.len(),
            names
        );
    }
    let mut subject = hits.remove(0);
    let display_name = clean_name(&subject.name);

    let offered_salary_cents = (salary_m * 100.0 * 1_000_000.0).round() as i64;
    let offered_salary = Cents(offered_salary_cents);

    let decision = accept_extension(&subject, offered_salary_cents, years, season);

    match decision {
        ExtensionDecision::Accept => {
            let mut existing_years = subject
                .contract
                .as_ref()
                .map(|c| c.years.clone())
                .unwrap_or_default();
            let start_offset = existing_years.len() as u16;
            for i in 0..years {
                existing_years.push(ContractYear {
                    season: SeasonId(season.0 + start_offset + i as u16),
                    salary: offered_salary,
                    guaranteed: true,
                    team_option: false,
                    player_option: false,
                });
            }
            let bird = subject
                .contract
                .as_ref()
                .map(|c| c.bird_rights)
                .unwrap_or(BirdRights::Non);
            subject.contract = Some(Contract {
                years: existing_years,
                signed_in_season: season,
                bird_rights: bird,
            });
            store.upsert_player(&subject)?;

            let total_m = salary_m * years as f64;
            let headline = format!(
                "{} extended {} ({}yr/${:.0}M)",
                store
                    .team_abbrev(user_team)?
                    .unwrap_or_else(|| format!("team {}", user_team.0)),
                display_name,
                years,
                total_m
            );
            store.record_news(season, state.day, "extension", &headline, None)?;

            println!(
                "extended {} {}yr/${:.0}M.",
                display_name, years, total_m
            );
        }
        ExtensionDecision::Counter { request_salary_cents, request_years } => {
            let request_m = request_salary_cents as f64 / 100.0 / 1_000_000.0;
            println!(
                "counter — {} wants ${:.0}M/{}yr.",
                display_name, request_m, request_years
            );
        }
        ExtensionDecision::Reject(reason) => {
            println!("{} rejects: {}.", display_name, reason);
        }
    }

    Ok(())
}

fn cmd_notes_add(app: &mut AppState, player: &str, text: Option<&str>) -> Result<()> {
    let store = app.store()?;
    let p = store
        .find_player_by_name(player)?
        .ok_or_else(|| anyhow!("unknown player '{}'", player))?;
    store.insert_note(p.id, text.unwrap_or(""))?;
    println!("Saved note for {}", clean_name(&p.name));
    Ok(())
}

fn cmd_notes_remove(app: &mut AppState, player: &str) -> Result<()> {
    let store = app.store()?;
    let p = store
        .find_player_by_name(player)?
        .ok_or_else(|| anyhow!("unknown player '{}'", player))?;
    let n = store.delete_note(p.id)?;
    let name = clean_name(&p.name);
    if n == 0 {
        println!("no note for {}", name);
    } else {
        println!("removed note for {}", name);
    }
    Ok(())
}

fn cmd_notes_list(app: &mut AppState, as_json: bool) -> Result<()> {
    let store = app.store()?;
    let notes = store.list_notes()?;

    // Build a single id -> Player index from the active roster pool. Notes
    // can outlive a player (retire / cut), so we tolerate misses and fall
    // back to placeholder fields rather than dropping rows.
    let active = store.all_active_players()?;
    let mut by_id: HashMap<PlayerId, Player> = HashMap::with_capacity(active.len());
    for p in active {
        by_id.insert(p.id, p);
    }

    #[derive(Serialize)]
    struct NoteOut {
        player_id: u32,
        name: String,
        team: String,
        overall: u8,
        text: String,
        created_at: String,
    }

    let mut out: Vec<NoteOut> = Vec::with_capacity(notes.len());
    for n in &notes {
        let (name, team, overall) = match by_id.get(&n.player_id) {
            Some(p) => {
                let team = match p.team {
                    Some(id) => store.team_abbrev(id)?.unwrap_or_else(|| "???".into()),
                    None => "FA".into(),
                };
                (clean_name(&p.name), team, p.overall)
            }
            None => {
                let name = store
                    .player_name(n.player_id)?
                    .map(|s| clean_name(&s))
                    .unwrap_or_else(|| format!("#{}", n.player_id.0));
                (name, "—".into(), 0)
            }
        };
        out.push(NoteOut {
            player_id: n.player_id.0,
            name,
            team,
            overall,
            text: n.text.clone().unwrap_or_default(),
            created_at: n.created_at.clone(),
        });
    }

    if as_json {
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if out.is_empty() {
        println!("no notes tracked.");
        return Ok(());
    }

    println!(
        "Notes ({} player{}):",
        out.len(),
        if out.len() == 1 { "" } else { "s" }
    );
    println!("  {:<22} {:<4} {:>3}  {}", "PLAYER", "TEAM", "OVR", "NOTE");
    for n in &out {
        let text = if n.text.is_empty() { "—" } else { n.text.as_str() };
        println!(
            "  {:<22} {:<4} {:>3}  {}",
            truncate_name(&n.name, 22),
            n.team,
            n.overall,
            text
        );
    }
    Ok(())
}

fn truncate_name(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn cmd_recap(app: &mut AppState, days: u32, as_json: bool) -> Result<()> {
    let state = current_state(app)?;
    let today = day_index_to_date(state.day);
    let cutoff = today - chrono::Duration::days(days as i64);

    let store = app.store()?;
    let games: Vec<GameResult> = store
        .read_games(state.season)?
        .into_iter()
        .filter(|g| g.date >= cutoff && g.date <= today)
        .collect();

    if games.is_empty() {
        if as_json {
            println!("[]");
        } else {
            println!("No games in last {} days.", days);
        }
        return Ok(());
    }

    struct TopLine {
        name: String,
        pts: u8,
        reb: u8,
        ast: u8,
    }

    let pick_top = |store: &nba3k_store::Store, lines: &[PlayerLine]| -> Result<Option<TopLine>> {
        let top = lines.iter().max_by_key(|l| l.pts);
        match top {
            None => Ok(None),
            Some(line) => {
                let name = store
                    .player_name(line.player)?
                    .unwrap_or_else(|| format!("Player#{}", line.player.0));
                Ok(Some(TopLine {
                    name,
                    pts: line.pts,
                    reb: line.reb,
                    ast: line.ast,
                }))
            }
        }
    };

    let mut rows: Vec<(GameResult, String, String, Option<TopLine>, Option<TopLine>)> =
        Vec::with_capacity(games.len());
    for g in games {
        let home_ab = store
            .team_abbrev(g.home)?
            .unwrap_or_else(|| format!("T{}", g.home.0));
        let away_ab = store
            .team_abbrev(g.away)?
            .unwrap_or_else(|| format!("T{}", g.away.0));
        let home_top = pick_top(store, &g.box_score.home_lines)?;
        let away_top = pick_top(store, &g.box_score.away_lines)?;
        rows.push((g, home_ab, away_ab, home_top, away_top));
    }

    if as_json {
        let arr: Vec<_> = rows
            .iter()
            .map(|(g, home_ab, away_ab, home_top, away_top)| {
                let top_json = |t: &Option<TopLine>| match t {
                    Some(t) => json!({
                        "name": t.name,
                        "pts": t.pts,
                        "reb": t.reb,
                        "ast": t.ast,
                    }),
                    None => serde_json::Value::Null,
                };
                json!({
                    "date": g.date.to_string(),
                    "home": home_ab,
                    "away": away_ab,
                    "home_score": g.home_score,
                    "away_score": g.away_score,
                    "home_top": top_json(home_top),
                    "away_top": top_json(away_top),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
    } else {
        for (g, home_ab, away_ab, home_top, away_top) in &rows {
            println!(
                "{} — {} {}, {} {}",
                g.date, home_ab, g.home_score, away_ab, g.away_score
            );
            if let Some(t) = home_top {
                println!(
                    "  {} led {} with {} pts, {} reb, {} ast.",
                    t.name, home_ab, t.pts, t.reb, t.ast
                );
            }
            if let Some(t) = away_top {
                println!(
                    "  {} led {} with {} pts, {} reb, {} ast.",
                    t.name, away_ab, t.pts, t.reb, t.ast
                );
            }
        }
    }

    Ok(())
}

fn cmd_saves_export(_app: &mut AppState, path: PathBuf, to: PathBuf) -> Result<()> {
    if !path.exists() {
        bail!("no such save: {}", path.display());
    }
    let store = nba3k_store::Store::open(&path)
        .with_context(|| format!("opening save {}", path.display()))?;
    let dump = store
        .dump_to_json()
        .with_context(|| format!("dumping save {}", path.display()))?;

    let (table_count, row_count) = match dump.get("tables") {
        Some(serde_json::Value::Object(map)) => {
            let n_tables = map.len();
            let n_rows: usize = map
                .values()
                .map(|v| v.as_array().map(|a| a.len()).unwrap_or(0))
                .sum();
            (n_tables, n_rows)
        }
        _ => (0, 0),
    };

    let pretty = serde_json::to_string_pretty(&dump)?;
    std::fs::write(&to, pretty)
        .with_context(|| format!("writing dump to {}", to.display()))?;

    println!(
        "exported {} → {} ({} tables, {} rows total)",
        path.display(),
        to.display(),
        table_count,
        row_count,
    );
    Ok(())
}
