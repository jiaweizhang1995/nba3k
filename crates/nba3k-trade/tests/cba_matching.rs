//! Salary-matching tier tests for the post-2023 CBA.

mod cba_common;

use cba_common::*;
use nba3k_core::Cents;
use nba3k_trade::cba::{
    self, classify_salary_tier, max_incoming_for_tier, validate, CbaViolation, SalaryTier,
};

/// Build a world where TEAM_A and TEAM_B are both in the **non-apron** tier
/// (over the cap, under apron_1) so the 200%+$250K matching rule applies.
///
/// Each team's anchor salary on roster is far above cap (~$160M each) but
/// below apron_1 ($195.9M). The two players being traded are extra slots on
/// each roster, sized by the test.
fn non_apron_world(a_send_dollars: i64, b_send_dollars: i64) -> World {
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let mut players = Vec::new();

    // Anchor salary ~$160M each side.
    players.push(player_on(101, TEAM_A, 1, 160_000_000));
    players.push(player_on(201, TEAM_B, 1, 160_000_000));

    // Players being swapped.
    players.push(player_on(102, TEAM_A, 3, a_send_dollars));
    players.push(player_on(202, TEAM_B, 3, b_send_dollars));

    let mut w = World::new(teams, players);
    // Pad to 14 (legal 13–15 → 14 - 1 + 1 = 14 post-trade).
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    w
}

#[test]
fn classify_non_apron_tier() {
    let w = non_apron_world(5_000_000, 10_000_000);
    let snap = w.snapshot();
    assert_eq!(classify_salary_tier(TEAM_A, &snap), SalaryTier::NonApron);
    assert_eq!(classify_salary_tier(TEAM_B, &snap), SalaryTier::NonApron);
}

#[test]
fn non_apron_match_5m_for_10m_passes() {
    // 5M outgoing → 5M*2 + $250K = $10.25M ceiling → $10M incoming OK.
    let w = non_apron_world(5_000_000, 10_000_000);
    let snap = w.snapshot();
    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[102]),
        TEAM_B,
        assets_players(&[202]),
    );
    let res = validate(&offer, &snap);
    assert!(res.is_ok(), "expected match OK, got {res:?}");
}

#[test]
fn non_apron_match_5m_for_11m_fails() {
    // 5M outgoing → ceiling $10.25M → 11M incoming exceeds.
    let w = non_apron_world(5_000_000, 11_000_000);
    let snap = w.snapshot();
    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[102]),
        TEAM_B,
        assets_players(&[202]),
    );
    let res = validate(&offer, &snap);
    match res {
        Err(CbaViolation::SalaryMatching { team, in_dollars, .. }) => {
            assert_eq!(team, TEAM_A);
            assert_eq!(in_dollars, 11_000_000);
        }
        other => panic!("expected SalaryMatching for TEAM_A, got {other:?}"),
    }
}

#[test]
fn non_apron_ceiling_math_explicit() {
    // Direct check on `max_incoming_for_tier` so the formula is locked in.
    let w = non_apron_world(5_000_000, 5_000_000);
    let snap = w.snapshot();

    // Below the $7.5M break: 200% + $250K.
    let cap_5m =
        max_incoming_for_tier(SalaryTier::NonApron, Cents::from_dollars(5_000_000), TEAM_A, &snap);
    assert_eq!(cap_5m.as_dollars(), 5_000_000 * 2 + 250_000);

    // Above the $7.5M break: 125% + $250K.
    let cap_10m = max_incoming_for_tier(
        SalaryTier::NonApron,
        Cents::from_dollars(10_000_000),
        TEAM_A,
        &snap,
    );
    assert_eq!(cap_10m.as_dollars(), 10_000_000 * 125 / 100 + 250_000);
}

#[test]
fn outgoing_pre_kicker_excludes_kicker() {
    // Sender side never includes a kicker bump even on a player who has one.
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let mut players = Vec::new();
    players.push(player_on(101, TEAM_A, 1, 160_000_000));
    players.push(player_on(201, TEAM_B, 1, 160_000_000));

    // $20M player WITH 15% kicker on TEAM_A.
    let mut p = player_on(150, TEAM_A, 1, 20_000_000);
    p.trade_kicker_pct = Some(15);
    players.push(p);

    // Match-grade incoming for TEAM_A: $30M player on TEAM_B (well under
    // 200%+$250K = $40.25M ceiling).
    players.push(player_on(250, TEAM_B, 1, 30_000_000));

    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot();

    // Sender side reads pre-kicker $20M (no bump).
    let out = cba::outgoing_salary_pre_kicker(
        TEAM_A,
        &two_team_offer(
            TEAM_A,
            assets_players(&[150]),
            TEAM_B,
            assets_players(&[250]),
        ),
        &snap,
    );
    assert_eq!(out.as_dollars(), 20_000_000);
}
