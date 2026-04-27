//! M13-B smoke test: news feed records state-mutating events.
//!
//! Approach: build a minimal save with two teams + a few players via
//! `nba3k_store::Store`, then drive `nba3k retire` / `nba3k trade propose`
//! via the CLI binary and assert the news rows land via `Store::recent_news`
//! and the `nba3k news` rendering.

use nba3k_core::{
    Coach, Conference, Division, GMArchetype, GMPersonality, GameMode, Player, PlayerId,
    PlayerRole, Position, Ratings, SeasonId, SeasonPhase, SeasonState, Team, TeamId,
};
use nba3k_store::Store;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

const BOS: TeamId = TeamId(1);
const LAL: TeamId = TeamId(2);
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

fn fresh_save(path: &std::path::Path) {
    let mut store = Store::open(path).expect("open store");
    store.upsert_team(&make_team(BOS, "BOS", "Boston", "Celtics"))
        .expect("upsert BOS");
    store.upsert_team(&make_team(LAL, "LAL", "Los Angeles", "Lakers"))
        .expect("upsert LAL");
    store.set_meta("user_team", "BOS").expect("set user_team");
    let state = SeasonState {
        season: SEASON,
        phase: SeasonPhase::Regular,
        day: 45,
        user_team: BOS,
        // God mode so trade-propose unanimous-accepts without going through
        // the evaluator (which needs a full league snapshot).
        mode: GameMode::God,
        rng_seed: 1,
    };
    store.save_season_state(&state).expect("save state");

    // BOS roster: one tradeable starter.
    let mut hauser = make_player(1001, "Sam Hauser", Some(BOS));
    hauser.ratings = Ratings::default();
    let players = vec![hauser];
    store.bulk_upsert_players(&players).expect("upsert BOS roster");

    // LAL roster: a star to be traded for.
    let lebron = make_player(2001, "LeBron James", Some(LAL));
    store.upsert_player(&lebron).expect("upsert LAL star");
    drop(store);
}

#[test]
fn news_starts_empty() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("news_empty.db");
    fresh_save(&save);

    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "news"])
        .output()
        .expect("run nba3k news");
    assert!(
        out.status.success(),
        "news exited non-zero on empty save:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No league news"),
        "expected empty-news message; got:\n{}",
        stdout
    );
}

#[test]
fn news_records_retirement() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("news_retire.db");
    fresh_save(&save);

    // Retire LeBron.
    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "retire",
            "LeBron James",
        ])
        .output()
        .expect("run nba3k retire");
    assert!(
        out.status.success(),
        "retire exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // Read news rows directly from the store.
    let store = Store::open(&save).expect("re-open store");
    let rows = store.recent_news(10).expect("recent_news");
    assert!(
        rows.iter().any(|r| r.kind == "retire" && r.headline.contains("LeBron James")),
        "expected a retire news row; got: {:?}",
        rows.iter().map(|r| (r.kind.clone(), r.headline.clone())).collect::<Vec<_>>()
    );
}

#[test]
fn news_records_god_mode_trade() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("news_trade.db");
    fresh_save(&save);

    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "trade",
            "propose",
            "--from",
            "BOS",
            "--to",
            "LAL",
            "--send",
            "Sam Hauser",
            "--receive",
            "LeBron James",
        ])
        .output()
        .expect("run nba3k trade propose");
    assert!(
        out.status.success(),
        "trade propose exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // The CLI prints "trade #N — verdict: accept | ...". Verify via store.
    let store = Store::open(&save).expect("re-open store");
    let rows = store.recent_news(10).expect("recent_news");
    let trade_row = rows.iter().find(|r| r.kind == "trade");
    assert!(
        trade_row.is_some(),
        "expected a trade news row; got: {:?}",
        rows.iter().map(|r| (r.kind.clone(), r.headline.clone())).collect::<Vec<_>>()
    );
    let head = &trade_row.unwrap().headline;
    assert!(
        head.contains("BOS") && head.contains("LAL"),
        "trade headline missing both teams: {}",
        head
    );
    assert!(
        head.contains("Sam Hauser") && head.contains("LeBron James"),
        "trade headline missing both players: {}",
        head
    );
}

#[test]
fn news_command_renders_text_and_json() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("news_render.db");
    fresh_save(&save);

    // Drop a manual row so the renderer has something to print.
    {
        let store = Store::open(&save).expect("open");
        store
            .record_news(SEASON, 10, "trade", "BOS sends X to LAL for Y", None)
            .expect("record_news");
        drop(store);
    }

    // Text render.
    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "news", "--limit", "5"])
        .output()
        .expect("run nba3k news");
    assert!(out.status.success(), "news exited non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Recent league news"), "header missing:\n{}", stdout);
    assert!(stdout.contains("[trade"), "kind tag missing:\n{}", stdout);
    assert!(stdout.contains("BOS sends X to LAL for Y"), "headline missing:\n{}", stdout);

    // JSON render.
    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "news", "--json"])
        .output()
        .expect("run nba3k news --json");
    assert!(out.status.success(), "news --json exited non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("news --json must emit valid JSON");
    let arr = v.as_array().expect("top-level must be array");
    assert!(!arr.is_empty(), "expected at least one row in JSON");
    let row = &arr[0];
    assert_eq!(row["kind"], "trade");
    assert_eq!(row["headline"], "BOS sends X to LAL for Y");
    assert_eq!(row["season"], 2026);
}
