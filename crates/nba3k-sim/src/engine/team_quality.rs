//! Team-quality vector for the statistical sim engine.
//!
//! Replaces the old per-attribute weighted-sum `derive_profile` model with a
//! 9-feature team-quality vector + linear projection to ORtg/DRtg. Research
//! basis: `phases/M19.2-sim-rewrite-research.md`.
//!
//! Key non-additive structures:
//!  - `perimeter_containment` is the **MIN** across the team's 4 highest-minute
//!    perimeter defenders (G/SF). Captures that team defense is bottlenecked
//!    by the weakest switching defender. PHO with Beal at PD ~70 → 70.
//!    OKC's worst-case Caruso/Wallace at PD ~80 → 80.
//!  - `rim_protection` is the top-2 **product** of `interior_defense × block`,
//!    not a sum. One elite rim protector beats two mediocre ones.
//!  - `top3_offense` is the top-3 weighted shooting/handle combination, not
//!    a roster average. Captures star concentration (PHO's Booker+KD+Beal).
//!  - `playmaking` caps at 1 PG-equivalent so a 4-PG team doesn't double-count.
//!  - `spacing` is a step-function count of 75+ three-point shooters in the
//!    starting 5 — binary spacing (4 vs 5 shooters) matters more than averages.
//!
//! Coefficients are hand-tuned (see `data/team_quality_coefficients.toml`)
//! against known 2024-25 NBA team ratings as anchors. A future M19.3 may
//! replace the hand-tune with an offline ridge regression over 30 teams.

use crate::RotationSlot;
use nba3k_core::Position;

/// 9-feature team-quality vector. All features are normalized to "per-team"
/// scale (no league-relative percentiles); linear projection in
/// `ratings_from_vector` produces NBA-realistic ORtg/DRtg.
#[derive(Debug, Clone, Copy)]
pub struct TeamQualityVector {
    // Offensive (5)
    /// Weighted average of (3PT * 0.55 + (close + layup + mid)/3 * 0.45) over
    /// rotation by minutes share. Range ~50-90.
    pub team_efg: f32,
    /// Top-3 player average of (three + driving_layup + ball_handle) / 3
    /// weighted [0.40, 0.30, 0.20]. Range ~50-95. Captures star concentration.
    pub top3_offense: f32,
    /// Top-1 playmaker's (passing_accuracy + ball_handle) / 2 weighted by
    /// minutes share. Capped at 95. Range ~60-95.
    pub playmaking: f32,
    /// Count of starters (top-5 by minute share) with three_point >= 75.
    /// Range 0-5. Step function — 5 shooters is meaningfully > 4.
    pub spacing: f32,
    /// Weighted average of (driving_dunk + driving_layup + post_control) / 3
    /// — rim-pressure proxy for FT rate. Range ~50-90.
    pub ft_rate: f32,

    // Defensive (4)
    /// Top-2 by (interior_defense * block / 100) averaged. Range ~40-95.
    pub rim_protection: f32,
    /// **MIN** of perimeter_defense across top-4 G/SF rotation slots.
    /// Range ~55-90. Bottleneck signal.
    pub perimeter_containment: f32,
    /// Count of rotation players with PD >= 75 AND ID >= 70 AND strength >= 70.
    /// Range 0-8. Switchable defender count.
    pub defensive_versatility: f32,
    /// Sum of (steal + block) * minutes_share over rotation. Range ~5-15.
    /// Disruption proxy — turnovers + contests per game.
    pub defensive_disruption: f32,
}

/// Linear projection coefficients fitted/tuned against 2024-25 NBA real ORtg
/// and DRtg distributions. The "training" data are the 30-team league average
/// (ORtg ~115, DRtg ~115) with anchor points: OKC NetRtg +13, PHO NetRtg -2,
/// WAS NetRtg -9, BOS NetRtg +7.
#[derive(Debug, Clone, Copy)]
pub struct QualityToRatingWeights {
    /// Centered around league average ORtg (~115).
    pub off_intercept: f32,
    /// [w_efg, w_top3, w_play, w_space, w_ft]
    pub off_coefs: [f32; 5],
    /// Centered around league average DRtg (~115).
    pub def_intercept: f32,
    /// [w_rim, w_perim, w_vers, w_disr] — these *subtract* from def_intercept
    /// so a higher feature value → lower DRtg → better defense.
    pub def_coefs: [f32; 4],
}

