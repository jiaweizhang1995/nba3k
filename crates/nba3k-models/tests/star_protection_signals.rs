//! Component-level tests for `star_protection`: the top_ovr bump on a
//! Contend-mode team, the young_ascending bump, and the FullRebuild
//! attenuation. These tests exercise the path WITHOUT relying on the
//! curated TOML so they're deterministic regardless of edits to the
//! franchise-tag list.

mod common;

use common::{build_named_snapshot, find_player_id, NamedRosterSpec};
use nba3k_core::{GMArchetype, TeamId, TeamRecordSummary};
use nba3k_models::star_protection::{star_protection, StarRoster};
use nba3k_models::weights::StarProtectionWeights;

fn weights() -> StarProtectionWeights {
    StarProtectionWeights::default()
}

#[test]
fn star_protection_top_ovr_player_on_contender_gets_bump() {
    // Roster shape pushes team into Contend mode (high top OVR, strong seed).
    let snap = build_named_snapshot(
        TeamId(40),
        "FAKE",
        GMArchetype::WinNow,
        NamedRosterSpec {
            members: vec![
                ("Alpha", 92, 27, 94),
                ("Beta", 87, 26, 88),
                ("Gamma", 85, 28, 86),
                ("Delta", 83, 25, 84),
                ("Epsilon", 81, 26, 82),
                ("Zeta", 80, 25, 81),
                ("Eta", 78, 24, 80),
                ("Theta", 76, 25, 78),
                ("Iota", 74, 24, 76),
            ],
        },
        TeamRecordSummary {
            wins: 30,
            losses: 12,
            conf_rank: 2,
            point_diff: 200,
        },
    );

    let alpha_id = find_player_id(&snap, "Alpha");
    let score = star_protection(
        alpha_id,
        TeamId(40),
        &snap.snapshot(),
        &StarRoster::default(),
        &weights(),
    );

    let labels: Vec<&'static str> = score.reasons.iter().map(|r| r.label).collect();
    assert!(
        labels.contains(&"top_ovr_on_team"),
        "expected top_ovr_on_team reason, got {:?}",
        labels
    );
    let top_ovr_delta = score
        .reasons
        .iter()
        .find(|r| r.label == "top_ovr_on_team")
        .map(|r| r.delta)
        .unwrap_or(0.0);
    assert!(
        top_ovr_delta > weights().top_ovr_bump as f64,
        "Contend mode should amplify top_ovr_bump above its base ({}); got {}",
        weights().top_ovr_bump,
        top_ovr_delta,
    );
}

#[test]
fn star_protection_young_ascending_prospect_gets_bump() {
    // 22-year-old, potential 94, on a rebuild team. Should pick up the
    // young_ascending reason.
    let snap = build_named_snapshot(
        TeamId(50),
        "FAKE2",
        GMArchetype::Rebuilder,
        NamedRosterSpec {
            members: vec![
                ("Prospect", 80, 22, 94),
                ("Vet", 78, 30, 79),
                ("Filler1", 75, 25, 78),
                ("Filler2", 73, 24, 77),
                ("Filler3", 72, 23, 76),
                ("Filler4", 70, 22, 75),
                ("Filler5", 68, 22, 75),
                ("Filler6", 66, 21, 74),
                ("Filler7", 65, 22, 73),
            ],
        },
        TeamRecordSummary {
            wins: 14,
            losses: 26,
            conf_rank: 12,
            point_diff: -100,
        },
    );

    let pid = find_player_id(&snap, "Prospect");
    let score = star_protection(
        pid,
        TeamId(50),
        &snap.snapshot(),
        &StarRoster::default(),
        &weights(),
    );

    let labels: Vec<&'static str> = score.reasons.iter().map(|r| r.label).collect();
    assert!(
        labels.contains(&"young_ascending"),
        "expected young_ascending reason, got {:?}",
        labels
    );
}

#[test]
fn star_protection_full_rebuild_attenuates_protection() {
    // Same alpha-shaped roster as the contender test, but record + age make
    // it a FullRebuild — top_ovr bump should be smaller, the
    // team_mode_full_rebuild reason should appear.
    let snap = build_named_snapshot(
        TeamId(60),
        "FAKE3",
        GMArchetype::Rebuilder,
        NamedRosterSpec {
            members: vec![
                ("Alpha", 80, 22, 88),
                ("Beta", 75, 21, 84),
                ("Gamma", 73, 23, 82),
                ("Delta", 72, 22, 80),
                ("Epsilon", 70, 21, 79),
                ("Zeta", 68, 24, 78),
                ("Eta", 66, 22, 77),
                ("Theta", 65, 21, 76),
                ("Iota", 64, 22, 75),
            ],
        },
        TeamRecordSummary {
            wins: 8,
            losses: 32,
            conf_rank: 14,
            point_diff: -300,
        },
    );

    let alpha_id = find_player_id(&snap, "Alpha");
    let score = star_protection(
        alpha_id,
        TeamId(60),
        &snap.snapshot(),
        &StarRoster::default(),
        &weights(),
    );

    let labels: Vec<&'static str> = score.reasons.iter().map(|r| r.label).collect();
    assert!(
        labels.contains(&"team_mode_full_rebuild"),
        "FullRebuild should emit attenuation reason, got {:?}",
        labels
    );
    // Without a franchise tag, FullRebuild's top_ovr should never cross the
    // absolute_threshold — that's the whole point of clearout mode.
    assert!(
        score.value < weights().absolute_threshold as f64,
        "FullRebuild's top OVR should not be untouchable; got {}",
        score.value
    );
}

#[test]
fn star_protection_score_is_clamped_to_unit_interval() {
    let snap = build_named_snapshot(
        TeamId(70),
        "FAKE4",
        GMArchetype::WinNow,
        NamedRosterSpec {
            members: vec![
                ("Alpha", 95, 23, 99),
                ("Beta", 88, 26, 90),
                ("Gamma", 86, 27, 88),
                ("Delta", 84, 25, 86),
                ("Epsilon", 82, 26, 84),
                ("Zeta", 80, 25, 82),
                ("Eta", 78, 24, 80),
                ("Theta", 76, 25, 78),
                ("Iota", 74, 24, 76),
            ],
        },
        TeamRecordSummary {
            wins: 30,
            losses: 12,
            conf_rank: 1,
            point_diff: 250,
        },
    );

    // Synthetic StarRoster: tag Alpha for FAKE4. Combined with top_ovr bump
    // and young_ascending, the raw deltas would push value > 1.0 — verify it
    // still clamps to [0, 1].
    let mut roster = StarRoster::default();
    roster
        .by_team
        .insert("FAKE4".to_string(), vec!["Alpha".to_string()]);

    let pid = find_player_id(&snap, "Alpha");
    let score = star_protection(pid, TeamId(70), &snap.snapshot(), &roster, &weights());
    assert!(
        score.value >= 0.0 && score.value <= 1.0,
        "score should be clamped to [0, 1], got {}",
        score.value
    );
}
