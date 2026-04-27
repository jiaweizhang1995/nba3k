//! M12-B smoke test: drive `nba3k hof` against a hand-built save DB so the
//! command's output format and ordering can be asserted without needing the
//! full scrape pipeline.
//!
//! Approach: use `nba3k_store::Store` to set up a tempfile DB with one team,
//! a couple of retired players, and one synthetic game per player so career
//! stats are non-empty. Then exec the CLI binary with `--save <path> hof`
//! and parse the stdout.

use chrono::NaiveDate;
use nba3k_core::{
    BoxScore, Coach, Conference, Division, GMArchetype, GMPersonality, GameId, GameResult,
    Player, PlayerId, PlayerLine, PlayerRole, Position, Ratings, SeasonId, Team, TeamId,
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

fn make_team(id: TeamId, abbrev: &str, city: &str, name: &str) -> Team {
    Team {
        id,
        abbrev: abbrev.into(),
        city: city.into(),
        name: name.into(),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
        coach: Coach::default_for(abbrev),
        roster: Vec::new(),
        draft_picks: Vec::new(),
    }
}

fn fresh_store(path: &std::path::Path) -> Store {
    let store = Store::open(path).expect("open store");
    store.upsert_team(&make_team(HOME, "BOS", "Boston", "Celtics")).expect("upsert home");
    store.upsert_team(&make_team(AWAY, "NYK", "New York", "Knicks")).expect("upsert away");
    store
}

fn make_player(id: u32, name: &str, pos: Position) -> Player {
    Player {
        id: PlayerId(id),
        name: name.into(),
        primary_position: pos,
        secondary_position: None,
        age: 38,
        overall: 80,
        potential: 80,
        ratings: Ratings::default(),
        contract: None,
        team: None,
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

#[test]
fn hof_empty_save_prints_empty_message() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("empty.db");
    // Just the table set — no players retired.
    let _store = fresh_store(&save);

    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "hof"])
        .output()
        .expect("run nba3k hof");
    assert!(
        out.status.success(),
        "hof exited non-zero on empty save:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Hall of Fame: empty (no retired players yet)."),
        "expected empty-HOF message; got:\n{}",
        stdout
    );
}

#[test]
fn hof_lists_retired_player_with_career_totals() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("hof.db");
    let store = fresh_store(&save);

    // Two retired players: a high-scoring star (should rank #1) and a role
    // player (should rank #2). Both must appear in the table.
    let star = make_player(1001, "LeBron James", Position::SF);
    let role = make_player(1002, "Average Joe", Position::PG);
    store.upsert_player(&star).expect("upsert star");
    store.upsert_player(&role).expect("upsert role");

    // Synthetic single-game box score: star drops a 40/8/8 line, role drops 8/2/3.
    // Career totals after one game: star = 40 PTS, role = 8 PTS.
    let game = GameResult {
        id: GameId(1),
        season: SEASON,
        date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
        home: HOME,
        away: TeamId(2),
        home_score: 110,
        away_score: 100,
        box_score: BoxScore {
            home_lines: vec![
                line_for(star.id, 40, 8, 8),
                line_for(role.id, 8, 2, 3),
            ],
            away_lines: Vec::new(),
        },
        overtime_periods: 0,
        is_playoffs: false,
    };
    store.record_game(&game).expect("record game");

    // Retire after the box score so they show up via list_retired_players.
    store.set_player_retired(star.id).expect("retire star");
    store.set_player_retired(role.id).expect("retire role");

    // Drop the store handle so the binary can re-open the DB cleanly.
    drop(store);

    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "hof", "--limit", "5"])
        .output()
        .expect("run nba3k hof");
    assert!(
        out.status.success(),
        "hof exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("Hall of Fame"), "missing header:\n{}", stdout);
    assert!(stdout.contains("LeBron James"), "star missing:\n{}", stdout);
    assert!(stdout.contains("Average Joe"), "role player missing:\n{}", stdout);

    // The star outscored the role player, so #1 must be LeBron.
    let lebron_idx = stdout.find("LeBron James").expect("lebron present");
    let joe_idx = stdout.find("Average Joe").expect("joe present");
    assert!(
        lebron_idx < joe_idx,
        "expected LeBron above Average Joe by career PTS; got:\n{}",
        stdout
    );

    // Career PTS for the star = 40 (one game). Make sure the number renders.
    assert!(stdout.contains(" 40 "), "career PTS=40 missing in:\n{}", stdout);
}

#[test]
fn hof_json_emits_structured_array() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("hof_json.db");
    let store = fresh_store(&save);

    let p = make_player(2001, "Test Player", Position::SG);
    store.upsert_player(&p).expect("upsert");
    let game = GameResult {
        id: GameId(2),
        season: SEASON,
        date: NaiveDate::from_ymd_opt(2026, 4, 2).unwrap(),
        home: HOME,
        away: TeamId(2),
        home_score: 100,
        away_score: 99,
        box_score: BoxScore {
            home_lines: vec![line_for(p.id, 30, 5, 4)],
            away_lines: Vec::new(),
        },
        overtime_periods: 0,
        is_playoffs: false,
    };
    store.record_game(&game).expect("record game");
    store.set_player_retired(p.id).expect("retire");
    drop(store);

    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "hof", "--json"])
        .output()
        .expect("run nba3k hof --json");
    assert!(out.status.success(), "hof --json exited non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("hof --json must emit valid JSON");
    let arr = v.as_array().expect("top-level must be array");
    assert_eq!(arr.len(), 1);
    let row = &arr[0];
    assert_eq!(row["rank"], 1);
    assert_eq!(row["name"], "Test Player");
    assert_eq!(row["pos"], "SG");
    assert_eq!(row["pts"], 30);
}