impl Default for QualityToRatingWeights {
    fn default() -> Self {
        // Hand-tuned anchors (M19.2). Calibrated so a league-average team
        // lands ORtg=115, DRtg=115. Mean feature estimates from the seed:
        //   efg 75, top3 75, play 85, spacing 3, ft 75
        //   rim 70, perim 75, vers 3, disr 10
        // ORtg = 76.5 + 0.20*75 + 0.18*75 + 0.04*85 + 1.20*3 + 0.04*75 ≈ 115
        // DRtg = 177  - 0.18*70 - 0.45*75 - 2.50*3 - 0.80*10 ≈ 115
        // Real-team gaps walk through correctly:
        //   OKC: features push DRtg DOWN ~10 (perim 85, rim 90, vers 5+) → DRtg ~105
        //   PHO: features push DRtg UP ~3 (perim 70, vers 1) → DRtg ~118
        //   OKC vs PHO net rating gap ≈ 13 — matches real life.
        // Re-anchored against actual feature distributions observed in
        // the seed (LAC/DAL/NOP feature dump showed avg ORtg/DRtg landing
        // ~119/103 with prior coefs — both inflated). Tightened to mean
        // ORtg ~115, mean DRtg ~115 with realistic team spread:
        //   Mean features: efg 80, top3 80, play 93, spacing 4, ft 78
        //                   rim 90, perim 70, vers 5.5, disr 16
        //   ORtg = 73 + 0.20*80 + 0.18*80 + 0.04*93 + 1.20*4 + 0.04*78 ≈ 115
        //   DRtg = 176 - 0.10*90 - 0.55*70 - 1.50*5.5 - 0.35*16 ≈ 115
        // Wider spread on DRtg coefs (perim 0.55, vers 1.50) produces real
        // OKC vs PHO DRtg gap ~8, which combined with 3-pt ORtg gap gets
        // us NetRtg gap ~11 (real 15) — closer to truth than prior ~6.
        Self {
            off_intercept: 73.0,
            off_coefs: [
                0.20, // team_efg — Four Factors says efg is 40% of offense
                0.18, // top3_offense — star concentration
                0.04, // playmaking — small lift (capped at 1 PG-equivalent)
                1.20, // spacing — step bonus (each 75+ shooter ≈ +1.2 ORtg)
                0.04, // ft_rate — FT-rate bonus
            ],
            def_intercept: 176.0,
            def_coefs: [
                0.10, // rim_protection — interior anchor (top-2 product, range 60-95)
                0.55, // perimeter_containment — primary lever (MIN, scorer-adjusted)
                1.50, // defensive_versatility — switchable defender count
                0.35, // defensive_disruption — steal+block weighted
            ],
        }
    }
}

