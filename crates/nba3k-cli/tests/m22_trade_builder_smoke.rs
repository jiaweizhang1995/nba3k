//! M22 smoke: the TUI trade builder dispatches to the same command path as
//! `trade propose3`, so this covers the 3-team proposal payload end-to-end.

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
    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args(["--save", save.to_str().unwrap(), "new", "--team", "BOS"])
        .output()
        .expect("nba3k new");
    assert!(
        out.status.success(),
        "new failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn three_team_builder_command_path_records_chain() {
    if !seed_present() {
        eprintln!("seed missing - skipping");
        return;
    }

    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m22_trade_builder.db");
    bootstrap(&save);

    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "trade",
            "propose3",
            "--leg",
            "BOS:Jayson Tatum",
            "--leg",
            "LAL:LeBron James",
            "--leg",
            "DAL:Kyrie Irving",
            "--json",
        ])
        .output()
        .expect("nba3k trade propose3");
    assert!(
        out.status.success(),
        "trade propose3 failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("trade propose3 --json must parse");
    assert_eq!(v["id"].as_u64(), Some(1));
    assert_eq!(v["round"].as_u64(), Some(1));
    assert_eq!(v["teams"].as_str(), Some("BOS/LAL/DAL"));
    assert!(
        matches!(v["status"].as_str(), Some("accepted" | "rejected")),
        "unexpected status: {}",
        v
    );
}
