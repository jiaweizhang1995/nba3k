//! M16-C smoke test: `nba3k compare BOS LAL` renders side-by-side text and
//! emits valid JSON; same-team comparison errors out.
//!
//! Requires the seed DB; skipped cleanly when missing.

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
fn compare_bos_vs_lal_text_renders() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m16c_compare_text.db");
    bootstrap(&save);

    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "compare",
            "BOS",
            "LAL",
        ])
        .output()
        .expect("run nba3k compare");
    assert!(
        out.status.success(),
        "compare exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Both team headers visible.
    assert!(stdout.contains("BOS"), "BOS column missing:\n{}", stdout);
    assert!(stdout.contains("LAL"), "LAL column missing:\n{}", stdout);
    // Every required label rendered.
    for label in &["roster size", "top-8 OVR (avg)", "payroll", "chemistry", "TOP 8"] {
        assert!(
            stdout.contains(label),
            "label `{}` missing from compare output:\n{}",
            label,
            stdout
        );
    }
}

#[test]
fn compare_bos_vs_lal_json_parses() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m16c_compare_json.db");
    bootstrap(&save);

    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "compare",
            "BOS",
            "LAL",
            "--json",
        ])
        .output()
        .expect("run nba3k compare --json");
    assert!(
        out.status.success(),
        "compare --json exited non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("compare --json must emit valid JSON");

    // Top-level shape.
    let team_a = &v["team_a"];
    let team_b = &v["team_b"];
    let deltas = &v["deltas"];
    assert_eq!(team_a["team"], "BOS", "team_a abbrev: {}", team_a);
    assert_eq!(team_b["team"], "LAL", "team_b abbrev: {}", team_b);

    // Each team has the required fields populated.
    for (label, t) in [("team_a", team_a), ("team_b", team_b)] {
        assert!(
            t["roster_size"].as_u64().is_some(),
            "{} roster_size missing/invalid: {}",
            label,
            t
        );
        assert!(
            t["top8_avg_overall"].as_f64().is_some(),
            "{} top8_avg_overall missing: {}",
            label,
            t
        );
        assert!(
            t["payroll_cents"].as_i64().is_some(),
            "{} payroll_cents missing: {}",
            label,
            t
        );
        assert!(
            t["chemistry"].as_f64().is_some(),
            "{} chemistry missing: {}",
            label,
            t
        );
        let top8 = t["top8"].as_array().expect("top8 array");
        assert!(!top8.is_empty(), "{} top8 empty: {}", label, t);
        assert!(top8.len() <= 8, "{} top8 has >8 entries: {}", label, t);
        let row = &top8[0];
        assert!(row["name"].is_string(), "top8 row missing name: {}", row);
        assert!(row["overall"].as_u64().is_some(), "top8 row missing overall: {}", row);
        assert!(row["position"].is_string(), "top8 row missing position: {}", row);
    }

    // Deltas object present.
    assert!(deltas["payroll_dollars"].as_i64().is_some(), "deltas.payroll_dollars: {}", deltas);
    assert!(deltas["top8_avg_overall"].as_f64().is_some(), "deltas.top8_avg_overall: {}", deltas);
    assert!(deltas["chemistry"].as_f64().is_some(), "deltas.chemistry: {}", deltas);
}

#[test]
fn compare_same_team_errors() {
    if !seed_present() {
        eprintln!("seed missing — skipping");
        return;
    }
    let dir = tempdir().expect("tempdir");
    let save = dir.path().join("m16c_compare_self.db");
    bootstrap(&save);

    let out = Command::new(nba3k_bin())
        .current_dir(workspace_root())
        .args([
            "--save",
            save.to_str().unwrap(),
            "compare",
            "BOS",
            "BOS",
        ])
        .output()
        .expect("run nba3k compare BOS BOS");
    assert!(
        !out.status.success(),
        "compare BOS BOS unexpectedly succeeded:\nstdout:\n{}",
        String::from_utf8_lossy(&out.stdout),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot compare a team to itself"),
        "expected self-compare error message; got stderr:\n{}",
        stderr
    );
}
