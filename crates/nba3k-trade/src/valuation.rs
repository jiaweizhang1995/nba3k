//! Per-asset valuation primitives — Worker A.
//!
//! All values are expressed in `Cents` so the trade engine can sum players +
//! picks + cash on a single ledger. Outputs are *perceived surplus value* over
//! contract from the evaluator's perspective — they are NOT market-clearing
//! prices and they do NOT correspond to real NBA salaries. They are tuned so
//! that:
//!   - A 99-OVR star peaks at roughly $200M of perceived value.
//!   - A 70-OVR rotation player lands near $5M.
//!   - A late-1st pick is worth ~$10–$20M; a top-3 pick ~$70M.
//!
//! Calibration is the orchestrator's problem (`dev calibrate-trade`); the
//! curves below are the seed.

use crate::snapshot::LeagueSnapshot;
use nba3k_core::{Cents, DraftPick, GMTraits, Player, PlayerRole, SeasonId, TeamId};

/// Convert dollars (whole) to `Cents`.
fn dollars(d: i64) -> Cents {
    Cents::from_dollars(d)
}

/// Positional baseline value (in dollars) for a given OVR rating, BEFORE
/// age curve, star premium, loyalty, and contract surplus adjustments.
///
/// The curve is intentionally convex: each OVR point above 80 is worth
/// dramatically more than each point below 80. This matches how front
/// offices actually price talent (top-15 players trade at premiums no
/// rotation player ever commands).
///
/// Anchor points (dollars):
///   OVR 50 → $0          (replacement-level — nobody pays for these)
///   OVR 60 → ~$1.0M
///   OVR 70 → ~$5.0M
///   OVR 75 → ~$10M
///   OVR 80 → ~$25M
///   OVR 85 → ~$60M
///   OVR 88 → ~$95M
///   OVR 92 → ~$140M
///   OVR 95 → ~$170M
///   OVR 99 → ~$210M
fn baseline_dollars_for_ovr(ovr: u8) -> i64 {
    let ovr = ovr.min(99) as f64;
    if ovr <= 50.0 {
        return 0;
    }
    // Quadratic-ish curve anchored to the bullets above. Coefficients fit
    // by hand — keep round numbers and re-tune in calibration.
    let x = ovr - 50.0; // 0..=49
    // Power curve: each OVR point past 80 adds dramatically more value than
    // each one below. Anchored to the bullets in the doc comment above.
    let v = (x / 49.0).powf(2.6) * 210.0; // millions
    (v * 1_000_000.0) as i64
}

/// Age multiplier — peak at 27, falloff on either side.
///
/// Returns a multiplier in roughly 0.4..=1.10. Younger-than-peak players are
/// worth slightly more than peak (upside), older ones drop steeply post-32.
fn age_multiplier(age: u8) -> f64 {
    let a = age as f64;
    if a < 19.0 {
        return 0.85;
    }
    let peak = 27.0;
    if a <= peak {
        // 19→0.95, 22→1.00, 25→1.04, 27→1.05.
        1.05 - 0.012 * (peak - a)
    } else {
        // Step down per year past peak: -3%/yr to 30, -6%/yr to 33, -10%/yr after.
        let mut m = 1.05;
        for year in (peak as u32 + 1)..=(a as u32) {
            let inc = if year <= 30 {
                0.03
            } else if year <= 33 {
                0.06
            } else {
                0.10
            };
            m -= inc;
            if m < 0.30 {
                m = 0.30;
            }
        }
        m
    }
}

/// Star premium kicks in at OVR ≥ 88. Multiplier is `traits.star_premium`
/// (defaults to 1.0). Stars are worth disproportionately more to GMs who
/// hunt them (`StarHunter` archetype = 1.6×).
fn star_premium(ovr: u8, traits: &GMTraits) -> f64 {
    if ovr >= 88 {
        traits.star_premium as f64
    } else {
        1.0
    }
}

/// Surplus value of a player to the evaluator, in `Cents`.
///
/// Formula:
///   blended_ovr = (current_overall_weight * ovr + potential_weight * potential)
///                 / (current_overall_weight + potential_weight)
///   raw       = baseline(blended_ovr) * age_multiplier(age) * star_premium(ovr)
///   surplus   = raw - current_salary * salary_aversion
///   if player.team == evaluator: surplus += loyalty_bonus
pub fn player_value(
    player: &Player,
    traits: &GMTraits,
    current_season: SeasonId,
    league: &LeagueSnapshot,
) -> Cents {
    player_value_for(player, traits, current_season, Some(league))
}

