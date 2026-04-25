//! Worker A — contract value model.
//!
//! Surplus of a contract relative to the player's market value at that
//! OVR/age/position. Positive = team-friendly deal; negative = overpay.
//! Drives Cheapskate vs WinNow divergence: Cheapskates discount overpriced
//! deals more aggressively than WinNows.
//!
//! Components (each emits a Reason):
//! - `expected_market`        — talent value spread evenly across contract years
//! - `actual_salary`          — current year + future years discounted 8%/yr
//! - `option_value`           — player option = positive for player (negative
//!                              for team); reverse for team option
//! - `expiring_premium`       — last-year contracts are tradeable assets
//!
//! See `phases/M4-realism.md` "Worker A" for the full spec.
//!
//! Output `value` is in cents.

use crate::player_value::talent_value_cents;
use crate::weights::{ContractValueWeights, PlayerValueWeights};
use crate::Score;
use nba3k_core::{Contract, GMTraits, LeagueYear, Player};

/// Surplus value of a contract from the evaluator's POV.
///
/// If `contract` is `None`, the player has no contract on the books — return
/// zero with a single reason. Callers (free-agent valuation) use
/// `player_value` directly in that case.
pub fn contract_value(
    player: &Player,
    contract: Option<&Contract>,
    traits: &GMTraits,
    league_year: &LeagueYear,
    weights: &ContractValueWeights,
) -> Score {
    let mut score = Score::new(0.0);

    let Some(contract) = contract else {
        score.add("no_contract", 0.0);
        return score;
    };

    if contract.years.is_empty() {
        score.add("no_contract_years", 0.0);
        return score;
    }

    let current_season = league_year.season;
    let years_remaining: Vec<_> = contract
        .years
        .iter()
        .filter(|y| y.season >= current_season)
        .collect();
    let n_years = years_remaining.len().max(1) as f64;

    // 1. Expected market across contract life — spread the player's
    //    talent value over the contract length so we compare apples to apples
    //    against multi-year salary commitments.
    //    Use neutral PlayerValueWeights for the talent reference; we don't
    //    want salary_aversion or loyalty leaking into the talent baseline.
    let pv_weights = PlayerValueWeights::default();
    let talent_per_year = talent_value_cents(player, traits, &pv_weights) as f64;
    let expected_market_total = talent_per_year * n_years;
    score.add("expected_market", expected_market_total);

    // 2. Actual salary — present value with 8%/yr discount.
    let discount = weights.future_year_discount_pct.max(0.0) as f64;
    let mut pv_salary = 0.0_f64;
    for y in &years_remaining {
        let years_out = (y.season.0 as i32 - current_season.0 as i32).max(0) as i32;
        let factor = (1.0 - discount).powi(years_out);
        pv_salary += (y.salary.0 as f64) * factor;
    }
    let aversion = traits.salary_aversion.max(0.0) as f64;
    let weighted_salary = pv_salary * aversion;
    score.add("actual_salary", -weighted_salary);

    // 3. Option value. The wire convention: an option held by the **player**
    //    is bad for the team (player picks up cheap years, opts out of bad
    //    ones — adverse selection). A team option is good for the team.
    //    Magnitudes are a fraction of the optioned year's salary.
    let mut option_delta_cents = 0.0_f64;
    for y in &years_remaining {
        let salary_cents = y.salary.0 as f64;
        if y.player_option {
            // Player option: subtract a fraction of the salary as expected loss.
            let loss = salary_cents * (weights.option_value_player.max(0.0) as f64);
            option_delta_cents -= loss;
        }
        if y.team_option {
            // Team option: tiny premium (we get to walk away). The default
            // (-0.05) is intentionally negative-sign-encoded as a *credit*.
            // We flip sign so a positive `option_value_team` would
            // hypothetically penalize, but the default is a credit.
            let credit = salary_cents * (-(weights.option_value_team as f64));
            option_delta_cents += credit;
        }
    }
    if option_delta_cents.abs() > 0.0 {
        score.add("option_value", option_delta_cents);
    }

    // 4. Expiring premium — last-year contracts are matchable cap relief.
    //    Bonus only applies when the contract has exactly one year remaining
    //    (current year). Scaled by a fraction of the salary.
    if years_remaining.len() == 1 {
        let year = years_remaining[0];
        let bonus = (year.salary.0 as f64) * (weights.expiring_premium_pct.max(0.0) as f64);
        if bonus > 0.0 {
            score.add("expiring_premium", bonus);
        }
    }

    score.sort_reasons();
    score
}
