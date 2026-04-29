//! Acceptance tests for `nba3k_models::contract_extension::accept_extension`.
//!
//! Market rate comes from `contract_gen::generate_contract`. OVR 85 lands at
//! `$30M` per the tier table — used as the anchor for offer-fraction tests.

use nba3k_core::{Cents, Player, PlayerId, PlayerRole, Position, Ratings, SeasonId};
use nba3k_models::contract_extension::{accept_extension, ExtensionDecision};

fn player_with_morale(ovr: u8, age: u8, morale: f32) -> Player {
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
        morale,
    }
}

const SEASON: SeasonId = SeasonId(2026);

#[test]
fn above_market_offer_accepted() {
    // OVR 85 market = $30M. Neutral morale → effective_min = $28.5M.
    // Offer $35M / 4yr is comfortably above; should Accept.
    let p = player_with_morale(85, 27, 0.5);
    let d = accept_extension(&p, Cents(35_000_000_00).0, 4, SEASON);
    assert_eq!(d, ExtensionDecision::Accept);
}

#[test]
fn below_85pct_of_market_rejected() {
    // OVR 85 market = $30M. Neutral effective_min = $28.5M.
    // 85% of $28.5M ≈ $24.225M. Offer $20M is well below → Reject.
    let p = player_with_morale(85, 27, 0.5);
    let d = accept_extension(&p, Cents(20_000_000_00).0, 4, SEASON);
    match d {
        ExtensionDecision::Reject(reason) => {
            assert!(
                reason.to_lowercase().contains("below market"),
                "expected below-market reason, got {:?}",
                reason
            );
        }
        other => panic!("expected Reject, got {:?}", other),
    }
}

#[test]
fn near_90pct_market_returns_counter_with_bumped_request() {
    // Neutral effective_min = $30M * 0.95 = $28.5M.
    // Offer $27M (90% of market = ~94.7% of effective_min) → Counter.
    // Counter request must be the effective_min and request_years = 4.
    let p = player_with_morale(85, 27, 0.5);
    let offered = Cents(27_000_000_00).0;
    let d = accept_extension(&p, offered, 4, SEASON);
    match d {
        ExtensionDecision::Counter {
            request_salary_cents,
            request_years,
        } => {
            assert!(
                request_salary_cents > offered,
                "counter ({}) should bump above offer ({})",
                request_salary_cents,
                offered
            );
            assert_eq!(request_years, 4);
            // Sanity: counter target sits at neutral effective_min ≈ $28.5M.
            assert!(request_salary_cents >= Cents(28_000_000_00).0);
            assert!(request_salary_cents <= Cents(29_000_000_00).0);
        }
        other => panic!("expected Counter, got {:?}", other),
    }
}

#[test]
fn happy_player_accepts_92pct_market_via_morale_discount() {
    // morale = 0.9 → effective_min = $30M * 0.90 = $27M.
    // Offer = 92% of market = $27.6M ≥ $27M → Accept.
    let p = player_with_morale(85, 27, 0.9);
    let offered = Cents(27_600_000_00).0;
    let d = accept_extension(&p, offered, 4, SEASON);
    assert_eq!(
        d,
        ExtensionDecision::Accept,
        "happy player at 92% market should accept via morale discount"
    );
}

#[test]
fn out_of_range_years_blocks_accept_routes_to_counter() {
    // Above effective_min in salary but only 2 years (< 3) → not Accept.
    // 2 years still passes the 85%-of-effective-min floor, so Counter.
    let p = player_with_morale(85, 27, 0.5);
    let d = accept_extension(&p, Cents(35_000_000_00).0, 2, SEASON);
    match d {
        ExtensionDecision::Counter { request_years, .. } => {
            assert_eq!(request_years, 4);
        }
        other => panic!("expected Counter on short years, got {:?}", other),
    }
}