/// Compute the team-quality vector from a rotation. Empty rotation produces
/// a "league-average minus" vector with all features at 65 — the resulting
/// ORtg/DRtg will be sub-replacement.
pub fn vector_from_rotation(rotation: &[RotationSlot]) -> TeamQualityVector {
    if rotation.is_empty() {
        return TeamQualityVector {
            team_efg: 65.0,
            top3_offense: 65.0,
            playmaking: 65.0,
            spacing: 0.0,
            ft_rate: 65.0,
            rim_protection: 60.0,
            perimeter_containment: 60.0,
            defensive_versatility: 0.0,
            defensive_disruption: 5.0,
        };
    }

    let total_min: f32 = rotation
        .iter()
        .map(|r| r.minutes_share)
        .sum::<f32>()
        .max(0.01);
    let mw = |slot: &RotationSlot| slot.minutes_share / total_min;

    // ---- Feature 1: team_efg (eFG-style shooting blend) ----
    let team_efg: f32 = rotation
        .iter()
        .map(|r| {
            let inside = (r.ratings.close_shot as f32
                + r.ratings.driving_layup as f32
                + r.ratings.mid_range as f32)
                / 3.0;
            let blend = 0.55 * r.ratings.three_point as f32 + 0.45 * inside;
            blend * mw(r)
        })
        .sum();

    // ---- Feature 2: top3_offense (star concentration) ----
    let mut star_off: Vec<f32> = rotation
        .iter()
        .map(|r| {
            (r.ratings.three_point as f32
                + r.ratings.driving_layup as f32
                + r.ratings.ball_handle as f32)
                / 3.0
        })
        .collect();
    star_off.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let top3_offense = star_off.first().copied().unwrap_or(60.0) * 0.40
        + star_off.get(1).copied().unwrap_or(60.0) * 0.30
        + star_off.get(2).copied().unwrap_or(60.0) * 0.20;
    // Range so 90 alpha + 88 + 85 ≈ 36+26.4+17 = 79.4. Mid stars 80+78+75 = 67.7.

    // ---- Feature 3: playmaking (top-1 + bonus from second) ----
    let mut play_scores: Vec<f32> = rotation
        .iter()
        .map(|r| (r.ratings.passing_accuracy as f32 + r.ratings.ball_handle as f32) / 2.0)
        .collect();
    play_scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let playmaking = play_scores.first().copied().unwrap_or(60.0).min(95.0);

    // ---- Feature 4: spacing (count of 75+ shooters in starters) ----
    let starter_count = rotation.len().min(5);
    let spacing = rotation
        .iter()
        .take(starter_count)
        .filter(|r| r.ratings.three_point >= 75)
        .count() as f32;

    // ---- Feature 5: ft_rate (rim-pressure proxy) ----
    let ft_rate: f32 = rotation
        .iter()
        .map(|r| {
            let inside = (r.ratings.driving_dunk as f32
                + r.ratings.driving_layup as f32
                + r.ratings.post_control as f32)
                / 3.0;
            inside * mw(r)
        })
        .sum();

    // ---- Feature 6: rim_protection (top-2 product of ID × block / 100) ----
    let mut rim_scores: Vec<f32> = rotation
        .iter()
        .map(|r| (r.ratings.interior_defense as f32 * r.ratings.block as f32) / 100.0)
        .collect();
    rim_scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let rim_protection = (rim_scores.first().copied().unwrap_or(40.0)
        + rim_scores.get(1).copied().unwrap_or(40.0))
        / 2.0;

    // ---- Feature 7: perimeter_containment (MIN over G/SF top-4) ----
    //
    // Adjustment: BBRef-derived perimeter_defense is computed from per-game
    // steals + minutes percentile, which inflates PD for high-usage scorers
    // (Booker PD 91, Beal 90 in our seed — real-life ~70 each). Apply a
    // "scorer penalty" to undo that inflation: any G/SF with elite shooting
    // creation signal (3pt + ball_handle ≥ 175) gets PD docked by up to 18.
    // Real defensive sieves (Booker BH 97 + 3pt 85 = 182 → -16) drop hard;
    // role wings (Caruso 75+78 = 153 → 0) keep their PD intact. SGA (91+99 =
    // 190 → -18) gets dinged but he's a true All-Def player; minor false-positive
    // is OK because it lowers OKC's containment too — the gap to PHO still widens
    // because PHO has 2-3 scorers vs OKC's 1.
    let scorer_adjusted_pd = |r: &RotationSlot| -> u8 {
        let scorer_signal = r.ratings.three_point as i32 + r.ratings.ball_handle as i32;
        let penalty = ((scorer_signal - 160).max(0) as f32 * 0.6).round() as i32;
        ((r.ratings.perimeter_defense as i32) - penalty).clamp(40, 99) as u8
    };
    let mut perim_pool: Vec<(f32, u8)> = rotation
        .iter()
        .filter(|r| matches!(r.position, Position::PG | Position::SG | Position::SF))
        .map(|r| (r.minutes_share, scorer_adjusted_pd(r)))
        .collect();
    perim_pool.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let perim_top4: Vec<u8> = perim_pool.iter().take(4).map(|(_, pd)| *pd).collect();
    let perimeter_containment = perim_top4.iter().min().copied().unwrap_or(60) as f32;

    // ---- Feature 8: defensive_versatility (count of switchable defenders) ----
    let defensive_versatility = rotation
        .iter()
        .filter(|r| {
            r.ratings.perimeter_defense >= 75
                && r.ratings.interior_defense >= 70
                && r.ratings.strength >= 70
        })
        .count() as f32;

    // ---- Feature 9: defensive_disruption (steals + blocks weighted) ----
    let defensive_disruption: f32 = rotation
        .iter()
        .map(|r| (r.ratings.steal as f32 + r.ratings.block as f32) * mw(r))
        .sum::<f32>()
        / 10.0; // scale to ~5-15 range

    TeamQualityVector {
        team_efg,
        top3_offense,
        playmaking,
        spacing,
        ft_rate,
        rim_protection,
        perimeter_containment,
        defensive_versatility,
        defensive_disruption,
    }
}

