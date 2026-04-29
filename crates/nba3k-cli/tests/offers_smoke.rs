//! M17-A smoke test: `nba3k offers` surfaces incoming AI proposals after a
//! 30-day sim, and `--json` returns a parseable array.
//!
//! Requires the seed DB (`data/seed_2025_26.sqlite`); skips cleanly when
//! the seed is missing — mirrors `rumors_smoke.rs` / `all_star_smoke.rs`.

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
    let new_status = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "new",
            "--team",
            "BOS",
            "--offline",
        ])
        .status()
        .expect("nba3k new");
    assert!(new_status.success(), "nba3k new failed");
}

fn sim_days(save: &std::path::Path, count: u32) {
    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "sim-day",
            &count.to_string(),
        ])
        .output()
        .expect("nba3k sim-day");
    assert!(
        out.status.success(),
        "sim-day failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn offers_header_renders_after_sim() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m17a_offers.db");
    bootstrap(&save);
    sim_days(&save, 30);

    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args(["--save", save.to_str().unwrap(), "offers", "--limit", "20"])
        .output()
        .expect("nba3k offers");
    assert!(
        out.status.success(),
        "offers failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Either we got at least one offer (header line) or the inbox is empty
    // (still prints the prefix). Both states must mention "Incoming offers".
    assert!(
        stdout.contains("Incoming offers"),
        "missing 'Incoming offers' header:\n{}",
        stdout
    );
}

#[test]
fn offers_json_parses_after_sim() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m17a_offers_json.db");
    bootstrap(&save);
    sim_days(&save, 30);

    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "offers",
            "--limit",
            "20",
            "--json",
        ])
        .output()
        .expect("nba3k offers --json");
    assert!(
        out.status.success(),
        "offers --json failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("offers --json must be valid JSON");
    let arr = v.as_array().expect("offers --json must be an array");
    // 30-day sim with 29 AI teams firing ~0.5%/day each ≈ 4-5 offers in
    // expectation, so an empty inbox would be suspicious — but we don't
    // hard-require it (low-aggression seeds + tight CBA can shut us out).
    if !arr.is_empty() {
        let first = &arr[0];
        for key in ["id", "from", "to", "wants", "sends", "verdict"] {
            assert!(
                !first[key].is_null(),
                "missing `{}` in first offer row: {}",
                key,
                first
            );
        }
        assert_eq!(
            first["to"].as_str(),
            Some("BOS"),
            "to-team should be the user team (BOS)"
        );
        let verdict = first["verdict"].as_str().unwrap_or("");
        assert!(
            ["accept", "counter", "reject"].contains(&verdict),
            "verdict must be accept|counter|reject, got '{}'",
            verdict
        );
    }
}
