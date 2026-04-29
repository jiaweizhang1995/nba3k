//! M7 integration test: drive the binary through one full season + part of
//! season 2 via the scripted loop in `tests/scripts/season1.txt`. Verifies
//! that the multi-season loop, schedule regen, and trade-accept fixes
//! survive a real end-to-end run.
//!
//! Skipped automatically when the seed DB is missing (CI without scrape).

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn binary() -> PathBuf {
    let mut p = workspace_root();
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("nba3k");
    p
}

#[test]
#[ignore = "requires release-build binary + seed; run with `cargo test --release -- --ignored`"]
fn full_season_scripted() {
    let bin = binary();
    if !bin.exists() {
        panic!(
            "binary not found at {}. Run `cargo build --release -p nba3k-cli` first.",
            bin.display()
        );
    }
    let seed = workspace_root().join("data/seed_2025_26.sqlite");
    if !seed.exists() {
        eprintln!("seed missing at {}, skipping", seed.display());
        return;
    }
    let save = std::env::temp_dir().join("nba3k-test-season1.db");
    let _ = std::fs::remove_file(&save);

    // 1) `new` to bootstrap the save. Run with cwd = workspace root so
    //    the binary's relative `data/seed_2025_26.sqlite` resolves.
    let root = workspace_root();
    // M34 — `new` defaults to live ESPN. Pin to `--offline` so the integ
    // test does not depend on internet or current real-world season state.
    let new_status = Command::new(&bin)
        .current_dir(&root)
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

    // 2) Drive the script.
    let script = root.join("tests/scripts/season1.txt");
    let out = Command::new(&bin)
        .current_dir(&root)
        .args([
            "--save",
            save.to_str().unwrap(),
            "--script",
            script.to_str().unwrap(),
        ])
        .output()
        .expect("nba3k --script");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "scripted run failed:\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );

    // 3) Spot-check expected substrings: champion crowned, season advanced.
    assert!(
        stdout.contains("Champion:") || stdout.contains("\"champion\":"),
        "no champion in script output:\n{}",
        stdout
    );
    assert!(
        stdout.contains("advanced to season 2027"),
        "season-advance did not roll forward:\n{}",
        stdout
    );
    // M12-C: AI FA market runs as part of season-advance and prints a
    // "<N> FAs signed" segment in the summary line. Loose match on the
    // suffix — the count varies with seed-DB FA pool size.
    assert!(
        stdout.contains("FAs signed"),
        "season-advance did not log FA signings:\n{}",
        stdout
    );

    // 4) Sanity status check after script: should be in season 2027 with
    //    a fresh schedule (1230 unplayed before sim-day 30, ~1100 after).
    let status_out = Command::new(&bin)
        .args(["--save", save.to_str().unwrap(), "status", "--json"])
        .output()
        .expect("status --json");
    assert!(
        status_out.status.success(),
        "status failed after script run"
    );
    let s = String::from_utf8_lossy(&status_out.stdout);
    assert!(s.contains("2027"), "status should be in 2027:\n{}", s);
}
