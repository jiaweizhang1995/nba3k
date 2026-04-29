//! Worker B — team context model.
//!
//! Returns both a discrete `TeamMode` and a continuous score vector so
//! downstream consumers can do soft mixing (e.g., "70% contend, 30%
//! retool"). Replaces `nba3k_trade::context::classify_team`.
//!
//! See `phases/M4-realism.md` "Worker B" for the full spec.
//!
//! Continuous-score philosophy: each component (roster age, top-OVR
//! presence, standings, cap flexibility) emits a value in `[0.0, 1.0]`
//! that pulls toward "contend" or "rebuild". The discrete `TeamMode` is
//! the argmax of these mixed scores, with a `Tank` carve-out for the
//! "old + losing + no star" pattern (punted year, not a real rebuild).

use crate::weights::TeamContextWeights;
use crate::Reason;
use nba3k_core::{LeagueSnapshot, Player, TeamId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TeamMode {
    FullRebuild,
    SoftRebuild,
    Retool,
    Contend,
    Tank,
}

#[derive(Debug, Clone)]
pub struct TeamContext {
    pub mode: TeamMode,
    pub contend_score: f32,
    pub rebuild_score: f32,
    pub win_now_pressure: f32,
    pub reasons: Vec<Reason>,
}

/// Top-K of roster (by OVR) used as the rotation slice for age + keeper
/// counts. Mirrors the M3 trade-engine choice so behavior matches.
const ROTATION_TOP_K: usize = 9;

/// "Strong seed" cutoff — conf rank 1..=8 counts as a real playoff seed.
const PLAYOFF_RANK: u8 = 8;

/// Bottom-of-standings cutoff for tank / rebuild signals.
const BOTTOM_RANK: u8 = 11;

/// Win-pct floor that triggers the "punted the year" tank when paired with
/// veteran roster + no star.
const TANK_WIN_PCT: f32 = 0.35;

/// Win-pct ceiling that triggers the contend signal.
const CONTEND_WIN_PCT: f32 = 0.55;

pub fn team_context(
    team: TeamId,
    league: &LeagueSnapshot,
    weights: &TeamContextWeights,
) -> TeamContext {
    let roster = league.roster(team);
    let record = league.record(team);
    let games_played = record.games_played();

    let (avg_age, top_ovr, keeper_count, star_count) = rotation_profile(&roster, weights);

    let win_pct = if games_played >= 10 {
        Some(record.win_pct())
    } else {
        None
    };
    let conf_rank = record.conf_rank;

    // ---- Component signals (each in [0.0, 1.0], either pro-contend or pro-rebuild).

    // top_ovr_signal: how loud is the star presence? Above the star OVR
    // threshold maps to ~1.0; below the keeper threshold drops fast.
    let top_ovr_signal = ramp(
        top_ovr as f32,
        weights.keeper_ovr as f32,
        weights.star_ovr as f32,
    );

    // roster_age_signal: young (≤ young_threshold) → 1.0 (rebuild flavour);
    // veteran (≥ veteran_threshold) → 0.0 (contend flavour). Linear in between.
    let roster_age_signal = inverse_ramp(
        avg_age,
        weights.young_age_threshold,
        weights.veteran_age_threshold,
    );

    // standings_signal: in [0.0, 1.0]. 1.0 = strong contender pace, 0.0 = bottom feeder.
    let standings_signal = match (win_pct, conf_rank) {
        (Some(p), _) => smoothstep(p, 0.30, 0.65),
        (None, r) if r > 0 && r <= PLAYOFF_RANK => 0.7,
        (None, r) if r >= BOTTOM_RANK => 0.2,
        _ => 0.5,
    };

    // cap_commitment_signal: 1.0 = lots committed (locked-in roster, contend
    // signal), 0.0 = wide-open books (rebuild signal). Proxy via keeper count
    // until full cap data is wired through (M5+).
    let cap_commitment_signal = (keeper_count as f32 / ROTATION_TOP_K as f32).clamp(0.0, 1.0);

    // recent_history_signal: M5+ (no historical data yet). Neutral 0.5.
    let recent_history_signal = 0.5_f32;

    // ---- Mix into the headline scores.
    //
    // contend_score pulls from: top_ovr, standings, cap_commitment, low age weight.
    // rebuild_score pulls from: youth, low standings, low cap commitment, lack of stars.
    let contend_score = (0.35 * top_ovr_signal
        + 0.30 * standings_signal
        + 0.20 * cap_commitment_signal
        + 0.10 * (1.0 - roster_age_signal)
        + 0.05 * recent_history_signal)
        .clamp(0.0, 1.0);

    let rebuild_score = (0.35 * roster_age_signal
        + 0.30 * (1.0 - standings_signal)
        + 0.20 * (1.0 - cap_commitment_signal)
        + 0.15 * (1.0 - top_ovr_signal))
        .clamp(0.0, 1.0);

    // win_now_pressure: how much does this team need to act NOW?
    // High when contender-shaped AND aging — Phoenix-with-KD energy.
    // Low when young or already losing.
    let win_now_pressure =
        (0.5 * top_ovr_signal + 0.4 * (1.0 - roster_age_signal) + 0.1 * standings_signal)
            .clamp(0.0, 1.0);

    // ---- Discrete classification. Order matters: contender first, hard-tank
    // carve-out before falling through to rebuild branches, retool as the
    // honest "neither" bucket.
    let losing = win_pct.map(|p| p <= TANK_WIN_PCT).unwrap_or(false);
    let competing = win_pct.map(|p| p >= CONTEND_WIN_PCT).unwrap_or(false);
    let strong_seed = conf_rank > 0 && conf_rank <= PLAYOFF_RANK;

    let has_star = star_count >= 1;
    let young = avg_age <= weights.young_age_threshold;
    let veteran = avg_age >= weights.veteran_age_threshold;

    let mode = if has_star && (competing || strong_seed) {
        TeamMode::Contend
    } else if veteran && losing && !has_star {
        TeamMode::Tank
    } else if young && keeper_count <= 1 && (losing || !strong_seed) {
        TeamMode::FullRebuild
    } else if young && keeper_count >= 2 {
        TeamMode::SoftRebuild
    } else {
        TeamMode::Retool
    };

    // ---- Reasons. Each component contributes one entry whose `delta` is the
    // signed pull on contend_score (positive raises contend, negative raises
    // rebuild). Order is established by Score::sort_reasons at the call site.
    let reasons = vec![
        Reason {
            label: "top_ovr_signal",
            delta: (top_ovr_signal - 0.5) as f64,
        },
        Reason {
            label: "roster_age_signal",
            delta: -((roster_age_signal - 0.5) as f64),
        },
        Reason {
            label: "standings_signal",
            delta: (standings_signal - 0.5) as f64,
        },
        Reason {
            label: "cap_commitment_signal",
            delta: (cap_commitment_signal - 0.5) as f64,
        },
        Reason {
            label: "recent_history_signal",
            delta: 0.0,
        },
    ];

    TeamContext {
        mode,
        contend_score,
        rebuild_score,
        win_now_pressure,
        reasons,
    }
}

/// Slice the roster down to the top-K-by-OVR rotation and compute
/// (avg_age, top_ovr, keeper_count, star_count). Empty roster falls back to
/// neutral-shaped defaults so an unseeded team doesn't blow up the model.
fn rotation_profile(roster: &[&Player], weights: &TeamContextWeights) -> (f32, u8, usize, usize) {
    if roster.is_empty() {
        return (27.0, 0, 0, 0);
    }

    let mut by_ovr: Vec<&Player> = roster.to_vec();
    by_ovr.sort_by(|a, b| b.overall.cmp(&a.overall));
    let rotation: Vec<&Player> = by_ovr.iter().take(ROTATION_TOP_K).copied().collect();

    let avg_age = if rotation.is_empty() {
        27.0
    } else {
        let sum: u32 = rotation.iter().map(|p| p.age as u32).sum();
        sum as f32 / rotation.len() as f32
    };

    let top_ovr = rotation.first().map(|p| p.overall).unwrap_or(0);
    let keeper_count = rotation
        .iter()
        .filter(|p| p.overall >= weights.keeper_ovr)
        .count();
    let star_count = rotation
        .iter()
        .filter(|p| p.overall >= weights.star_ovr)
        .count();

    (avg_age, top_ovr, keeper_count, star_count)
}

/// Linear ramp: value below `lo` → 0.0, above `hi` → 1.0, linear in between.
fn ramp(value: f32, lo: f32, hi: f32) -> f32 {
    if hi <= lo {
        return if value >= hi { 1.0 } else { 0.0 };
    }
    ((value - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Inverse ramp: value below `lo` → 1.0, above `hi` → 0.0.
fn inverse_ramp(value: f32, lo: f32, hi: f32) -> f32 {
    1.0 - ramp(value, lo, hi)
}

/// Smoothstep on `[lo, hi]` — 0 below `lo`, 1 above `hi`, S-curve in between.
fn smoothstep(value: f32, lo: f32, hi: f32) -> f32 {
    if hi <= lo {
        return if value >= hi { 1.0 } else { 0.0 };
    }
    let t = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
