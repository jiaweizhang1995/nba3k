//! Tests for `star_protection` — the untouchable-star model. Verifies the
//! curated `data/star_roster.toml` loads, that franchise-tagged players
//! exceed the absolute_threshold (≥ 0.85), and that the same name on a
//! different team does NOT inherit the tag.

mod common;

use common::{build_named_snapshot, find_player_id, NamedRosterSpec};
use nba3k_core::{GMArchetype, TeamId, TeamRecordSummary};
use nba3k_models::star_protection::{
    load_star_roster, star_protection, StarRoster, STAR_ROSTER_PATH,
};
use nba3k_models::weights::StarProtectionWeights;
use std::path::{Path, PathBuf};

/// Resolve the workspace-root star roster path. `cargo test` runs in the
/// crate dir, so we walk up to find `data/star_roster.toml`.
fn resolve_star_roster_path() -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Walk up to the workspace root.
    let workspace_root = crate_dir
        .ancestors()
        .find(|p| p.join("Cargo.lock").exists())
        .expect("workspace Cargo.lock should exist somewhere above the crate dir");
    workspace_root.join(STAR_ROSTER_PATH)
}

fn weights() -> StarProtectionWeights {
    StarProtectionWeights::default()
}

#[test]
fn star_protection_roster_toml_loads_with_at_least_eight_teams() {
    let path = resolve_star_roster_path();
    let roster = load_star_roster(&path).expect("star_roster.toml should parse");
    assert!(
        roster.team_count() >= 8,
        "expected ≥ 8 teams with franchise tags, got {}",
        roster.team_count()
    );
    for (team, players) in &roster.by_team {
        assert!(
            !players.is_empty(),
            "team {} should have at least one tagged player",
            team
        );
    }
}

#[test]
fn star_protection_franchise_tagged_player_on_correct_team_is_untouchable() {
    let path = resolve_star_roster_path();
    let roster = load_star_roster(&path).expect("star_roster.toml should parse");

    // Luka on LAL — the headline test case from the user spec. He's listed
    // in the roster file. Build a snapshot that places him on LAL.
    let snap = build_named_snapshot(
        TeamId(10),
        "LAL",
        GMArchetype::WinNow,
        NamedRosterSpec {
            members: vec![
                ("Luka Dončić", 92, 26, 96),
                ("LeBron James", 90, 40, 90),
                ("Austin Reaves", 80, 26, 84),
                ("Rui Hachimura", 78, 27, 82),
                ("Dorian Finney-Smith", 75, 31, 78),
                ("Jaxson Hayes", 72, 24, 80),
                ("Gabe Vincent", 70, 28, 75),
                ("Maxi Kleber", 70, 32, 75),
                ("Bronny James", 65, 21, 78),
            ],
        },
        TeamRecordSummary {
            wins: 28,
            losses: 14,
            conf_rank: 3,
            point_diff: 120,
        },
    );
    let pid = find_player_id(&snap, "Luka Dončić");

    let score = star_protection(pid, TeamId(10), &snap.snapshot(), &roster, &weights());
    assert!(
        score.value >= weights().absolute_threshold as f64,
        "Luka on LAL should hit absolute_threshold ({}); got {}",
        weights().absolute_threshold,
        score.value
    );
    let labels: Vec<&'static str> = score.reasons.iter().map(|r| r.label).collect();
    assert!(
        labels.contains(&"franchise_tag"),
        "expected franchise_tag reason, got {:?}",
        labels
    );
}

#[test]
fn star_protection_same_name_on_different_team_does_not_inherit_tag() {
    let path = resolve_star_roster_path();
    let roster = load_star_roster(&path).expect("star_roster.toml should parse");

    // Hypothetical: Luka on UTA. Without the franchise tag, his protection
    // should fall through to OVR/age signals only — well below 0.30 if he's
    // not the team's top OVR (we deliberately give UTA a 95-OVR teammate).
    let snap = build_named_snapshot(
        TeamId(20),
        "UTA",
        GMArchetype::Rebuilder,
        NamedRosterSpec {
            members: vec![
                ("Lauri Markkanen", 88, 28, 89), // top OVR on team
                ("Luka Dončić", 75, 26, 80),     // mid-pack OVR, no franchise tag for UTA
                ("Walker Kessler", 78, 23, 86),
                ("Keyonte George", 74, 21, 84),
                ("Collin Sexton", 76, 26, 80),
                ("John Collins", 76, 27, 79),
                ("Jordan Clarkson", 75, 33, 76),
                ("Taylor Hendricks", 72, 22, 86),
                ("Brice Sensabaugh", 70, 22, 82),
            ],
        },
        TeamRecordSummary {
            wins: 12,
            losses: 28,
            conf_rank: 13,
            point_diff: -150,
        },
    );
    let pid = find_player_id(&snap, "Luka Dončić");

    let score = star_protection(pid, TeamId(20), &snap.snapshot(), &roster, &weights());
    assert!(
        score.value < 0.30,
        "Luka on UTA should fall through to weak signals (< 0.30); got {}",
        score.value
    );
    let labels: Vec<&'static str> = score.reasons.iter().map(|r| r.label).collect();
    assert!(
        !labels.contains(&"franchise_tag"),
        "should NOT have franchise_tag reason, got {:?}",
        labels
    );
}

#[test]
fn star_protection_missing_star_roster_returns_empty() {
    let bogus = Path::new("/tmp/nba3k-models-this-file-does-not-exist.toml");
    let roster = load_star_roster(bogus).expect("missing file should not error");
    assert_eq!(roster.team_count(), 0);
}

#[test]
fn star_protection_untagged_player_with_loaded_empty_roster_has_no_franchise_reason() {
    // Use an empty StarRoster to verify the franchise_tag path is opt-in.
    let roster = StarRoster::default();

    let snap = build_named_snapshot(
        TeamId(30),
        "BOS",
        GMArchetype::WinNow,
        NamedRosterSpec {
            members: vec![
                ("Generic Star", 92, 26, 95),
                ("Sidekick", 84, 26, 86),
                ("Filler 1", 80, 25, 82),
                ("Filler 2", 78, 24, 80),
                ("Filler 3", 76, 25, 78),
                ("Filler 4", 75, 26, 78),
                ("Filler 5", 74, 25, 78),
                ("Filler 6", 72, 25, 76),
                ("Filler 7", 70, 24, 76),
            ],
        },
        TeamRecordSummary {
            wins: 28,
            losses: 14,
            conf_rank: 2,
            point_diff: 150,
        },
    );
    let pid = find_player_id(&snap, "Generic Star");
    let score = star_protection(pid, TeamId(30), &snap.snapshot(), &roster, &weights());

    let labels: Vec<&'static str> = score.reasons.iter().map(|r| r.label).collect();
    assert!(
        !labels.contains(&"franchise_tag"),
        "no franchise_tag without TOML entry"
    );
    // Top-OVR-on-team bump should still appear.
    assert!(labels.contains(&"top_ovr_on_team"));
}
