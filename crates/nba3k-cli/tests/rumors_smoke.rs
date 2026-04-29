//! M16-B smoke test: `nba3k rumors` returns rows on a fresh save and the
//! JSON shape parses cleanly.
//!
//! Requires the seed DB (`data/seed_2025_26.sqlite`) to be present; mirrors
//! `all_star_smoke.rs` and skips cleanly when the seed is missing.

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

#[test]
fn rumors_returns_rows_on_fresh_save() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m16_rumors.db");
    bootstrap(&save);

    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args(["--save", save.to_str().unwrap(), "rumors", "--limit", "20"])
        .output()
        .expect("nba3k rumors");
    assert!(
        out.status.success(),
        "rumors failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Trade rumors"),
        "header missing:\n{}",
        stdout
    );
    // Heuristic: the table header row + at least one ranked row → 3 lines.
    let line_count = stdout.lines().count();
    assert!(
        line_count >= 3,
        "expected ≥1 rumor row + header; got {} line(s):\n{}",
        line_count,
        stdout
    );
}

#[test]
fn rumors_json_parses() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m16_rumors_json.db");
    bootstrap(&save);

    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "rumors",
            "--limit",
            "5",
            "--json",
        ])
        .output()
        .expect("nba3k rumors --json");
    assert!(
        out.status.success(),
        "rumors --json failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("rumors --json must be valid JSON");
    let arr = v.as_array().expect("rumors json must be an array");
    assert!(!arr.is_empty(), "expected ≥1 rumor on a fresh save");
    let first = &arr[0];
    for key in [
        "rank", "player", "team", "ovr", "role", "interest", "suitors",
    ] {
        assert!(
            !first[key].is_null(),
            "missing `{}` in first row: {}",
            key,
            first
        );
    }
    assert!(first["suitors"].is_array(), "suitors must be array");
    assert_eq!(first["rank"].as_u64(), Some(1));
}
