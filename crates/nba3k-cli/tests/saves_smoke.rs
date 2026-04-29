//! M15-C smoke test: `saves list/show/delete` over a temp save.

use nba3k_core::SeasonId;
use nba3k_store::Store;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

fn nba3k_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nba3k"))
}

fn fresh_save(path: &std::path::Path) {
    let store = Store::open(path).expect("open store");
    store.init_metadata(SeasonId(2026)).expect("init metadata");
    store.set_meta("user_team", "BOS").expect("set user_team");
    drop(store);
}

#[test]
fn saves_show_prints_metadata() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("show.db");
    fresh_save(&save);

    let out = Command::new(nba3k_bin())
        .args(["saves", "show", save.to_str().unwrap()])
        .output()
        .expect("run nba3k saves show");
    assert!(
        out.status.success(),
        "saves show exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("save:"), "missing save line:\n{}", stdout);
    assert!(stdout.contains("BOS"), "missing team abbrev:\n{}", stdout);

    // JSON variant parses.
    let out = Command::new(nba3k_bin())
        .args(["saves", "show", save.to_str().unwrap(), "--json"])
        .output()
        .expect("run nba3k saves show --json");
    assert!(out.status.success(), "saves show --json non-zero");
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("saves show --json must be valid JSON");
    assert_eq!(v["team"], "BOS");
    assert_eq!(v["season"], 2026);
}

#[test]
fn saves_delete_requires_yes_flag() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("noyes.db");
    fresh_save(&save);

    let out = Command::new(nba3k_bin())
        .args(["saves", "delete", save.to_str().unwrap()])
        .output()
        .expect("run nba3k saves delete");
    assert!(
        !out.status.success(),
        "expected non-zero exit without --yes; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--yes"),
        "expected --yes hint in stderr; got:\n{}",
        stderr
    );
    assert!(save.exists(), "save was deleted despite missing --yes flag");
}

#[test]
fn saves_delete_with_yes_removes_file() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("doomed.db");
    fresh_save(&save);
    assert!(save.exists(), "fresh save should exist");

    let out = Command::new(nba3k_bin())
        .args(["saves", "delete", save.to_str().unwrap(), "--yes"])
        .output()
        .expect("run nba3k saves delete --yes");
    assert!(
        out.status.success(),
        "saves delete --yes exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("deleted"),
        "expected `deleted` in stdout; got:\n{}",
        stdout
    );
    assert!(!save.exists(), "file still present after --yes delete");
}

#[test]
fn saves_list_finds_save_in_explicit_dir() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("listed.db");
    fresh_save(&save);

    let out = Command::new(nba3k_bin())
        .args(["saves", "list", "--dir", dir.path().to_str().unwrap()])
        .output()
        .expect("run nba3k saves list");
    assert!(
        out.status.success(),
        "saves list exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("listed.db"), "missing path:\n{}", stdout);
    assert!(stdout.contains("team=BOS"), "missing team:\n{}", stdout);
    assert!(
        stdout.contains("season=2026"),
        "missing season:\n{}",
        stdout
    );

    // JSON variant.
    let out = Command::new(nba3k_bin())
        .args([
            "saves",
            "list",
            "--dir",
            dir.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("run nba3k saves list --json");
    assert!(out.status.success(), "saves list --json non-zero");
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("saves list --json must be valid JSON");
    let arr = v.as_array().expect("top-level must be array");
    assert_eq!(arr.len(), 1, "expected one row, got {:?}", arr);
    assert_eq!(arr[0]["team"], "BOS");
    assert_eq!(arr[0]["season"], 2026);
}

#[test]
fn saves_delete_refuses_currently_open_save() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("open.db");
    fresh_save(&save);

    // Pass --save pointing at the same file we're trying to delete.
    let out = Command::new(nba3k_bin())
        .args([
            "--save",
            save.to_str().unwrap(),
            "saves",
            "delete",
            save.to_str().unwrap(),
            "--yes",
        ])
        .output()
        .expect("run nba3k saves delete on open save");
    assert!(
        !out.status.success(),
        "expected refusal to delete open save; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("currently-open"),
        "expected `currently-open` in stderr; got:\n{}",
        stderr
    );
    assert!(save.exists(), "open save was deleted despite refusal");
}