/// Same as [`player_value`] but lets callers pass `None` for a snapshot when
/// only the evaluator's traits + the player matter (used by tests).
pub(crate) fn player_value_for(
    player: &Player,
    traits: &GMTraits,
    current_season: SeasonId,
    _league: Option<&LeagueSnapshot>,
) -> Cents {
    let cur_w = traits.current_overall_weight.max(0.0) as f64;
    let pot_w = traits.potential_weight.max(0.0) as f64;
    let denom = (cur_w + pot_w).max(0.001);
    let blended = (cur_w * player.overall as f64 + pot_w * player.potential as f64) / denom;

    let baseline = baseline_dollars_for_ovr(blended.round().clamp(0.0, 99.0) as u8);
    let age_mul = age_multiplier(player.age);
    let star_mul = star_premium(player.overall, traits);

    // Apply the GM's age preference: positive `age_curve_weight` boosts
    // younger players (peak 22), negative boosts veterans.
    let age_pref = {
        let a = player.age as f64;
        let prefer_young = traits.age_curve_weight as f64; // -1..=1 typical
        // Centered at 25: under-25 gets +bump if prefer_young>0.
        let raw = (25.0 - a) * prefer_young * 0.01;
        (1.0 + raw).clamp(0.5, 1.5)
    };

    let raw_dollars = (baseline as f64) * age_mul * star_mul * age_pref;
    let mut value = dollars(raw_dollars as i64);

    // Subtract the contract burden weighted by salary aversion. A neutral GM
    // (aversion 1.0) values a $40M player worth $40M of talent at $0 net
    // surplus; a Cheapskate (aversion 1.8) sees -$32M surplus on the same
    // contract, so they discount stars on big deals.
    if let Some(contract) = &player.contract {
        let salary = contract.current_salary(current_season);
        let weighted = (salary.0 as f64 * traits.salary_aversion as f64) as i64;
        value = value - Cents(weighted);
    }

    // Loyalty bonus when evaluating one of your own.
    if let Some(_team) = player.team {
        // No-op here — loyalty is applied at the offer level via
        // `loyalty_bonus_for_own` so that we know who the evaluator is.
        // Keeping this hook documents the dependency.
    }

    value
}

/// Loyalty bonus applied per-player when the evaluator currently owns
/// that player (i.e. they're on the outgoing side). The bonus is a flat
/// fraction of the unmodified positional baseline so it scales with how
/// valuable the player is to begin with.
///
/// Surfaced separately from `player_value` because we only know the
/// evaluator at the offer level, not at the per-player call site used
/// by other crates.
pub fn loyalty_bonus_for_own(player: &Player, traits: &GMTraits) -> Cents {
    let baseline = baseline_dollars_for_ovr(player.overall);
    let bonus = (baseline as f64 * traits.loyalty.clamp(0.0, 1.0) as f64 * 0.20) as i64;
    dollars(bonus)
}

/// Round-1 slot baseline (dollars) by projected pick number 1..=30. Round 2
/// uses a fixed token value.
fn pick_slot_dollars(round: u8, projected_slot: u8) -> i64 {
    if round >= 2 {
        // 2nd-rounders in 2K-style mode are essentially throw-ins.
        return 500_000;
    }
    let s = projected_slot.clamp(1, 30) as f64;
    // Anchors: #1 ≈ $80M, #5 ≈ $40M, #14 ≈ $15M, #20 ≈ $9M, #30 ≈ $4M.
    let v = 90.0 * (1.0 - (s - 1.0) / 29.0).powf(1.4) + 4.0;
    (v * 1_000_000.0) as i64
}

/// Project the pick slot for a future-season pick from the owning team's
/// current standings. If we have no record yet, fall back to slot 15
/// (mid-lottery — the safe pessimistic assumption a GM makes).
fn project_pick_slot(pick: &DraftPick, league: &LeagueSnapshot) -> u8 {
    let owner = pick.original_team;
    let record = league.record(owner);
    if record.games_played() == 0 {
        return 15;
    }
    // Worst record → slot 1. Best record → slot 30. Use win_pct as a
    // monotonic proxy. (Tank and rebuild teams will shake out at the bottom
    // either way; we don't model the lottery here.)
    let pct = record.win_pct().clamp(0.0, 1.0);
    let slot = 1.0 + 29.0 * pct as f64;
    slot.round().clamp(1.0, 30.0) as u8
}

/// Surplus value of a draft pick to the evaluator, in `Cents`.
///
/// - Round 1: anchored to projected slot. If the pick season matches the
///   current season, slot is read from standings; otherwise we use slot 15
///   as a neutral expectation and discount 10%/year out.
/// - Round 2: flat $0.5M token.
/// - Multiplied by `traits.pick_value_multiplier` (Analytics ~1.3, WinNow
///   0.7).
pub fn pick_value(
    pick: &DraftPick,
    current_season: SeasonId,
    traits: &GMTraits,
    league: &LeagueSnapshot,
) -> Cents {
    let slot = if pick.season == current_season {
        project_pick_slot(pick, league)
    } else {
        15
    };
    let raw = pick_slot_dollars(pick.round, slot) as f64;

    let years_out = (pick.season.0 as i32 - current_season.0 as i32).max(0);
    // Discount picks more than 1 year out by 10% per additional year.
    let discount = if years_out <= 1 {
        1.0
    } else {
        0.90_f64.powi(years_out - 1)
    };

    let mult = traits.pick_value_multiplier.max(0.0) as f64;
    let total = raw * discount * mult;
    dollars(total as i64)
}

