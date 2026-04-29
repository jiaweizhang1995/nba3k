//! NTC, cash limits, roster size.

mod cba_common;

use cba_common::*;
use nba3k_core::{Cents, PlayerId};
use nba3k_trade::cba::{validate, CbaViolation};

/// Non-apron baseline world with a $5M↔$5M swap that's well under any tier.
fn baseline_world() -> World {
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        player_on(150, TEAM_A, 1, 5_000_000),
        player_on(250, TEAM_B, 1, 5_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    w
}

#[test]
fn ntc_hard_rejects() {
    let mut w = baseline_world();
    // Mark player 150 as holding a no-trade clause.
    w.players.get_mut(&PlayerId(150)).unwrap().no_trade_clause = true;
    let snap = w.snapshot();

    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[150]),
        TEAM_B,
        assets_players(&[250]),
    );
    match validate(&offer, &snap) {
        Err(CbaViolation::NoTradeClause(pid)) => assert_eq!(pid, PlayerId(150)),
        other => panic!("expected NoTradeClause(150), got {other:?}"),
    }
}

#[test]
fn cash_limit_per_team_per_season() {
    // 2025-26 limit = $7,964,000. Sending $8M out should trip
    // CashLimitExceeded for that team.
    let w = baseline_world();
    let snap = w.snapshot();
    let offer = two_team_offer(
        TEAM_A,
        assets_with_cash(&[150], Cents::from_dollars(8_000_000)),
        TEAM_B,
        assets_players(&[250]),
    );
    match validate(&offer, &snap) {
        Err(CbaViolation::CashLimitExceeded {
            team,
            amount_dollars,
        }) => {
            assert_eq!(team, TEAM_A);
            assert_eq!(amount_dollars, 8_000_000);
        }
        other => panic!("expected CashLimitExceeded for TEAM_A, got {other:?}"),
    }
}

#[test]
fn cash_at_limit_passes_check_isolated() {
    // Direct check on `check_cash_limits` — at-limit cash must NOT trip the
    // cash check. (Salary matching may still reject the full offer because
    // cash received counts toward the receiver's incoming total — that is
    // by design and tested in cba_matching.rs.)
    use nba3k_trade::cba::check_cash_limits;
    let w = baseline_world();
    let snap = w.snapshot();
    let limit = snap.league_year.max_trade_cash;
    let offer = two_team_offer(
        TEAM_A,
        assets_with_cash(&[150], limit),
        TEAM_B,
        assets_players(&[250]),
    );
    let res = check_cash_limits(&offer, &snap);
    assert!(
        res.is_ok(),
        "expected at-limit cash check to pass, got {res:?}"
    );
}

#[test]
fn roster_too_small_rejected() {
    // Isolated check on `check_roster_size`: TEAM_A has 12, sends 1, takes
    // 0 → 11 post-trade (<13).
    use nba3k_trade::cba::check_roster_size;
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        player_on(150, TEAM_A, 1, 5_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 12, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot();

    use nba3k_core::TradeAssets;
    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[150]),
        TEAM_B,
        TradeAssets::default(),
    );
    match check_roster_size(TEAM_A, &offer, &snap) {
        Err(CbaViolation::RosterSize { team, size }) => {
            assert_eq!(team, TEAM_A);
            assert_eq!(size, 11);
        }
        other => panic!("expected RosterSize for TEAM_A=11, got {other:?}"),
    }
}

#[test]
fn roster_too_large_rejected() {
    // Isolated: TEAM_A has 18, receives 1 with 0 outgoing → 19 (>18).
    // Bound is 13..=18 (15 standard + 3 two-way per 2025-26 CBA).
    use nba3k_trade::cba::check_roster_size;
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        player_on(250, TEAM_B, 1, 5_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 18, 1_000);
    pad_roster(&mut w, TEAM_B, 17, 2_000);
    let snap = w.snapshot();

    use nba3k_core::TradeAssets;
    let offer = two_team_offer(
        TEAM_A,
        TradeAssets::default(),
        TEAM_B,
        assets_players(&[250]),
    );
    match check_roster_size(TEAM_A, &offer, &snap) {
        Err(CbaViolation::RosterSize { team, size }) => {
            assert_eq!(team, TEAM_A);
            assert_eq!(size, 19);
        }
        other => panic!("expected RosterSize for TEAM_A=19, got {other:?}"),
    }
}

#[test]
fn roster_in_bounds_passes() {
    // Sanity: a balanced 1-for-1 trade with a 14-player roster on each side
    // remains 14 post-trade and passes the roster-size check.
    use nba3k_trade::cba::check_roster_size;
    let w = baseline_world();
    let snap = w.snapshot();
    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[150]),
        TEAM_B,
        assets_players(&[250]),
    );
    assert!(check_roster_size(TEAM_A, &offer, &snap).is_ok());
    assert!(check_roster_size(TEAM_B, &offer, &snap).is_ok());
}
