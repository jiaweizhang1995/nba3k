//! Contract extension acceptance logic.
//!
//! User proposes an extension; the model evaluates fairness vs market rate
//! (from `contract_gen::generate_contract`) and adjusts for player morale.
//!
//! Decision tiers (after morale adjustment to `effective_min`):
//! - `offered_salary >= effective_min` AND `years` in [3, 5] → Accept.
//! - `offered_salary >= 0.85 * effective_min` → Counter at `effective_min` for 4 years.
//! - Else → Reject ("below market").
//!
//! Morale adjustment:
//! - `morale > 0.7`: happy player accepts a 10% discount (`effective_min = 0.90 * market`).
//! - `morale < 0.3`: salty player demands a 10% premium (`effective_min = 1.10 * market`).
//! - Otherwise: `effective_min = 0.95 * market` (default near-market threshold).
//!
//! Inputs: pure function over the player + offer; no I/O. Salary is in
//! `Cents` (integer math) but ratio comparisons promote to `f64`.

use nba3k_core::{Cents, Player, SeasonId};

use crate::contract_gen::generate_contract;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionDecision {
    Accept,
    Counter { request_salary_cents: i64, request_years: u8 },
    Reject(String),
}

const MIN_REASONABLE_YEARS: u8 = 3;
const MAX_REASONABLE_YEARS: u8 = 5;
const COUNTER_FLOOR_FRAC: f64 = 0.85;
const HAPPY_DISCOUNT_FRAC: f64 = 0.90;
const NEUTRAL_FRAC: f64 = 0.95;
const SALTY_PREMIUM_FRAC: f64 = 1.10;
const HAPPY_MORALE: f32 = 0.7;
const SALTY_MORALE: f32 = 0.3;
const COUNTER_REQUEST_YEARS: u8 = 4;

pub fn accept_extension(
    player: &Player,
    offered_salary_cents: i64,
    offered_years: u8,
    season: SeasonId,
) -> ExtensionDecision {
    let market_cents = generate_contract(player, season).years[0].salary.0 as f64;

    let effective_frac = if player.morale > HAPPY_MORALE {
        HAPPY_DISCOUNT_FRAC
    } else if player.morale < SALTY_MORALE {
        SALTY_PREMIUM_FRAC
    } else {
        NEUTRAL_FRAC
    };
    let effective_min = market_cents * effective_frac;

    let offered = offered_salary_cents as f64;

    if offered >= effective_min
        && offered_years >= MIN_REASONABLE_YEARS
        && offered_years <= MAX_REASONABLE_YEARS
    {
        return ExtensionDecision::Accept;
    }

    if offered >= COUNTER_FLOOR_FRAC * effective_min {
        let request_salary_cents = effective_min.round() as i64;
        return ExtensionDecision::Counter {
            request_salary_cents,
            request_years: COUNTER_REQUEST_YEARS,
        };
    }

    ExtensionDecision::Reject("below market".into())
}

/// Convenience: `accept_extension` taking `Cents` directly.
pub fn accept_extension_cents(
    player: &Player,
    offered_salary: Cents,
    offered_years: u8,
    season: SeasonId,
) -> ExtensionDecision {
    accept_extension(player, offered_salary.0, offered_years, season)
}
