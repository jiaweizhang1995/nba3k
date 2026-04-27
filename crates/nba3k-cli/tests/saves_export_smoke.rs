//! M18-C smoke test: `saves export <path> --to <json>` dumps every
//! persistent table out of a fresh save into a portable JSON file.

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
    store
        .init_metadata(SeasonId(2026))
        .expect("init metadata");
    store.set_meta("user_team", "BOS").expect("set user_team");
    drop(store);
}

#[test]
fn saves_export_writes_pretty_json_with_all_tables() {
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("export.db");
    let dump = dir.path().join("export.json");
    fresh_save(&save);

    let out = Command::new(nba3k_bin())
        .args([
            "saves",
            "export",
            save.to_str().unwrap(),
            "--to",
            dump.to_str().unwrap(),
        ])
        .output()
        .expect("run nba3k saves export");
    assert!(
        out.status.success(),
        "saves export exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("exported"),
        "missing `exported` line in stdout:\n{}",
        stdout,
    );
    assert!(
        stdout.contains("tables"),
        "missing `tables` count in stdout:\n{}",
        stdout,
    );

    let bytes = std::fs::read(&dump).expect("read dump file");
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).expect("dump must be valid JSON");

    let tables = v
        .get("tables")
        .and_then(|t| t.as_object())
        .expect("top-level `tables` must be an object");

    assert!(
        tables.len() >= 5,
        "expected at least 5 tables in dump, got {} ({:?})",
        tables.len(),
        tables.keys().collect::<Vec<_>>(),
    );

    // meta must carry the schema/app version we just wrote.
    let meta_rows = tables
        .get("meta")
        .and_then(|t| t.as_array())
        .expect("dump must include meta table as array");
    let app_version_row = meta_rows
        .iter()
        .find(|row| row.get("key").and_then(|k| k.as_str()) == Some("app_version"));
    assert!(
        app_version_row.is_some(),
        "meta dump missing app_version row: {:?}",
        meta_rows,
    );
}

#[test]
fn saves_export_refuses_missing_save() {
    let dir = tempdir().expect("tempdir");
    let missing = dir.path().join("does_not_exist.db");
    let dump = dir.path().join("nope.json");

    let out = Command::new(nba3k_bin())
        .args([
            "saves",
            "export",
            missing.to_str().unwrap(),
            "--to",
            dump.to_str().unwrap(),
        ])
        .output()
        .expect("run nba3k saves export");
    assert!(
        !out.status.success(),
        "expected non-zero exit for missing save; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no such save"),
        "expected `no such save` in stderr; got:\n{}",
        stderr,
    );
    assert!(
        !dump.exists(),
        "dump file should not be written when source is missing",
    );
}
