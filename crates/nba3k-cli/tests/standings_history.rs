//! M15-B smoke test: per-season standings recall.
//!
//! `nba3k standings --season N` must return the standings table that was
//! frozen for season N, even after `season-advance` rolls state forward.
//! Approach: seed the `standings` table with two distinct season rows via
//! `nba3k_store::Store`, then drive the binary with `--season` and assert
//! the JSON output reflects the expected season's data.

use nba3k_core::{
    Coach, Conference, Division, GMArchetype, GMPersonality, GameMode, SeasonId, SeasonPhase,
    SeasonState, Team, TeamId,
};
use nba3k_store::Store;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

fn nba3k_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nba3k"))
}

fn make_team(id: u8, abbrev: &str, conf: Conference, div: Division) -> Team {
    Team {
        id: TeamId(id),
        abbrev: abbrev.into(),
        city: abbrev.into(),
        name: abbrev.into(),
        conference: conf,
        division: div,
        gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
        coach: Coach::default_for(abbrev),
        roster: Vec::new(),
        draft_picks: Vec::new(),
    }
}

/// Build a save with 30 stub teams and standings rows for two seasons.
/// Season 2026 totals exactly 1230 wins (full 82-game schedule for 30 teams).
/// Season 2027 uses a different shape so the rows are distinguishable.
fn seed_two_season_save(path: &std::path::Path, current_season: SeasonId) {
    // Spread 30 teams across the four conference/division combos enough to
    // exercise both halves of the league. Specific divisions don't matter
    // for this read-back smoke test.
    let conf_div = [
        (Conference::East, Division::Atlantic),
        (Conference::East, Division::Central),
        (Conference::East, Division::Southeast),
        (Conference::West, Division::Northwest),
        (Conference::West, Division::Pacific),
        (Conference::West, Division::Southwest),
    ];

    let store = Store::open(path).expect("open store");
    let mut teams = Vec::with_capacity(30);
    for i in 0..30u8 {
        let abbrev = format!("T{:02}", i + 1);
        let (c, d) = conf_div[(i as usize) % conf_div.len()];
        let t = make_team(i + 1, &abbrev, c, d);
        store.upsert_team(&t).expect("upsert team");
        teams.push(t);
    }
    store.set_meta("user_team", "T01").expect("set user_team");

    // Season 2026 — half the teams go 50-32, half 32-50. Total wins = 1230,
    // total losses = 1230, total games = 1230 * 2 / 2 = matches the 82-game,
    // 30-team league schedule.
    for (i, t) in teams.iter().enumerate() {
        let (w, l) = if i < 15 {
            (50u16, 32u16)
        } else {
            (32u16, 50u16)
        };
        store
            .upsert_standing(t.id, SeasonId(2026), w, l, None)
            .expect("upsert standing 2026");
    }

    // Season 2027 — a clearly different distribution (every team 41-41) so
    // the read-back can't be confused with the prior season.
    for t in &teams {
        store
            .upsert_standing(t.id, SeasonId(2027), 41, 41, None)
            .expect("upsert standing 2027");
    }

    let state = SeasonState {
        season: current_season,
        phase: SeasonPhase::Regular,
        day: 1,
        user_team: TeamId(1),
        mode: GameMode::God,
        rng_seed: 1,
    };
    store.save_season_state(&state).expect("save state");
    drop(store);
}

fn run_standings_json(save: &std::path::Path, season: u16) -> serde_json::Value {
    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "standings",
            "--season",
            &season.to_string(),
            "--json",
        ])
        .output()
        .expect("run nba3k standings");
    assert!(
        out.status.success(),
        "standings --season {} exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        season,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).expect("standings --json must emit valid JSON")
}

fn total_wins(rows: &serde_json::Value) -> u32 {
    rows.as_array()
        .expect("array")
        .iter()
        .map(|r| r["wins"].as_u64().expect("wins") as u32)
        .sum()
}

#[test]
fn standings_recall_prior_season_after_advance() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("standings_history.db");
    // Current season is 2027 — same as if `season-advance` rolled from 2026.
    seed_two_season_save(&save, SeasonId(2027));

    let prior = run_standings_json(&save, 2026);
    let current = run_standings_json(&save, 2027);

    assert_eq!(
        prior.as_array().map(|a| a.len()).unwrap_or(0),
        30,
        "expected 30 teams in 2026 standings, got: {}",
        prior
    );
    assert_eq!(
        current.as_array().map(|a| a.len()).unwrap_or(0),
        30,
        "expected 30 teams in 2027 standings, got: {}",
        current
    );

    // 2026 was seeded as 50/32 vs 32/50 across 30 teams = 1230 total wins.
    assert_eq!(
        total_wins(&prior),
        1230,
        "season 2026 should have a full 82-game-equivalent total wins (1230); rows: {}",
        prior
    );
    // 2027 was seeded as 41/41 across 30 teams = 1230 too, but the per-row
    // shape is different — rank-1 should be 41-41, not 50-32.
    let current_top = &current.as_array().expect("array")[0];
    assert_eq!(current_top["wins"].as_u64(), Some(41));
    assert_eq!(current_top["losses"].as_u64(), Some(41));

    // And rank-1 of the prior season must be a 50-32 team — confirms the
    // two seasons aren't aliased onto the same row set.
    let prior_top = &prior.as_array().expect("array")[0];
    assert_eq!(prior_top["wins"].as_u64(), Some(50));
    assert_eq!(prior_top["losses"].as_u64(), Some(32));
}

#[test]
fn standings_default_uses_current_season() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("standings_default.db");
    seed_two_season_save(&save, SeasonId(2027));

    let out = Command::new(nba3k_bin())
        .args(["--save", save.to_str().unwrap(), "standings", "--json"])
        .output()
        .expect("run nba3k standings");
    assert!(out.status.success(), "standings exited non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("standings --json must emit valid JSON");
    // Default (no --season) should resolve to current season 2027 — every
    // team is 41-41 in that snapshot.
    let top = &v.as_array().expect("array")[0];
    assert_eq!(top["wins"].as_u64(), Some(41));
    assert_eq!(top["losses"].as_u64(), Some(41));
}