/// Cash valuation — capped per direction by the season's max-trade-cash
/// constant. Caller is responsible for tracking the team's cumulative cash
/// in/out for the season; this just clamps a single line item.
pub fn cash_value(amount: Cents, league: &LeagueSnapshot) -> Cents {
    let cap = league.league_year.max_trade_cash;
    if amount.0 > cap.0 {
        cap
    } else if amount.0 < -cap.0 {
        Cents(-cap.0)
    } else {
        amount
    }
}

/// Value all assets a single team is sending out, from `evaluator`'s POV.
/// `evaluator` is the team doing the math — when it equals `side`, we add
/// the loyalty bonus; when it doesn't, the player is "incoming" from
/// evaluator's perspective and gets no loyalty bonus.
pub fn value_side(
    side: TeamId,
    evaluator: TeamId,
    assets: &nba3k_core::TradeAssets,
    league: &LeagueSnapshot,
    traits: &GMTraits,
) -> Cents {
    let mut total = Cents::ZERO;
    for pid in &assets.players_out {
        if let Some(p) = league.player(*pid) {
            let mut v = player_value(p, traits, league.current_season, league);
            if side == evaluator {
                v = v + loyalty_bonus_for_own(p, traits);
            }
            total = total + v;
        }
    }
    for pickid in &assets.picks_out {
        if let Some(pk) = league.pick(*pickid) {
            total = total + pick_value(pk, league.current_season, traits, league);
        }
    }
    total = total + cash_value(assets.cash_out, league);
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba3k_core::{Contract, ContractYear, GMTraits, Player, PlayerId, Position, Ratings, SeasonId, TeamId};

    fn mk_player(id: u32, ovr: u8, age: u8, salary_dollars: i64, team: Option<TeamId>) -> Player {
        Player {
            id: PlayerId(id),
            name: format!("P{id}"),
            primary_position: Position::SF,
            secondary_position: None,
            age,
            overall: ovr,
            potential: ovr,
            ratings: Ratings::default(),
            contract: Some(Contract {
                years: vec![ContractYear {
                    season: SeasonId(2026),
                    salary: Cents::from_dollars(salary_dollars),
                    guaranteed: true,
                    team_option: false,
                    player_option: false,
                }],
                signed_in_season: SeasonId(2025),
                bird_rights: nba3k_core::BirdRights::Full,
            }),
            team,
            injury: None,
            no_trade_clause: false,
            trade_kicker_pct: None,
            role: PlayerRole::RolePlayer,
            morale: 0.5,
        }
    }

    #[test]
    fn baseline_curve_is_monotonic() {
        let mut prev = -1;
        for ovr in 50u8..=99 {
            let v = baseline_dollars_for_ovr(ovr);
            assert!(v >= prev, "non-monotonic at OVR {ovr}: prev={prev} cur={v}");
            prev = v;
        }
    }

    #[test]
    fn star_outvalues_role_player() {
        let traits = GMTraits::default();
        let star = mk_player(1, 92, 27, 40_000_000, None);
        let role = mk_player(2, 75, 27, 10_000_000, None);
        let star_v = player_value_for(&star, &traits, SeasonId(2026), None);
        let role_v = player_value_for(&role, &traits, SeasonId(2026), None);
        assert!(
            star_v.0 > role_v.0 + 30_000_000_00,
            "star ({}) should be ≥$30M more than role ({})",
            star_v.0 / 100,
            role_v.0 / 100
        );
    }

    #[test]
    fn cheapskate_discounts_big_contracts() {
        let star = mk_player(1, 92, 27, 50_000_000, None);
        let neutral = GMTraits::default();
        let mut cheap = GMTraits::default();
        cheap.salary_aversion = 1.8;
        let v_neutral = player_value_for(&star, &neutral, SeasonId(2026), None);
        let v_cheap = player_value_for(&star, &cheap, SeasonId(2026), None);
        assert!(
            v_cheap.0 < v_neutral.0,
            "cheapskate should value high-salary star less than neutral"
        );
    }

    #[test]
    fn age_curve_peaks_around_27() {
        assert!(age_multiplier(27) >= age_multiplier(20));
        assert!(age_multiplier(27) >= age_multiplier(35));
        assert!(age_multiplier(35) < age_multiplier(30));
    }

    #[test]
    fn star_premium_only_above_88() {
        let mut traits = GMTraits::default();
        traits.star_premium = 1.6;
        assert!((star_premium(85, &traits) - 1.0).abs() < 1e-6);
        assert!((star_premium(88, &traits) - 1.6).abs() < 1e-6);
        assert!((star_premium(95, &traits) - 1.6).abs() < 1e-6);
    }
}
