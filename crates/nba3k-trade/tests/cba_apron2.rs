//! Apron 2 hard restrictions: no aggregation, no cash sent.

mod cba_common;

use cba_common::*;
use nba3k_core::Cents;
use nba3k_trade::cba::{classify_salary_tier, validate, CbaViolation, SalaryTier};

/// Build a world where TEAM_A sits **above the second apron** ($207.8M+) and
/// TEAM_B is non-apron. Used to assert apron-2 restrictions hit TEAM_A.
fn apron2_world() -> World {
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    // TEAM_A anchor $210M (above apron_2 $207.8M).
    let mut players = vec![
        player_on(101, TEAM_A, 1, 210_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        // TEAM_A two outgoing players (aggregation bait).
        player_on(150, TEAM_A, 1, 5_000_000),
        player_on(151, TEAM_A, 1, 5_000_000),
        // TEAM_B incoming player (matched-ish).
        player_on(250, TEAM_B, 1, 9_000_000),
    ];
    let _ = &mut players;
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    w
}

#[test]
fn apron2_team_classifier_is_apron2() {
    let w = apron2_world();
    let snap = w.snapshot();
    assert_eq!(classify_salary_tier(TEAM_A, &snap), SalaryTier::Apron2);
}

#[test]
fn apron2_aggregation_rejected() {
    let w = apron2_world();
    let snap = w.snapshot();

    // TEAM_A sends two players (aggregation) → must fail.
    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[150, 151]),
        TEAM_B,
        assets_players(&[250]),
    );
    match validate(&offer, &snap) {
        Err(CbaViolation::Apron2Restriction { team }) => assert_eq!(team, TEAM_A),
        other => panic!("expected Apron2Restriction for TEAM_A, got {other:?}"),
    }
}

#[test]
fn apron2_cash_sent_rejected() {
    let w = apron2_world();
    let snap = w.snapshot();

    // TEAM_A sends one player + cash → cash from an apron-2 team is illegal.
    let offer = two_team_offer(
        TEAM_A,
        assets_with_cash(&[150], Cents::from_dollars(100_000)),
        TEAM_B,
        assets_players(&[250]),
    );
    match validate(&offer, &snap) {
        Err(CbaViolation::Apron2Restriction { team }) => assert_eq!(team, TEAM_A),
        other => panic!("expected Apron2Restriction (cash) for TEAM_A, got {other:?}"),
    }
}
