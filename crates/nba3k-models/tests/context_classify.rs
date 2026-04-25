//! Tests for `team_context` — the M4 replacement for the M3
//! `nba3k_trade::context::classify_team`. Verifies discrete TeamMode
//! classification, continuous score vector, and reason emission.

mod common;

use chrono::NaiveDate;
use common::{build_snapshot, RosterSpec};
use nba3k_core::{GMArchetype, SeasonPhase, TeamId, TeamRecordSummary};
use nba3k_models::team_context::{team_context, TeamMode};
use nba3k_models::weights::TeamContextWeights;

fn weights() -> TeamContextWeights {
    TeamContextWeights::default()
}

#[test]
fn context_contender_with_seven_keepers_and_top_three_seed() {
    // 7 of top-9 rotation OVR ≥ 85, standings rank 2 → Contend, contend_score > 0.7.
    let roster = RosterSpec {
        members: vec![
            (92, 27),
            (89, 26),
            (88, 28),
            (87, 25),
            (86, 24),
            (86, 26),
            (85, 23),
            (78, 27),
            (76, 22),
        ],
    };
    let record = TeamRecordSummary {
        wins: 30,
        losses: 12,
        conf_rank: 2,
        point_diff: 200,
    };
    let owned = build_snapshot(
        TeamId(1),
        "BOS",
        GMArchetype::WinNow,
        roster,
        record,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );

    let ctx = team_context(TeamId(1), &owned.snapshot(), &weights());
    assert_eq!(ctx.mode, TeamMode::Contend, "expected Contend mode");
    assert!(
        ctx.contend_score > 0.7,
        "expected contend_score > 0.7, got {}",
        ctx.contend_score
    );
    assert!(
        ctx.contend_score > ctx.rebuild_score,
        "contend_score should dominate ({} vs {})",
        ctx.contend_score,
        ctx.rebuild_score
    );
    assert!(
        !ctx.reasons.is_empty(),
        "reasons should explain the classification"
    );
    let labels: Vec<&'static str> = ctx.reasons.iter().map(|r| r.label).collect();
    assert!(labels.contains(&"top_ovr_signal"));
    assert!(labels.contains(&"standings_signal"));
}

#[test]
fn context_full_rebuild_young_no_keepers_bottom_of_standings() {
    // Avg rotation age ≤ 23, 0 keepers (no one ≥ 82 OVR), rank 14 → FullRebuild,
    // rebuild_score > 0.7.
    let roster = RosterSpec {
        members: vec![
            (78, 22),
            (76, 21),
            (74, 23),
            (73, 22),
            (72, 21),
            (70, 24),
            (70, 22),
            (68, 21),
            (66, 22),
        ],
    };
    let record = TeamRecordSummary {
        wins: 8,
        losses: 32,
        conf_rank: 14,
        point_diff: -300,
    };
    let owned = build_snapshot(
        TeamId(2),
        "WAS",
        GMArchetype::Rebuilder,
        roster,
        record,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );

    let ctx = team_context(TeamId(2), &owned.snapshot(), &weights());
    assert_eq!(ctx.mode, TeamMode::FullRebuild, "expected FullRebuild");
    assert!(
        ctx.rebuild_score > 0.7,
        "expected rebuild_score > 0.7, got {}",
        ctx.rebuild_score
    );
    assert!(
        ctx.rebuild_score > ctx.contend_score,
        "rebuild_score should dominate ({} vs {})",
        ctx.rebuild_score,
        ctx.contend_score
    );
    let labels: Vec<&'static str> = ctx.reasons.iter().map(|r| r.label).collect();
    assert!(labels.contains(&"roster_age_signal"));
    assert!(labels.contains(&"standings_signal"));
}

#[test]
fn context_win_now_pressure_high_for_aging_contender() {
    // 95-OVR star at age 36, vet supporting cast — classic high win-now urgency.
    let roster = RosterSpec {
        members: vec![
            (95, 36),
            (88, 33),
            (86, 32),
            (84, 31),
            (82, 30),
            (80, 29),
            (78, 28),
            (76, 27),
            (74, 26),
        ],
    };
    let record = TeamRecordSummary {
        wins: 28,
        losses: 14,
        conf_rank: 3,
        point_diff: 150,
    };
    let owned = build_snapshot(
        TeamId(3),
        "PHO",
        GMArchetype::StarHunter,
        roster,
        record,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );

    let ctx = team_context(TeamId(3), &owned.snapshot(), &weights());
    assert_eq!(ctx.mode, TeamMode::Contend);
    assert!(
        ctx.win_now_pressure > 0.7,
        "expected win_now_pressure > 0.7 for old contender, got {}",
        ctx.win_now_pressure
    );
}

#[test]
fn context_empty_roster_does_not_panic() {
    let roster = RosterSpec { members: vec![] };
    let record = TeamRecordSummary::default();
    let owned = build_snapshot(
        TeamId(9),
        "XXX",
        GMArchetype::Conservative,
        roster,
        record,
        SeasonPhase::PreSeason,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );

    let ctx = team_context(TeamId(9), &owned.snapshot(), &weights());
    // Empty roster falls into the "Retool" honest-neither bucket.
    assert!(matches!(ctx.mode, TeamMode::Retool | TeamMode::Tank));
    assert!(ctx.contend_score.is_finite());
    assert!(ctx.rebuild_score.is_finite());
    assert!(ctx.win_now_pressure.is_finite());
}

#[test]
fn context_scores_are_in_unit_interval() {
    let roster = RosterSpec {
        members: vec![
            (88, 26),
            (84, 27),
            (82, 25),
            (80, 24),
            (78, 26),
            (76, 25),
            (74, 24),
            (72, 23),
            (70, 22),
        ],
    };
    let record = TeamRecordSummary {
        wins: 20,
        losses: 22,
        conf_rank: 9,
        point_diff: 0,
    };
    let owned = build_snapshot(
        TeamId(4),
        "ABC",
        GMArchetype::Analytics,
        roster,
        record,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );
    let ctx = team_context(TeamId(4), &owned.snapshot(), &weights());
    for s in [ctx.contend_score, ctx.rebuild_score, ctx.win_now_pressure] {
        assert!(
            (0.0..=1.0).contains(&s),
            "score should be in [0, 1], got {}",
            s
        );
    }
}
