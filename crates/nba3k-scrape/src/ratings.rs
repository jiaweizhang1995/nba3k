//! Box-stats → 0..99 ratings.
//!
//! Spine: a simplified BPM (Box Plus/Minus) regression based on per-game
//! rates. The full BPM 2.0 model needs per-100 stats and team context we
//! don't have in v1; the simplified form below is enough to rank players
//! reliably (top stars 90+, role players 70-80, fringe 50-65).
//!
//! Pipeline:
//!   1. Compute a raw "production score" from per-game stats and shooting
//!      efficiency (TS-style proxy from FG%/3P%/FT%).
//!   2. Apply an age curve (peak 27, gentle linear decay before/after).
//!   3. Percentile-rank the position group → 0..99 sub-ratings per stat.
//!   4. Combine sub-ratings into an overall (position-weighted), then map
//!      that into 0..99 by anchoring the league-wide max to ~96 and min
//!      to ~55 (rookies/end-of-bench).
//!
//! No ML. Hand-calibrated. The user can override any specific player via
//! `data/rating_overrides.toml`.

use nba3k_core::{Position, Ratings};

use crate::sources::RawPlayerStats;

#[derive(Debug, Clone)]
pub struct RatedPlayer {
    pub stats: RawPlayerStats,
    pub ratings: Ratings,
    pub overall: u8,
    pub potential: u8,
}

/// Convert a slice of raw stats into rated players. We do percentile ranks
/// inside this function because every sub-rating needs the league context.
pub fn rate_all(players: &[RawPlayerStats]) -> Vec<RatedPlayer> {
    if players.is_empty() {
        return vec![];
    }

    // Per-stat percentiles (within the whole batch — small enough that
    // position-group splits add complexity without much accuracy gain).
    let pct_pts = percentile_ranks(players.iter().map(|p| p.pts).collect());
    let pct_trb = percentile_ranks(players.iter().map(|p| p.trb).collect());
    let pct_ast = percentile_ranks(players.iter().map(|p| p.ast).collect());
    let pct_stl = percentile_ranks(players.iter().map(|p| p.stl).collect());
    let pct_blk = percentile_ranks(players.iter().map(|p| p.blk).collect());
    let pct_fg = percentile_ranks(players.iter().map(|p| p.fg_pct).collect());
    let pct_3p = percentile_ranks(players.iter().map(|p| p.three_pct).collect());
    let pct_ft = percentile_ranks(players.iter().map(|p| p.ft_pct).collect());
    let pct_min = percentile_ranks(players.iter().map(|p| p.minutes_per_game).collect());

    // Per-game production composite — a single all-around impact signal that
    // captures stars who don't lead in any one stat but are top-shelf across
    // the board (Tatum 27/9/6, LeBron 25/8/9, Wembanyama 24/11/4 with blocks).
    // Weights are the standard fantasy-style impact proxy.
    let production: Vec<f32> = players
        .iter()
        .map(|p| p.pts + 1.5 * p.ast + 1.2 * p.trb + 2.0 * (p.stl + p.blk) - 0.7 * p.tov)
        .collect();
    let pct_prod = percentile_ranks(production);

    let mut out = Vec::with_capacity(players.len());
    for (i, p) in players.iter().enumerate() {
        // Map percentile signals → 21-attribute schema. Volume (PPG) is
        // the primary star signal — accuracy % and rate stats are noisy at
        // the per-game level. Heavy weight on pct_pts so a 27-PPG alpha
        // doesn't get washed out by a middling 3P%.
        let three = scale(pct_3p[i] * 0.5 + pct_pts[i] * 0.5);
        let mid = scale(pct_pts[i] * 0.7 + pct_fg[i] * 0.3);
        let finish = scale(pct_pts[i] * 0.7 + pct_fg[i] * 0.3);
        let pmk = scale(pct_ast[i] * 0.6 + pct_pts[i] * 0.4);
        let reb_score = scale(pct_trb[i]);
        let d_peri = scale(pct_stl[i] * 0.4 + pct_min[i] * 0.6);
        let d_int = scale(pct_blk[i] * 0.5 + pct_trb[i] * 0.3 + pct_min[i] * 0.2);
        let ath = scale((pct_min[i] + pct_stl[i] + pct_blk[i]) / 3.0);
        let ft = scale(pct_ft[i]);
        let care = scale(1.0 - p.tov.min(5.0) / 5.0); // ball-protection signal

        let mut ratings = Ratings {
            // Inside scoring (5) — derived from finishing signal
            close_shot: finish,
            driving_layup: finish,
            driving_dunk: finish.saturating_sub(2),
            standing_dunk: finish.saturating_sub(4),
            post_control: finish.saturating_sub(6),
            // Ranged shooting (3)
            mid_range: mid,
            three_point: three,
            free_throw: ft,
            // Handling (3)
            passing_accuracy: ((pmk as u32 + care as u32) / 2) as u8,
            ball_handle: pmk,
            speed_with_ball: ((ath as u32 + pmk as u32) / 2) as u8,
            // Defense (4)
            interior_defense: d_int,
            perimeter_defense: d_peri,
            steal: scale(pct_stl[i]),
            block: scale(pct_blk[i]),
            // Rebounding (2)
            off_reb: reb_score.saturating_sub(4),
            def_reb: reb_score,
            // Athleticism (4)
            speed: ath,
            agility: ath,
            strength: ((ath as u32 + d_int as u32) / 2) as u8,
            vertical: scale((pct_blk[i] + pct_min[i]) / 2.0),
        };

        // Position-aware tweak: PGs/SGs get a playmaking bonus, Cs lose
        // playmaking weight; Cs get a rebound/defense_interior bonus.
        position_tweak(&mut ratings, p.primary_position);

        let overall = blended_overall(&ratings, p.primary_position);
        // Only uplift players who actually played a meaningful sample. A
        // 30-game stint with elite per-game numbers is hot streak, not
        // star-caliber baseline (Jalen Johnson 2024-25, etc.).
        let overall_uplifted = if p.games >= 30.0 {
            apply_production_uplift(overall, pct_prod[i])
        } else {
            overall
        };
        let overall_age = apply_age_curve(overall_uplifted, p.age);
        let potential = potential_from(overall_age, p.age);

        out.push(RatedPlayer {
            stats: p.clone(),
            ratings,
            overall: overall_age,
            potential,
        });
    }
    out
}

