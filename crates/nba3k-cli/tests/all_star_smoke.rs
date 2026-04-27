//! M15-A smoke test: drive a real save through `sim-day 41` and assert the
//! All-Star roster is recorded + `cmd_all_star --json` parses.
//!
//! Requires the seed DB (`data/seed_2025_26.sqlite`) to be present. Skipped
//! cleanly when missing — same pattern as `integration_season1.rs`.

use nba3k_store::Store;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn nba3k_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nba3k"))
}

fn seed_present() -> bool {
    workspace_root().join("data/seed_2025_26.sqlite").exists()
}

fn bootstrap(save: &std::path::Path) {
    let root = workspace_root();
    let new_status = Command::new(nba3k_bin())
        .current_dir(&root)
        .args([
            "--save",
            save.to_str().unwrap(),
            "new",
            "--team",
            "BOS",
        ])
        .status()
        .expect("nba3k new");
    assert!(new_status.success(), "nba3k new failed");
}

#[test]
fn all_star_recorded_after_sim_day_41() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m15_all_star.db");
    bootstrap(&save);

    // Sim well past day 41 so each team has played the 20+ games the
    // all-star eligibility gate requires. Day 41 fires the trigger, but the
    // pool may be thin until ~day 60 when most teams clear the 20-game floor.
    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "sim-day",
            "80",
        ])
        .output()
        .expect("nba3k sim-day");
    assert!(
        out.status.success(),
        "sim-day failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // The day-41 hook persists at least one all-star row.
    let store = Store::open(&save).expect("open save");
    let state = store
        .load_season_state()
        .expect("load state")
        .expect("state present");
    let rows = store.read_all_star(state.season).expect("read_all_star");
    assert!(
        !rows.is_empty(),
        "expected ≥1 all-star row after crossing day 41; got 0"
    );

    // News feed should carry the `all_star` row from the day-41 trigger.
    let news = store.recent_news(50).expect("recent_news");
    assert!(
        news.iter().any(|r| r.kind == "all_star"),
        "expected an `all_star` news entry; got: {:?}",
        news.iter().map(|r| r.kind.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn all_star_json_parses_after_trigger() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m15_all_star_json.db");
    bootstrap(&save);

    // Cross the day-41 marker with enough volume that ≥20 games per team
    // are recorded (matches the awards-engine eligibility gate).
    let sim = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args(["--save", save.to_str().unwrap(), "sim-day", "80"])
        .output()
        .expect("nba3k sim-day");
    assert!(sim.status.success(), "sim-day failed");

    // `all-star --json` for the current season must emit a parseable shape
    // with the four roster vectors.
    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args(["--save", save.to_str().unwrap(), "all-star", "--json"])
        .output()
        .expect("nba3k all-star --json");
    assert!(
        out.status.success(),
        "all-star --json failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("must emit valid JSON");
    for key in ["east_starters", "east_reserves", "west_starters", "west_reserves"] {
        assert!(v[key].is_array(), "missing/non-array `{}` in: {}", key, stdout);
    }
    assert!(v["season"].is_u64(), "missing season: {}", stdout);
    let total = v["east_starters"].as_array().unwrap().len()
        + v["east_reserves"].as_array().unwrap().len()
        + v["west_starters"].as_array().unwrap().len()
        + v["west_reserves"].as_array().unwrap().len();
    assert!(total > 0, "expected ≥1 all-star selection; got 0 in: {}", stdout);
}

#[test]
fn all_star_returns_empty_payload_before_trigger() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m15_all_star_empty.db");
    bootstrap(&save);

    // Don't sim — roster is empty. `--json` must still parse and the four
    // vectors must be empty arrays.
    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args(["--save", save.to_str().unwrap(), "all-star", "--json"])
        .output()
        .expect("nba3k all-star --json");
    assert!(out.status.success(), "all-star --json failed before trigger");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("must emit valid JSON pre-trigger");
    assert_eq!(v["east_starters"].as_array().unwrap().len(), 0);
    assert_eq!(v["east_reserves"].as_array().unwrap().len(), 0);
    assert_eq!(v["west_starters"].as_array().unwrap().len(), 0);
    assert_eq!(v["west_reserves"].as_array().unwrap().len(), 0);
}
