//! Worker A — player value model.
//!
//! `player_value(player, traits, evaluator, league, weights)` returns a
//! `Score` whose `value` is the perceived surplus value (cents) of the player
//! from the evaluator's POV. Components contribute as Reasons:
//! - positional baseline (no flat OVR→$ globally)
//! - age curve (peak 27, position-specific cliffs)
//! - star premium (nonlinear above 88 OVR)
//! - contract surplus (talent_value − salary, weighted by salary_aversion)
//! - loyalty bonus (only when player is on evaluator's team)
//!
//! Named-star override and asset-fit are applied at composite time
//! (Worker D), not here. This model is pure OVR/age/contract math.
//!
//! Output units: `value` is in cents (i64 representable inside f64).
//!
//! See `phases/M4-realism.md` "Worker A" for the full spec.

use crate::weights::PlayerValueWeights;
use crate::Score;
use nba3k_core::{Cents, GMTraits, LeagueSnapshot, Player, Position, TeamId};

// ---------------------------------------------------------------------------
// Component curves. Pure functions; no I/O; no allocation. Factored out so
// `contract_value` can reuse the talent-side formula.
// ---------------------------------------------------------------------------

/// Positional baseline talent value in *cents*.
///
/// Recalibrated for M4 to drop the M3 hack of "everyone-25-OVR-75".
/// The curve is intentionally **lower** than the M3 trade-engine seed so
/// that contract-overpay tests bite (an OVR-80 player on $40M is overpaid;
/// the *star premium* component carries the top tier separately).
///
/// Anchors (neutral position SF, no age modifier, no star premium):
///   OVR 50 → $0, OVR 60 → ~$0.8M, OVR 70 → ~$5M, OVR 75 → ~$10M,
///   OVR 80 → ~$20M, OVR 85 → ~$30M, OVR 88 → ~$38M, OVR 92 → ~$50M,
///   OVR 95 → ~$58M (alone — star premium below adds another $98M+),
///   OVR 99 → ~$80M (alone — star premium adds ~$960M at peak).
/// Star premium (above 88) is layered on top by `star_premium_dollars`.
///
/// Positional adjustment: lead guards command a small premium because of
/// playmaking scarcity; bigs get a small discount at the floor (bigs are
/// easier to find at OVR < 78). The multiplier is applied uniformly.
pub(crate) fn baseline_for_ovr_position(ovr: u8, pos: Position) -> i64 {
    let ovr = ovr.min(99) as f64;
    if ovr <= 50.0 {
        return 0;
    }
    let x = ovr - 50.0; // 0..=49
    let v_millions = (x / 49.0).powf(2.6) * 55.0;

    let pos_mul = match pos {
        // Lead guards command a small premium; defensive wings near neutral;
        // bigs slightly discounted at lower OVR but the curve catches them at
        // star tier (the multiplier is applied uniformly).
        Position::PG => 1.04,
        Position::SG => 1.00,
        Position::SF => 1.00,
        Position::PF => 0.98,
        Position::C => 0.96,
    };
    (v_millions * pos_mul * 1_000_000.0 * 100.0) as i64
}

/// Age multiplier with position-specific cliffs. Peak at `weights.age_peak`.
///
/// Position-specific: bigs (PF/C) decline slower past peak (size and
/// rebounding age better than guard quickness). Guards (PG/SG) have a
/// slightly steeper post-peak cliff.
pub(crate) fn age_multiplier(age: u8, pos: Position, peak: f32) -> f64 {
    let a = age as f64;
    let peak = peak as f64;
    if a < 19.0 {
        return 0.85;
    }
    if a <= peak {
        // Pre-peak: rising slope. 19→0.93, 22→0.99, 25→1.03, 27→1.05.
        return 1.05 - 0.012 * (peak - a);
    }
    // Post-peak: cumulative annual decline. Guards decline faster than bigs.
    // Tuned so a 33-yo wing sits ≥30% below their 28-yo same-OVR self.
    let (mid_step, late_step, oldest_step) = match pos {
        Position::PG | Position::SG => (0.040, 0.090, 0.12),
        Position::SF => (0.035, 0.085, 0.11),
        Position::PF | Position::C => (0.025, 0.060, 0.10),
    };
    let mut m = 1.05;
    let peak_year = peak.floor() as u32;
    for year in (peak_year + 1)..=(a as u32) {
        let inc = if year <= 30 {
            mid_step
        } else if year <= 33 {
            late_step
        } else {
            oldest_step
        };
        m -= inc;
        if m < 0.30 {
            m = 0.30;
        }
    }
    m
}

/// Star premium **delta** (added to baseline, not a multiplier on it).
///
/// Above OVR 88, the league is a different supply curve. The previous flat
/// `star_premium = 1.6` multiplier produced linear extrapolation past 88; we
/// want **nonlinear** so 95 is dramatically more than 89, and 89 is more
/// than 87 by more than the linear curve predicts.
///
/// Returns dollars (whole) of premium contribution. Scaled by
/// `traits.star_premium` (1.0 default; StarHunter 1.6).
pub(crate) fn star_premium_dollars(ovr: u8, traits: &GMTraits, threshold: u8) -> i64 {
    if ovr < threshold {
        return 0;
    }
    let over = (ovr - threshold) as f64; // 0..=11
    // Quadratic in `over`. With default traits (star_premium=1.0):
    //   OVR89→$2M, OVR92→$32M, OVR95→$98M, OVR99→$242M (premium alone).
    // StarHunter (1.6×) bumps these up; Wildcard/most archetypes stay 1.0.
    let raw_millions = 2.0 * over * over;
    let scale = traits.star_premium.clamp(0.0, 3.0) as f64;
    (raw_millions * scale * 1_000_000.0) as i64
}

