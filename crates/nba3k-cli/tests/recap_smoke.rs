//! M18-B smoke test: `recap --days N` lists recent games + top scorers.
//!
//! Approach: build a minimal save with two teams + a few players via
//! `nba3k_store::Store`, inject a fully-formed `GameResult` for "today",
//! and assert the CLI's `recap` text + JSON output.

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

const BOS: TeamId = TeamId(1);
const LAL: TeamId = TeamId(2);
const SEASON: SeasonId = SeasonId(2026);

// `day_index_to_date(0) == 2025-10-14`. Pick day 60 ≈ 2025-12-13.
const DAY: u32 = 60;

fn nba3k_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nba3k"))
}

fn make_team(id: TeamId, abbrev: &str) -> Team {
    Team {
        id,
        abbrev: abbrev.into(),
        city: format!("{} City", abbrev),
        name: format!("{}s", abbrev),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
        coach: Coach::default_for(abbrev),
        roster: Vec::new(),
        draft_picks: Vec::new(),
    }
}

fn make_player(id: u32, name: &str, team: Option<TeamId>) -> Player {
    Player {
        id: PlayerId(id),
        name: name.into(),
        primary_position: Position::SF,
        secondary_position: None,
        age: 28,
        overall: 80,
        potential: 80,
        ratings: Ratings::default(),
        contract: None,
        team,
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::Starter,
        morale: 0.5,
    }
}

fn line(player: PlayerId, pts: u8, reb: u8, ast: u8) -> PlayerLine {
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
        three_made: 1,
        three_att: 4,
        ft_made: 2,
        ft_att: 2,
        plus_minus: 0,
    }
}

fn fresh_save_with_game(path: &std::path::Path, game_date: NaiveDate, day: u32) {
    let mut store = Store::open(path).expect("open store");
    store.upsert_team(&make_team(BOS, "BOS")).expect("upsert BOS");
    store.upsert_team(&make_team(LAL, "LAL")).expect("upsert LAL");
    store.set_meta("user_team", "BOS").expect("set user_team");

    // Players: Tatum (BOS, top scorer 38), Brown (BOS, 22),
    //          LeBron (LAL, top scorer 32), AD (LAL, 18).
    let tatum = make_player(1001, "Jayson Tatum", Some(BOS));
    let brown = make_player(1002, "Jaylen Brown", Some(BOS));
    let lebron = make_player(2001, "LeBron James", Some(LAL));
    let ad = make_player(2002, "Anthony Davis", Some(LAL));
    store.bulk_upsert_players(&[tatum, brown, lebron, ad])
        .expect("upsert players");

    let state = SeasonState {
        season: SEASON,
        phase: SeasonPhase::Regular,
        day,
        user_team: BOS,
        mode: GameMode::Standard,
        rng_seed: 1,
    };
    store.save_season_state(&state).expect("save state");

    let box_score = BoxScore {
        home_lines: vec![
            line(PlayerId(1001), 38, 11, 6),
            line(PlayerId(1002), 22, 5, 4),
        ],
        away_lines: vec![
            line(PlayerId(2001), 32, 7, 9),
            line(PlayerId(2002), 18, 12, 2),
        ],
    };

    let game = GameResult {
        id: GameId(1),
        season: SEASON,
        date: game_date,
        home: BOS,
        away: LAL,
        home_score: 112,
        away_score: 108,
        overtime_periods: 0,
        is_playoffs: false,
        box_score,
    };
    store.record_game(&game).expect("record game");
    drop(store);
}

fn day_index_to_date(day: u32) -> NaiveDate {
    let start = NaiveDate::from_ymd_opt(2025, 10, 14).unwrap();
    start + chrono::Duration::days(day as i64)
}

#[test]
fn recap_text_lists_top_scorers() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("recap_text.db");
    let date = day_index_to_date(DAY);
    fresh_save_with_game(&save, date, DAY);

    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "recap", "--days", "1"])
        .output()
        .expect("run nba3k recap");
    assert!(
        out.status.success(),
        "recap exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Score line.
    let score_line = format!("{} — BOS 112, LAL 108", date);
    assert!(
        stdout.contains(&score_line),
        "expected score line `{}`, got:\n{}",
        score_line,
        stdout
    );
    // Top scorer per side.
    assert!(
        stdout.contains("Jayson Tatum led BOS with 38 pts, 11 reb, 6 ast."),
        "missing BOS top scorer line:\n{}",
        stdout
    );
    assert!(
        stdout.contains("LeBron James led LAL with 32 pts, 7 reb, 9 ast."),
        "missing LAL top scorer line:\n{}",
        stdout
    );
}

#[test]
fn recap_json_has_expected_shape() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("recap_json.db");
    let date = day_index_to_date(DAY);
    fresh_save_with_game(&save, date, DAY);

    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "recap",
            "--days",
            "1",
            "--json",
        ])
        .output()
        .expect("run nba3k recap --json");
    assert!(
        out.status.success(),
        "recap --json exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .expect("recap --json must emit valid JSON");
    let arr = v.as_array().expect("top-level must be array");
    assert_eq!(arr.len(), 1, "expected exactly one game; got {:?}", arr);

    let row = &arr[0];
    assert_eq!(row["date"], date.to_string());
    assert_eq!(row["home"], "BOS");
    assert_eq!(row["away"], "LAL");
    assert_eq!(row["home_score"], 112);
    assert_eq!(row["away_score"], 108);
    assert_eq!(row["home_top"]["name"], "Jayson Tatum");
    assert_eq!(row["home_top"]["pts"], 38);
    assert_eq!(row["home_top"]["reb"], 11);
    assert_eq!(row["home_top"]["ast"], 6);
    assert_eq!(row["away_top"]["name"], "LeBron James");
    assert_eq!(row["away_top"]["pts"], 32);
    assert_eq!(row["away_top"]["reb"], 7);
    assert_eq!(row["away_top"]["ast"], 9);
}

#[test]
fn recap_empty_when_no_games_in_window() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("recap_empty.db");

    // Game on day 10, but state.day = 60 → cutoff with --days 1 is day 59.
    let game_date = day_index_to_date(10);
    fresh_save_with_game(&save, game_date, DAY);

    // Text mode.
    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "recap", "--days", "1"])
        .output()
        .expect("run nba3k recap");
    assert!(out.status.success(), "recap exited non-zero on empty window");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No games in last"),
        "expected empty-window message, got:\n{}",
        stdout
    );

    // JSON mode → empty array.
    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "recap",
            "--days",
            "1",
            "--json",
        ])
        .output()
        .expect("run nba3k recap --json");
    assert!(out.status.success(), "recap --json non-zero on empty window");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .expect("recap --json must emit valid JSON even when empty");
    let arr = v.as_array().expect("top-level must be array");
    assert!(arr.is_empty(), "expected empty array, got {:?}", arr);
}
