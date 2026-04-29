//! Fixture-driven tests for the ESPN client parsers. Live HTTP is never
//! exercised here — every parse_* function is fed a checked-in JSON sample
//! captured during M31 development. If ESPN changes its schema, these
//! tests catch it before the live importer does.

use chrono::NaiveDate;
use nba3k_scrape::sources::espn;
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/espn");
    p.push(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read fixture {p:?}: {e}"))
}

#[test]
fn parse_teams_returns_30() {
    let bytes = fixture("teams.json");
    let teams = espn::parse_teams(&bytes).expect("parse teams");
    assert_eq!(teams.len(), 30);
    let bos = teams.iter().find(|t| t.abbrev == "BOS").expect("BOS exists");
    assert!(bos.display_name.contains("Boston"));
    assert!(bos.id > 0);
}

#[test]
fn parse_standings_returns_30_with_records() {
    let bytes = fixture("standings_2026.json");
    let rows = espn::parse_standings(&bytes).expect("parse standings");
    assert_eq!(rows.len(), 30, "expect 30 standings rows");
    // Spot-check: the league has a non-zero number of total wins.
    let total_w: u32 = rows.iter().map(|r| r.w as u32).sum();
    assert!(
        total_w > 100,
        "league total wins should be > 100 mid-season, got {total_w}"
    );
    // Conference field is populated.
    assert!(rows.iter().all(|r| !r.conf.is_empty()));
}

#[test]
fn parse_scoreboard_finds_completed_lal_game() {
    // Fixture pulled for 2026-01-28 — LAL @ CLE on that night.
    let bytes = fixture("scoreboard_20260128.json");
    let games = espn::parse_scoreboard(&bytes).expect("parse scoreboard");
    assert!(!games.is_empty(), "scoreboard has events on this date");
    let lal = games
        .iter()
        .find(|g| g.away_abbrev == "LAL" || g.home_abbrev == "LAL")
        .expect("LAL appears on 2026-01-28");
    assert!(lal.completed);
    assert!(lal.home_pts.is_some() && lal.away_pts.is_some());
    assert_eq!(lal.date, NaiveDate::from_ymd_opt(2026, 1, 29).unwrap());
}

#[test]
fn parse_roster_finds_doncic_with_injury() {
    let bytes = fixture("roster_lal.json");
    let (abbrev, roster) = espn::parse_roster(&bytes).expect("parse roster");
    assert_eq!(abbrev, "LAL");
    let luka = roster
        .iter()
        .find(|r| r.display_name == "Luka Doncic")
        .expect("Doncic in LAL roster");
    assert!(luka.injury_status.is_some(), "Luka has injury status set");
}

#[test]
fn parse_player_stats_yields_per_game_averages() {
    let bytes = fixture("player_stats_2026.json");
    let stats = espn::parse_player_stats(&bytes).expect("parse player stats");
    assert!(!stats.is_empty(), "fixture trimmed to first N athletes");
    let first = &stats[0];
    assert!(first.gp > 0, "{} has games played", first.display_name);
    assert!(first.ppg > 0.0, "{} has ppg", first.display_name);
    assert!(first.mpg > 10.0, "{} has minutes", first.display_name);
    // FG% must round-trip to 0..=1 (we divide by 100).
    assert!(first.fg_pct > 0.0 && first.fg_pct <= 1.0);
}

#[test]
fn parse_news_trades_returns_items() {
    let bytes = fixture("news_trades.json");
    let items = espn::parse_news_trades(&bytes).expect("parse news");
    assert!(!items.is_empty(), "news fixture has at least one item");
    let it = &items[0];
    assert!(!it.headline.is_empty());
}
