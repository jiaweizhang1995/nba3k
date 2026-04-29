//! M13-C smoke test: drive `nba3k awards-race` against hand-built and
//! seed-driven saves. Covers: insufficient-games path, JSON parse, and a
//! 30-game simmed save returning a non-empty MVP ballot.

use chrono::NaiveDate;
use nba3k_core::{
    BoxScore, Coach, Conference, Division, GMArchetype, GMPersonality, GameId, GameMode,
    GameResult, Player, PlayerId, PlayerLine, PlayerRole, Position, Ratings, SeasonId, SeasonPhase,
    SeasonState, Team, TeamId,
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
        .upsert_team(&make_team(
            HOME,
            "BOS",
            Conference::East,
            Division::Atlantic,
        ))
        .expect("upsert home");
    store
        .upsert_team(&make_team(
            AWAY,
            "NYK",
            Conference::East,
            Division::Atlantic,
        ))
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
fn awards_race_insufficient_games_message() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("empty.db");
    let _store = fresh_store(&save);

    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "awards-race"])
        .output()
        .expect("run nba3k awards-race");
    assert!(
        out.status.success(),
        "awards-race exited non-zero on empty save:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("insufficient games"),
        "expected insufficient-games message; got:\n{}",
        stdout
    );
}

#[test]
fn awards_race_json_parses_when_insufficient() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("empty_json.db");
    let _store = fresh_store(&save);

    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "awards-race", "--json"])
        .output()
        .expect("run nba3k awards-race --json");
    assert!(out.status.success(), "awards-race --json exited non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("must emit valid JSON");
    assert_eq!(v["insufficient_games"], serde_json::Value::Bool(true));
    assert_eq!(v["season"], 2026);
}

/// Build a save with 30 simulated games where one player dominates so the
/// MVP ballot has a clear leader.
#[test]
fn awards_race_returns_non_empty_mvp_after_30_games() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("simmed.db");
    let store = fresh_store(&save);

    // Star scorer on HOME, also a role player on each side so games have
    // box-score lines for both teams.
    let star = make_player(101, "Star Scorer", HOME, Position::SF);
    let home_role = make_player(102, "Home Role", HOME, Position::PG);
    let away_role = make_player(201, "Away Role", AWAY, Position::PG);
    store.upsert_player(&star).expect("upsert star");
    store.upsert_player(&home_role).expect("upsert home role");
    store.upsert_player(&away_role).expect("upsert away role");

    // 30 games, all HOME wins to keep win pct high (so MVP team-success gate
    // doesn't zero out the composite). Star drops a 35/8/8 line every game.
    for i in 0..30u32 {
        let game = GameResult {
            id: GameId((i + 1) as u64),
            season: SEASON,
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Duration::days(i as i64),
            home: HOME,
            away: AWAY,
            home_score: 110,
            away_score: 100,
            box_score: BoxScore {
                home_lines: vec![
                    line_for(star.id, 35, 8, 8),
                    line_for(home_role.id, 12, 4, 3),
                ],
                away_lines: vec![line_for(away_role.id, 14, 5, 4)],
            },
            overtime_periods: 0,
            is_playoffs: false,
        };
        store.record_game(&game).expect("record game");
    }
    drop(store);

    // Text run: must contain the MVP section and the star's name.
    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "awards-race"])
        .output()
        .expect("run nba3k awards-race");
    assert!(
        out.status.success(),
        "awards-race exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Award race"), "missing header:\n{}", stdout);
    assert!(stdout.contains("MVP"), "missing MVP section:\n{}", stdout);
    assert!(
        stdout.contains("Star Scorer"),
        "MVP ballot did not include the dominant scorer:\n{}",
        stdout
    );

    // JSON run: top entry's vote share must be >= every other entry.
    let out_json = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "awards-race", "--json"])
        .output()
        .expect("run nba3k awards-race --json");
    assert!(
        out_json.status.success(),
        "awards-race --json exited non-zero"
    );
    let stdout_json = String::from_utf8_lossy(&out_json.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout_json).expect("must emit valid JSON");
    let mvp = v["mvp"].as_array().expect("mvp must be array");
    assert!(
        !mvp.is_empty(),
        "MVP ballot must be non-empty after 30 games"
    );
    let top_share = mvp[0]["share"].as_f64().expect("share must be a number");
    assert!(top_share > 0.0, "top share must be positive");
    for entry in mvp.iter().skip(1) {
        let s = entry["share"].as_f64().unwrap_or(0.0);
        assert!(
            top_share >= s,
            "ballot must be sorted by share desc; top={} other={}",
            top_share,
            s
        );
    }
    // Top entry should be our dominant Star Scorer.
    assert_eq!(mvp[0]["name"], "Star Scorer");
}
