//! Acceptance tests for `nba3k_models::contract_gen::generate_contract`.
//!
//! Drives the OVR → tier mapping and the age → length adjustment from the
//! M11-A charter. Salary numbers are intentionally loose (`>=`/`<=` instead
//! of `==`) so the model can be retuned without churning tests — the
//! invariants we care about are tier ordering, not exact dollars.

use nba3k_core::{Cents, Player, PlayerId, PlayerRole, Position, Ratings, SeasonId};
use nba3k_models::contract_gen::generate_contract;

fn player(ovr: u8, age: u8) -> Player {
    Player {
        id: PlayerId(1),
        name: "Test Player".into(),
        primary_position: Position::SF,
        secondary_position: None,
        age,
        overall: ovr,
        potential: ovr,
        ratings: Ratings::default(),
        contract: None,
        team: None,
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::default(),
        morale: 0.5,
    }
}

#[test]
fn ovr_95_gets_max_money_and_length() {
    let c = generate_contract(&player(95, 27), SeasonId(2026));
    let yr1 = c.years[0];
    assert!(
        yr1.salary >= Cents(40_000_000_00),
        "OVR 95 should land at >= $40M/yr, got {}",
        yr1.salary
    );
    assert!(
        c.years.len() >= 3,
        "OVR 95 should land at >= 3 years, got {}",
        c.years.len()
    );
}

#[test]
fn ovr_65_gets_veteran_min() {
    let c = generate_contract(&player(65, 33), SeasonId(2026));
    let yr1 = c.years[0];
    assert!(
        yr1.salary <= Cents(3_000_000_00),
        "OVR 65 should land near veteran-min (~$2-3M), got {}",
        yr1.salary
    );
    assert!(
        !c.years.is_empty(),
        "every contract must have at least 1 year"
    );
}

#[test]
fn younger_player_gets_longer_contract_at_same_ovr() {
    let young = generate_contract(&player(82, 24), SeasonId(2026));
    let old = generate_contract(&player(82, 34), SeasonId(2026));
    assert!(
        young.years.len() > old.years.len(),
        "24yo OVR 82 should outlast 34yo OVR 82 ({} vs {} years)",
        young.years.len(),
        old.years.len()
    );
}

#[test]
fn first_year_aligns_with_signing_season() {
    let c = generate_contract(&player(80, 28), SeasonId(2026));
    assert_eq!(c.years[0].season, SeasonId(2026));
    assert_eq!(c.signed_in_season, SeasonId(2026));
    // Subsequent years are sequential.
    for w in c.years.windows(2) {
        assert_eq!(w[1].season.0, w[0].season.0 + 1);
    }
}