/// Talent-side value (baseline × age + star premium delta) in cents.
/// Reused by `contract_value` to compute "expected market for this player".
pub(crate) fn talent_value_cents(player: &Player, traits: &GMTraits, weights: &PlayerValueWeights) -> i64 {
    // Blend OVR and potential per GM trait (rebuilders weight potential
    // higher). Falls through to OVR for neutral GMs.
    let cur_w = traits.current_overall_weight.max(0.0) as f64;
    let pot_w = traits.potential_weight.max(0.0) as f64;
    let denom = (cur_w + pot_w).max(0.001);
    let blended = (cur_w * player.overall as f64 + pot_w * player.potential as f64) / denom;
    let blended_ovr = blended.round().clamp(0.0, 99.0) as u8;

    let pos = player.primary_position;
    let baseline = baseline_for_ovr_position(blended_ovr, pos) as f64;
    let age_mul = age_multiplier(player.age, pos, weights.age_peak);
    let aged = baseline * age_mul;

    // Star premium uses the *raw* OVR (not the blended one) so calling a
    // 78-OVR with potential 92 still doesn't get a star bonus today. The
    // potential boost is captured already through the blended baseline.
    let star = star_premium_dollars(player.overall, traits, weights.star_threshold_ovr) as f64 * 100.0;

    (aged + star) as i64
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute player value Score from evaluator's perspective.
/// `evaluator` is the team currently asking; used for loyalty bonus.
pub fn player_value(
    player: &Player,
    traits: &GMTraits,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    weights: &PlayerValueWeights,
) -> Score {
    let pos = player.primary_position;
    let cur_w = traits.current_overall_weight.max(0.0) as f64;
    let pot_w = traits.potential_weight.max(0.0) as f64;
    let denom = (cur_w + pot_w).max(0.001);
    let blended = (cur_w * player.overall as f64 + pot_w * player.potential as f64) / denom;
    let blended_ovr = blended.round().clamp(0.0, 99.0) as u8;

    // 1. Positional baseline.
    let baseline_cents = baseline_for_ovr_position(blended_ovr, pos);
    let mut score = Score::new(0.0);
    score.add("positional_baseline", baseline_cents as f64);

    // 2. Age curve — emitted as a delta from baseline (multiplier applied).
    let age_mul = age_multiplier(player.age, pos, weights.age_peak);
    let aged = (baseline_cents as f64) * age_mul;
    let age_delta = aged - baseline_cents as f64;
    score.add("age_curve", age_delta);

    // 2b. GM age preference (small bias on top of the multiplicative curve).
    // Positive `age_curve_weight` (Rebuilder, Analytics) bumps under-25;
    // negative (OldSchool) bumps veterans. ±10% cap.
    let pref_w = traits.age_curve_weight as f64;
    if pref_w.abs() > 0.001 {
        let pref_factor = (((25.0 - player.age as f64) * pref_w * 0.012).clamp(-0.10, 0.10)) as f64;
        let pref_delta = aged * pref_factor;
        score.add("gm_age_preference", pref_delta);
    }

    // 3. Star premium — nonlinear above threshold.
    let star_cents = star_premium_dollars(player.overall, traits, weights.star_threshold_ovr) * 100;
    if star_cents != 0 {
        score.add("star_premium", star_cents as f64);
    }

    // 4. Contract surplus — subtract salary × salary_aversion (current year).
    // We treat the contract burden as a negative contribution; sign-flip is
    // intentional so callers see "contract_burden: −$N" reasons.
    if let Some(contract) = &player.contract {
        let salary = contract.current_salary(league.current_season);
        let burden = (salary.0 as f64) * (traits.salary_aversion.max(0.0) as f64);
        if burden > 0.0 {
            score.add("contract_burden", -burden);
        }
    }

    // 5. Loyalty bonus — only if evaluator owns this player today.
    if let Some(owner) = player.team {
        if owner == evaluator && traits.loyalty > 0.0 {
            // Bonus scales with the unmodified positional baseline so it
            // tracks how much it actually hurts to ship a homegrown star.
            let bonus = (baseline_cents as f64)
                * (traits.loyalty.clamp(0.0, 1.0) as f64)
                * (weights.loyalty_bonus_default.clamp(0.0, 1.0) as f64);
            if bonus > 0.0 {
                score.add("loyalty_bonus", bonus);
            }
        }
    }

    // Sort reasons by |delta| desc so callers see the dominant component
    // first. Top-K trimming is a caller decision.
    score.sort_reasons();
    score
}

/// Helper: same as `player_value` but takes `Cents` for callers that prefer it.
#[allow(dead_code)]
pub(crate) fn player_value_cents(
    player: &Player,
    traits: &GMTraits,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    weights: &PlayerValueWeights,
) -> Cents {
    Cents(player_value(player, traits, evaluator, league, weights).value as i64)
}
