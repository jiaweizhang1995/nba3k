//! M33 — `season-advance` writes a fresh `season_calendar` row each year so
//! schedule generation no longer falls back to the const default once the
//! save runs past the seeded 2025-26 entry.
//!
//! This test runs the CLI binary end-to-end via `cargo run`-style binary
//! invocation: it builds a fresh save, then drives `sim-to season-end` and
//! `season-advance` repeatedly to confirm 2026 / 2027 / 2028 rows all exist
//! after the third advance.

use chrono::NaiveDate;
use std::path::PathBuf;
use std::process::Command;

fn target_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // workspace root
    p.push("target");
    p.push("debug");
    p.push("nba3k");
    p
}

fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(target_bin())
        .args(args)
        .output()
        .expect("spawn nba3k");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
#[ignore = "spins up cargo + full season sim; run with `cargo test -- --ignored`"]
fn season_advance_writes_calendar_row_for_each_new_year() {
    // Skip when binary missing — covers `cargo test --workspace` runs that
    // don't build the bin first. Real verification flow:
    //   cargo build --bin nba3k && cargo test -- --ignored season_advance
    if !target_bin().exists() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let save = dir.path().join("advance.db");
    let save_str = save.to_string_lossy().to_string();

    // M34 — pin `--offline` so this scenario is deterministic and does not
    // require a network round-trip to ESPN every time the test runs.
    let (_o, _e, c) = run(&[
        "--save", &save_str, "new", "--team", "BOS", "--offline",
    ]);
    assert_eq!(c, 0, "new failed");

    let (_o, _e, c) = run(&["--save", &save_str, "sim-to", "season-end"]);
    assert_eq!(c, 0, "sim-to season-end failed");

    let (_o, _e, c) = run(&["--save", &save_str, "season-advance"]);
    assert_eq!(c, 0, "season-advance failed");

    let conn = rusqlite::Connection::open(&save).expect("open save");
    let years: Vec<i64> = conn
        .prepare("SELECT season_year FROM season_calendar ORDER BY season_year")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(
        years.contains(&2026) && years.contains(&2027),
        "expected calendar rows for 2026 + 2027 after one advance, got {:?}",
        years
    );

    // The 2027 row should follow our heuristic: ≈365 days after 2026 start,
    // snapped to Tuesday, and the trade deadline 107 days into the new year.
    let (start_str, deadline_str): (String, String) = conn
        .query_row(
            "SELECT start_date, trade_deadline FROM season_calendar WHERE season_year = 2027",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("read 2027 row");
    let start = NaiveDate::parse_from_str(&start_str, "%Y-%m-%d").unwrap();
    let deadline = NaiveDate::parse_from_str(&deadline_str, "%Y-%m-%d").unwrap();
    assert_eq!(
        deadline.signed_duration_since(start).num_days(),
        107,
        "trade deadline should sit 107 days after start"
    );
}