fn percentile_ranks(values: Vec<f32>) -> Vec<f32> {
    // Average-rank for ties so a league of identical 3P% shooters all get
    // the same percentile (rather than spuriously ranking one of them top).
    let n = values.len();
    if n == 0 {
        return vec![];
    }
    if n == 1 {
        return vec![0.5];
    }
    let mut sorted: Vec<(usize, f32)> = values.iter().copied().enumerate().collect();
    sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut ranks = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && (sorted[j].1 - sorted[i].1).abs() < f32::EPSILON {
            j += 1;
        }
        // average rank for [i..j)
        let avg_rank = ((i + j - 1) as f32) / 2.0;
        let pct = avg_rank / (n - 1) as f32;
        for k in i..j {
            ranks[sorted[k].0] = pct;
        }
        i = j;
    }
    ranks
}

fn scale(pct: f32) -> u8 {
    // Piecewise-linear non-linear mapping that stretches the top end so
    // stars (top 5% by signal) actually land at 95-99, not 94. Old linear
    // mapping (50 + 49*p) compressed top stars and inflated mid-tier role
    // players. Anchors:
    //   p=0.00 → 50  (worst end-of-bench)
    //   p=0.50 → 73  (median rotation player)
    //   p=0.70 → 80
    //   p=0.85 → 88
    //   p=0.95 → 95
    //   p=1.00 → 99
    let p = pct.clamp(0.0, 1.0);
    let v = if p < 0.40 {
        50.0 + (p / 0.40) * 19.0           // 0.00..0.40 → 50..69
    } else if p < 0.70 {
        69.0 + ((p - 0.40) / 0.30) * 11.0  // 0.40..0.70 → 69..80
    } else if p < 0.85 {
        80.0 + ((p - 0.70) / 0.15) * 8.0   // 0.70..0.85 → 80..88
    } else if p < 0.95 {
        88.0 + ((p - 0.85) / 0.10) * 7.0   // 0.85..0.95 → 88..95
    } else {
        95.0 + ((p - 0.95) / 0.05) * 4.0   // 0.95..1.00 → 95..99
    };
    v.round().clamp(0.0, 99.0) as u8
}