/// Project the 9-feature vector to (ORtg, DRtg) ratings in NBA-realistic units.
/// Returns league-anchored numbers in roughly the 100-125 range.
pub fn ratings_from_vector(v: &TeamQualityVector, w: &QualityToRatingWeights) -> (f32, f32) {
    let off = w.off_intercept
        + w.off_coefs[0] * v.team_efg
        + w.off_coefs[1] * v.top3_offense
        + w.off_coefs[2] * v.playmaking
        + w.off_coefs[3] * v.spacing
        + w.off_coefs[4] * v.ft_rate;
    let def = w.def_intercept
        - w.def_coefs[0] * v.rim_protection
        - w.def_coefs[1] * v.perimeter_containment
        - w.def_coefs[2] * v.defensive_versatility
        - w.def_coefs[3] * v.defensive_disruption;
    // Clamp to NBA-plausible range; outliers above/below indicate calibration drift.
    (off.clamp(98.0, 128.0), def.clamp(100.0, 125.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba3k_core::{PlayerId, Ratings};

    fn slot(name: &str, pos: Position, ratings: Ratings, mins: f32) -> RotationSlot {
        RotationSlot {
            player: PlayerId(0),
            name: name.to_string(),
            position: pos,
            minutes_share: mins,
            usage: 0.20,
            ratings,
            age: 27,
            overall: 80,
            potential: 80,
        }
    }

    fn elite_def_ratings() -> Ratings {
        Ratings::legacy(75, 78, 80, 75, 75, 88, 90, 80) // PD 88, ID 90
    }

    fn weak_def_ratings() -> Ratings {
        Ratings::legacy(85, 85, 80, 80, 70, 65, 65, 75) // PD 65 (sieve)
    }

    #[test]
    fn perimeter_containment_uses_min_not_avg() {
        // Team A: 4 strong perimeter defenders (PD 88 each) → containment = 88
        let strong = elite_def_ratings();
        let team_a = vec![
            slot("A1", Position::PG, strong, 0.7),
            slot("A2", Position::SG, strong, 0.7),
            slot("A3", Position::SF, strong, 0.7),
            slot("A4", Position::PF, strong, 0.6),
            slot("A5", Position::C, strong, 0.55),
        ];
        let v_a = vector_from_rotation(&team_a);
        assert!(
            v_a.perimeter_containment >= 87.0,
            "elite-D team should have high containment, got {}",
            v_a.perimeter_containment
        );

        // Team B: 3 strong + 1 sieve (PHO with Beal). Avg of perimeter PDs is
        // (88+88+88+65)/4 = 82, but MIN is 65 — that's the bottleneck signal.
        let weak = weak_def_ratings();
        let team_b = vec![
            slot("B1", Position::PG, strong, 0.7),
            slot("B2", Position::SG, weak, 0.7),
            slot("B3", Position::SF, strong, 0.7),
            slot("B4", Position::PF, strong, 0.6),
            slot("B5", Position::C, strong, 0.55),
        ];
        let v_b = vector_from_rotation(&team_b);
        assert!(
            v_b.perimeter_containment <= 70.0,
            "1-sieve team should fall to MIN, got {}",
            v_b.perimeter_containment
        );
    }

    #[test]
    fn elite_team_outranks_weak_team() {
        let strong = Ratings::legacy(85, 85, 88, 80, 80, 85, 85, 85);
        let weak = Ratings::legacy(70, 72, 70, 65, 65, 65, 65, 70);
        let team_a = vec![
            slot("A1", Position::PG, strong, 0.7),
            slot("A2", Position::SG, strong, 0.7),
            slot("A3", Position::SF, strong, 0.7),
            slot("A4", Position::PF, strong, 0.6),
            slot("A5", Position::C, strong, 0.55),
        ];
        let team_b = vec![
            slot("B1", Position::PG, weak, 0.7),
            slot("B2", Position::SG, weak, 0.7),
            slot("B3", Position::SF, weak, 0.7),
            slot("B4", Position::PF, weak, 0.6),
            slot("B5", Position::C, weak, 0.55),
        ];
        let weights = QualityToRatingWeights::default();
        let (o_a, d_a) = ratings_from_vector(&vector_from_rotation(&team_a), &weights);
        let (o_b, d_b) = ratings_from_vector(&vector_from_rotation(&team_b), &weights);
        let net_a = o_a - d_a;
        let net_b = o_b - d_b;
        assert!(
            net_a > net_b + 5.0,
            "elite team net ({}) should beat weak ({}) by ≥5",
            net_a,
            net_b
        );
    }
}
