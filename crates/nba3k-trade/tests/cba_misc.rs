//! NTC, cash limits, roster size.

mod cba_common;

use cba_common::*;
use nba3k_core::{Cents, DraftPickId, PlayerId, SeasonId, SeasonPhase};
use nba3k_trade::cba::{
    check_season_start_rosters, check_season_start_user_roster, validate, CbaViolation,
    REGULAR_SEASON_ROSTER_MAX,
};

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
fn seven_year_rule_blocks_year_8() {
    let mut w = baseline_world();
    pad_picks(&mut w, &[TEAM_A, TEAM_B], SEASON, 8);
    let pick = add_pick(&mut w, SeasonId(SEASON.0 + 8), 1, TEAM_A, TEAM_A);
    let snap = w.snapshot();
    let offer = two_team_offer(
        TEAM_A,
        assets_picks(&[pick]),
        TEAM_B,
        nba3k_core::TradeAssets::default(),
    );
    match validate(&offer, &snap) {
        Err(CbaViolation::PickTooFarOut { team, year }) => {
            assert_eq!(team, TEAM_A);
            assert_eq!(year, SEASON.0 + 8);
        }
        other => panic!("expected PickTooFarOut, got {other:?}"),
    }
}

#[test]
fn seven_year_rule_allows_year_7() {
    let mut w = baseline_world();
    pad_picks(&mut w, &[TEAM_A, TEAM_B], SEASON, 8);
    let pick = add_pick(&mut w, SeasonId(SEASON.0 + 7), 1, TEAM_A, TEAM_A);
    let snap = w.snapshot();
    let offer = two_team_offer(
        TEAM_A,
        assets_picks(&[pick]),
        TEAM_B,
        nba3k_core::TradeAssets::default(),
    );
    assert!(validate(&offer, &snap).is_ok());
}

#[test]
fn stepien_blocks_consecutive_first_loss() {
    let mut w = baseline_world();
    pad_picks(&mut w, &[TEAM_A, TEAM_B], SEASON, 7);
    let missing_2026 = DraftPickId((SEASON.0 as u32) * 1000 + 100 + TEAM_A.0 as u32);
    w.picks.get_mut(&missing_2026).unwrap().current_owner = TEAM_B;
    let trade_2027 = DraftPickId(((SEASON.0 + 1) as u32) * 1000 + 100 + TEAM_A.0 as u32);
    let snap = w.snapshot();
    let offer = two_team_offer(
        TEAM_A,
        assets_picks(&[trade_2027]),
        TEAM_B,
        nba3k_core::TradeAssets::default(),
    );
    match validate(&offer, &snap) {
        Err(CbaViolation::StepienViolation { team, year1, year2 }) => {
            assert_eq!(team, TEAM_A);
            assert_eq!((year1, year2), (SEASON.0, SEASON.0 + 1));
        }
        other => panic!("expected StepienViolation, got {other:?}"),
    }
}

