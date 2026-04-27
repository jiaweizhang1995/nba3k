//! M14-C smoke tests for `nba3k records`.
//! Covers: 30-game simmed save returns at least 5 PPG leaders, unknown stat
//! produces a clean error, and a fresh save under career scope reports the
//! "no qualifying players" message.

use chrono::NaiveDate;
use nba3k_core::{
    BoxScore, Coach, Conference, Division, GMArchetype, GMPersonality, GameId, GameMode,
    GameResult, Player, PlayerId, PlayerLine, PlayerRole, Position, Ratings, SeasonId,
    SeasonPhase, SeasonState, Team, TeamId,
};
use nba3k_store::Store;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

const HOME: TeamId = TeamId(1);
const AWAY: TeamId = TeamId(2);
const SEASON: SeasonId = SeasonId(2026);

fn nba3k_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nba3k"))
}

fn make_team(id: TeamId, abbrev: &str, conf: Conference, div: Division) -> Team {
    Team {
        id,
        abbrev: abbrev.into(),
        city: "City".into(),
        name: "Team".into(),
        conference: conf,
        division: div,
        gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
        coach: Coach::default_for(abbrev),
        roster: Vec::new(),
        draft_picks: Vec::new(),
    }
}

fn make_player(id: u32, name: &str, team: TeamId, pos: Position) -> Player {
    Player {
        id: PlayerId(id),
        name: name.into(),
        primary_position: pos,
        secondary_position: None,
        age: 26,
        overall: 80,
        potential: 80,
        ratings: Ratings::default(),
        contract: None,
        team: Some(team),
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

fn line_for(player: PlayerId, pts: u8, reb: u8, ast: u8) -> PlayerLine {
    PlayerLine {
        player,
        minutes: 36,
        pts,
        reb,
        ast,
        stl: 1,
        blk: 0,
        tov: 2,
        fg_made: pts / 2,
        fg_att: pts,
        three_made: 0,
        three_att: 0,
        ft_made: 0,
        ft_att: 0,
        plus_minus: 0,
    }
}

fn fresh_store(path: &std::path::Path) -> Store {
    let store = Store::open(path).expect("open store");
    store
        .upsert_team(&make_team(HOME, "BOS", Conference::East, Division::Atlantic))
        .expect("upsert home");
    store
        .upsert_team(&make_team(AWAY, "NYK", Conference::East, Division::Atlantic))
        .expect("upsert away");
    let st = SeasonState {
        season: SEASON,
        phase: SeasonPhase::Regular,
        day: 60,
        user_team: HOME,
        mode: GameMode::Standard,
        rng_seed: 42,
    };
    store.save_season_state(&st).expect("save state");
    store
}

#[test]
fn records_unknown_stat_errors_cleanly() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("unknown_stat.db");
    let _store = fresh_store(&save);

    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "records",
            "--stat",
            "bogus",
        ])
        .output()
        .expect("run nba3k records");
    assert!(
        !out.status.success(),
        "records --stat bogus must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown stat") && stderr.contains("ppg"),
        "expected helpful error listing supported stats; stderr was:\n{}",
        stderr,
    );
}

#[test]
fn records_career_on_fresh_save_reports_empty() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("career_empty.db");
    let _store = fresh_store(&save);

    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "records",
            "--scope",
            "career",
            "--stat",
            "ppg",
        ])
        .output()
        .expect("run nba3k records --scope career");
    assert!(
        out.status.success(),
        "records --scope career exited non-zero on fresh save:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No qualifying players") && stdout.contains("100 GP"),
        "expected empty-career message; got:\n{}",
        stdout,
    );
}

/// Build 30 games where multiple players cross the 20-GP min — top-N PPG must
/// have at least 5 entries and be sorted descending.
#[test]
fn records_season_ppg_after_30_games() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("simmed.db");
    let store = fresh_store(&save);

    // 6 players (3 per team), all play 30 games each. Different scoring tiers
    // so the leaderboard has a clear order.
    let players = [
        (101u32, "Star One", HOME, Position::SF, 35u8),
        (102, "Star Two", HOME, Position::PG, 28),
        (103, "Star Three", HOME, Position::C, 22),
        (201, "Visitor One", AWAY, Position::SG, 30),
        (202, "Visitor Two", AWAY, Position::PF, 24),
        (203, "Visitor Three", AWAY, Position::C, 18),
    ];
    for (id, name, team, pos, _pts) in players.iter() {
        let p = make_player(*id, name, *team, *pos);
        store.upsert_player(&p).expect("upsert player");
    }

    for i in 0..30u32 {
        let game = GameResult {
            id: GameId((i + 1) as u64),
            season: SEASON,
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                + chrono::Duration::days(i as i64),
            home: HOME,
            away: AWAY,
            home_score: 110,
            away_score: 100,
            box_score: BoxScore {
                home_lines: vec![
                    line_for(PlayerId(101), 35, 8, 8),
                    line_for(PlayerId(102), 28, 4, 6),
                    line_for(PlayerId(103), 22, 11, 2),
                ],
                away_lines: vec![
                    line_for(PlayerId(201), 30, 5, 5),
                    line_for(PlayerId(202), 24, 7, 3),
                    line_for(PlayerId(203), 18, 9, 1),
                ],
            },
            overtime_periods: 0,
            is_playoffs: false,
        };
        store.record_game(&game).expect("record game");
    }
    drop(store);

    // Text view.
    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "records",
            "--scope",
            "season",
            "--stat",
            "ppg",
            "--limit",
            "10",
        ])
        .output()
        .expect("run nba3k records");
    assert!(
        out.status.success(),
        "records exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("PPG"), "missing PPG header:\n{}", stdout);
    assert!(
        stdout.contains("Star One"),
        "missing top scorer:\n{}",
        stdout
    );

    // JSON view: must have at least 5 rows, sorted desc by `value`, all with games >= 20.
    let out_json = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "records",
            "--scope",
            "season",
            "--stat",
            "ppg",
            "--limit",
            "10",
            "--json",
        ])
        .output()
        .expect("run nba3k records --json");
    assert!(out_json.status.success(), "records --json exited non-zero");
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout_json).expect("must emit valid JSON");
    let rows = v["rows"].as_array().expect("rows must be array");
    assert!(
        rows.len() >= 5,
        "expected at least 5 leaderboard rows after 30-game sim; got {}",
        rows.len()
    );
    let mut prev: f64 = f64::INFINITY;
    for row in rows {
        let val = row["value"].as_f64().expect("value must be a number");
        let games = row["games"].as_u64().expect("games must be a number");
        assert!(games >= 20, "min-GP filter must drop sub-20 GP players");
        assert!(
            val <= prev + 1e-6,
            "rows must be sorted desc; saw {} after {}",
            val,
            prev
        );
        prev = val;
    }
    // Top entry should be Star One (35 PPG).
    assert_eq!(rows[0]["name"], "Star One");
}