fn position_tweak(r: &mut Ratings, pos: Position) {
    match pos {
        Position::PG => {
            r.ball_handle = r.ball_handle.saturating_add(8).min(99);
            r.passing_accuracy = r.passing_accuracy.saturating_add(6).min(99);
            r.three_point = r.three_point.saturating_add(2).min(99);
            r.off_reb = r.off_reb.saturating_sub(6);
            r.def_reb = r.def_reb.saturating_sub(6);
        }
        Position::SG => {
            r.three_point = r.three_point.saturating_add(4).min(99);
            r.mid_range = r.mid_range.saturating_add(2).min(99);
        }
        Position::SF => {
            r.speed = r.speed.saturating_add(2).min(99);
        }
        Position::PF => {
            r.off_reb = r.off_reb.saturating_add(4).min(99);
            r.def_reb = r.def_reb.saturating_add(4).min(99);
            r.interior_defense = r.interior_defense.saturating_add(2).min(99);
        }
        Position::C => {
            r.off_reb = r.off_reb.saturating_add(4).min(99);
            r.def_reb = r.def_reb.saturating_add(4).min(99);
            r.interior_defense = r.interior_defense.saturating_add(4).min(99);
            r.standing_dunk = r.standing_dunk.saturating_add(2).min(99);
            r.block = r.block.saturating_add(2).min(99);
            r.three_point = r.three_point.saturating_sub(8);
            r.ball_handle = r.ball_handle.saturating_sub(6);
        }
    }
}

fn blended_overall(r: &Ratings, pos: Position) -> u8 {
    // Delegate to the canonical position-weighted overall in nba3k-core.
    r.overall_for(pos)
}

/// Production-percentile floor: a top-tier all-around impact player should
/// never land below the floor for their tier, even if individual sub-ratings
/// undershoot due to noisy shooting %. Anchors:
///   prod_pct >= 0.99 → floor 95  (top 5 player league-wide)
///   prod_pct >= 0.96 → floor 90  (all-NBA tier)
///   prod_pct >= 0.90 → floor 85  (all-star tier)
///   prod_pct >= 0.80 → floor 80  (high-end starter)
fn apply_production_uplift(overall: u8, prod_pct: f32) -> u8 {
    let floor = if prod_pct >= 0.99 {
        95
    } else if prod_pct >= 0.96 {
        90
    } else if prod_pct >= 0.90 {
        85
    } else if prod_pct >= 0.80 {
        80
    } else {
        0
    };
    overall.max(floor)
}

fn apply_age_curve(overall: u8, age: u8) -> u8 {
    // Peak 27. Pre-peak: 2pts/yr (rookies have growth ahead). Post-peak:
    // 0.5pts/yr because real NBA vets hold skill — Curry at 38 is still 90+,
    // not 77. Hard cap at -6 either way so apply_age_curve never wipes a
    // genuine star. The base per-game data is already age-loaded so we just
    // soften the edges.
    let peak: i32 = 27;
    let delta = age as i32 - peak;
    let drop = if delta < 0 {
        (-delta).min(6) // young: -2 to -6 (rookie cap)
    } else {
        ((delta as f32) * 0.5).round() as i32  // older: half a point per year
    }
    .min(6);
    let adjusted = (overall as i32 - drop).clamp(35, 99);
    adjusted as u8
}

fn potential_from(current_overall: u8, age: u8) -> u8 {
    // Younger = more upside; older = capped at current.
    let bump = if age <= 22 {
        10
    } else if age <= 25 {
        6
    } else if age <= 27 {
        3
    } else {
        0
    };
    (current_overall as i32 + bump).clamp(current_overall as i32, 99) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(name: &str, pts: f32, ast: f32, trb: f32, age: u8, pos: Position) -> RawPlayerStats {
        RawPlayerStats {
            name: name.to_string(),
            primary_position: pos,
            secondary_position: None,
            age,
            games: 60.0,
            minutes_per_game: 32.0,
            pts,
            trb,
            ast,
            stl: 1.0,
            blk: 0.5,
            tov: 2.0,
            fg_pct: 0.45,
            three_pct: 0.36,
            ft_pct: 0.80,
            usage: None,
        }
    }

    #[test]
    fn star_outranks_role() {
        let players = vec![
            fake("Star", 30.0, 8.0, 6.0, 27, Position::SG),
            fake("Role", 6.0, 1.0, 3.0, 27, Position::SG),
            fake("Bench", 3.0, 0.5, 1.5, 27, Position::SG),
        ];
        let rated = rate_all(&players);
        assert!(rated[0].overall > rated[1].overall);
        assert!(rated[1].overall > rated[2].overall);
    }

    #[test]
    fn potential_higher_for_youth() {
        let players = vec![
            fake("Young", 18.0, 4.0, 5.0, 21, Position::SF),
            fake("Vet", 18.0, 4.0, 5.0, 32, Position::SF),
        ];
        let rated = rate_all(&players);
        assert!(rated[0].potential >= rated[0].overall);
        assert_eq!(rated[1].potential, rated[1].overall);
    }
}