#[test]
fn stepien_allows_non_consecutive() {
    let mut w = baseline_world();
    pad_picks(&mut w, &[TEAM_A, TEAM_B], SEASON, 7);
    let missing_2026 = DraftPickId((SEASON.0 as u32) * 1000 + 100 + TEAM_A.0 as u32);
    w.picks.get_mut(&missing_2026).unwrap().current_owner = TEAM_B;
    let trade_2028 = DraftPickId(((SEASON.0 + 2) as u32) * 1000 + 100 + TEAM_A.0 as u32);
    let snap = w.snapshot();
    let offer = two_team_offer(
        TEAM_A,
        assets_picks(&[trade_2028]),
        TEAM_B,
        nba3k_core::TradeAssets::default(),
    );
    assert!(validate(&offer, &snap).is_ok());
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
    // Regular-season bound remains 13..=18 (15 standard + 3 two-way per
    // 2025-26 CBA in this v1 roster model).
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
fn roster_offseason_allows_up_to_21_passes() {
    use nba3k_core::TradeAssets;
    use nba3k_trade::cba::check_roster_size;

    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        player_on(250, TEAM_B, 1, 5_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 20, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot_with_phase(SeasonPhase::OffSeason);
    let offer = two_team_offer(
        TEAM_A,
        TradeAssets::default(),
        TEAM_B,
        assets_players(&[250]),
    );

    assert!(check_roster_size(TEAM_A, &offer, &snap).is_ok());
}

#[test]
fn roster_offseason_22_rejects() {
    use nba3k_core::TradeAssets;
    use nba3k_trade::cba::check_roster_size;

    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        player_on(250, TEAM_B, 1, 5_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 21, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot_with_phase(SeasonPhase::OffSeason);
    let offer = two_team_offer(
        TEAM_A,
        TradeAssets::default(),
        TEAM_B,
        assets_players(&[250]),
    );

    match check_roster_size(TEAM_A, &offer, &snap) {
        Err(CbaViolation::RosterSize { team, size }) => {
            assert_eq!(team, TEAM_A);
            assert_eq!(size, 22);
        }
        other => panic!("expected RosterSize for TEAM_A=22, got {other:?}"),
    }
}

#[test]
fn roster_preseason_uses_offseason_bounds() {
    use nba3k_core::TradeAssets;
    use nba3k_trade::cba::check_roster_size;

    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        player_on(250, TEAM_B, 1, 5_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 20, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot_with_phase(SeasonPhase::PreSeason);
    let offer = two_team_offer(
        TEAM_A,
        TradeAssets::default(),
        TEAM_B,
        assets_players(&[250]),
    );

    assert!(check_roster_size(TEAM_A, &offer, &snap).is_ok());
}

#[test]
fn roster_regular_ceiling_still_18() {
    use nba3k_core::TradeAssets;
    use nba3k_trade::cba::check_roster_size;

    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        player_on(250, TEAM_B, 1, 5_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 18, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot_with_phase(SeasonPhase::Regular);
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

#[test]
fn season_start_with_15_passes() {
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, REGULAR_SEASON_ROSTER_MAX as usize, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot_with_phase(SeasonPhase::PreSeason);

    assert!(check_season_start_rosters(&snap).is_empty());
}

#[test]
fn season_start_with_16_flags_team() {
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 16, 1_000);
    pad_roster(&mut w, TEAM_B, 15, 2_000);
    let snap = w.snapshot_with_phase(SeasonPhase::PreSeason);

    let violations = check_season_start_rosters(&snap);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].team, TEAM_A);
    assert_eq!(violations[0].size, 16);
    assert_eq!(violations[0].limit, REGULAR_SEASON_ROSTER_MAX);
}

#[test]
fn season_start_user_only_15_passes_even_when_ai_over() {
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, REGULAR_SEASON_ROSTER_MAX as usize, 1_000);
    pad_roster(&mut w, TEAM_B, 16, 2_000);
    let snap = w.snapshot_with_phase(SeasonPhase::PreSeason);

    assert_eq!(check_season_start_user_roster(&snap, TEAM_A), None);

    let violations = check_season_start_rosters(&snap);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].team, TEAM_B);
    assert_eq!(violations[0].size, 16);
    assert_eq!(violations[0].limit, REGULAR_SEASON_ROSTER_MAX);
}

#[test]
fn season_start_user_only_16_blocks_user() {
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 16, 1_000);
    pad_roster(&mut w, TEAM_B, REGULAR_SEASON_ROSTER_MAX as usize, 2_000);
    let snap = w.snapshot_with_phase(SeasonPhase::PreSeason);

    let violation = check_season_start_user_roster(&snap, TEAM_A)
        .expect("user team over 15 should block season start");
    assert_eq!(violation.team, TEAM_A);
    assert_eq!(violation.size, 16);
    assert_eq!(violation.limit, REGULAR_SEASON_ROSTER_MAX);
}
