//! Team-context classifier + trait modulation (Worker B).
//!
//! `classify_team` and `apply_context` together let the trade engine ask the
//! same GM the same question in different situations and get different
//! answers — Conservative-archetype Bulls in March of a tank year don't act
//! like Conservative-archetype Bulls on draft night of a contending year.
//!
//! Heuristic for `classify_team` (documented as constants below — tweak
//! these in the calibration harness, not here):
//!
//! - Average age of top-9 rotation by OVR.
//! - Top OVR on the roster (proxy for star presence).
//! - Conference rank from the live snapshot (0 means "unknown" — pre-season
//!   or before any games — and counts as neutral).
//! - Win pct.
//!
//! Output is one of `FullRebuild | SoftRebuild | Retool | Contend | Tank`.
//! `Tank` is reserved for explicit "we are losing on purpose" — in v1 we
//! collapse it into the rebuild branches and only flag it when standings
//! confirm a deep losing record without the youth profile of a real
//! rebuild (an old, expensive team that lost — i.e. punted the season).
//!
//! `apply_context` then maps `(TeamMode, SeasonPhase, NaiveDate)` to a
//! per-evaluation trait adjustment. Multipliers compose; exact factors live
//! at the top of the impl so they're easy to find.

use crate::{snapshot::LeagueSnapshot, TeamMode};
use chrono::{Duration, NaiveDate};
use nba3k_core::{GMTraits, SeasonPhase, TeamId};

/// 2025-26 NBA trade deadline. Mirrors `nba3k_season::phases::TRADE_DEADLINE`
/// — duplicated here as a `const fn`-friendly tuple so this crate doesn't
/// have to depend on nba3k-season.
pub const TRADE_DEADLINE: (i32, u32, u32) = (2026, 2, 5);

/// Days before the deadline that count as "pre-deadline" for context
/// modulation. Two weeks matches the spec.
pub const PRE_DEADLINE_WINDOW_DAYS: i64 = 14;

// ----- Thresholds for `classify_team`. Tune in calibration, not at call site.

/// Top-K of roster (by OVR) used as the "rotation" age proxy.
const ROTATION_TOP_K: usize = 9;

/// A clearly-young roster: average rotation age ≤ this.
const YOUNG_AGE_THRESHOLD: f32 = 25.0;

/// A clearly-veteran roster: average rotation age ≥ this.
const VETERAN_AGE_THRESHOLD: f32 = 29.0;

/// Star-tier OVR — a roster with at least one player >= this is plausibly
/// a contender or retool, not a true rebuild.
const STAR_OVR_THRESHOLD: u8 = 88;

/// Above-average OVR — used to distinguish soft-rebuild ("a couple keepers")
/// from full rebuild ("everyone's a project").
const KEEPER_OVR_THRESHOLD: u8 = 82;

/// Top-half conference rank ≤ this is treated as a real playoff seed.
const PLAYOFF_RANK: u8 = 8;

/// A losing record at this win-pct or below counts as "bottom of the
/// standings" for tank/rebuild detection.
const BOTTOM_TIER_WIN_PCT: f32 = 0.35;

/// At-or-above this win-pct counts as "competing now" — fuels Contend/Retool.
const CONTEND_TIER_WIN_PCT: f32 = 0.55;

pub fn classify_team(team: TeamId, snap: &LeagueSnapshot) -> TeamMode {
    let roster = snap.roster(team);
    let record = snap.record(team);
    let games_played = record.games_played();

    let (avg_age, top_ovr, keeper_count) = age_and_ovr_profile(&roster);

    // Pre-season / no games yet — fall back to roster shape only.
    let win_pct = if games_played >= 10 {
        Some(record.win_pct())
    } else {
        None
    };
    let conf_rank = record.conf_rank;

    let losing = match win_pct {
        Some(p) => p <= BOTTOM_TIER_WIN_PCT,
        None => false,
    };
    let competing = match win_pct {
        Some(p) => p >= CONTEND_TIER_WIN_PCT,
        None => false,
    };
    let strong_seed = conf_rank > 0 && conf_rank <= PLAYOFF_RANK;

    let has_star = top_ovr >= STAR_OVR_THRESHOLD;
    let young = avg_age <= YOUNG_AGE_THRESHOLD;
    let veteran = avg_age >= VETERAN_AGE_THRESHOLD;

    // Decision tree. Order matters: contender first, full rebuild second.
    if has_star && (competing || strong_seed) {
        return TeamMode::Contend;
    }

    if veteran && losing {
        // Old, expensive, losing — classic "punted the year" tank.
        return TeamMode::Tank;
    }

    if young && keeper_count <= 1 && (losing || !strong_seed) {
        return TeamMode::FullRebuild;
    }

    if young && keeper_count >= 2 {
        return TeamMode::SoftRebuild;
    }

    // Everyone else — neither clearly contending nor clearly tearing down.
    TeamMode::Retool
}

fn age_and_ovr_profile(roster: &[&nba3k_core::Player]) -> (f32, u8, usize) {
    if roster.is_empty() {
        return (27.0, 0, 0);
    }

    let mut by_ovr: Vec<&nba3k_core::Player> = roster.to_vec();
    by_ovr.sort_by(|a, b| b.overall.cmp(&a.overall));
    let rotation: Vec<&nba3k_core::Player> = by_ovr.iter().take(ROTATION_TOP_K).copied().collect();

    let avg_age = if rotation.is_empty() {
        27.0
    } else {
        let sum: u32 = rotation.iter().map(|p| p.age as u32).sum();
        sum as f32 / rotation.len() as f32
    };

    let top_ovr = rotation.first().map(|p| p.overall).unwrap_or(0);
    let keeper_count = rotation
        .iter()
        .filter(|p| p.overall >= KEEPER_OVR_THRESHOLD)
        .count();

    (avg_age, top_ovr, keeper_count)
}

/// Adjust `traits` for a specific `(mode, phase, date)` situation. Multipliers
/// only — never zero a field out, never invert a sign.
///
/// The factors below are deliberately simple — calibration tunes the leaf
/// numbers, not the structure.
pub fn apply_context(
    traits: &GMTraits,
    mode: TeamMode,
    phase: SeasonPhase,
    date: NaiveDate,
) -> GMTraits {
    let mut adjusted = *traits;

    // ----- Mode-driven shifts.
    match mode {
        TeamMode::Contend => {
            adjusted.current_overall_weight *= 1.5;
            adjusted.potential_weight *= 0.6;
            adjusted.pick_value_multiplier *= 0.75;
            adjusted.patience *= 0.5;
            adjusted.star_premium *= 1.15;
        }
        TeamMode::Retool => {
            adjusted.current_overall_weight *= 1.1;
            adjusted.potential_weight *= 0.9;
        }
        TeamMode::SoftRebuild => {
            adjusted.current_overall_weight *= 0.85;
            adjusted.potential_weight *= 1.25;
            adjusted.pick_value_multiplier *= 1.2;
            adjusted.patience = (adjusted.patience * 1.4).min(1.0);
        }
        TeamMode::FullRebuild => {
            adjusted.current_overall_weight *= 0.6;
            adjusted.potential_weight *= 1.5;
            adjusted.pick_value_multiplier *= 1.4;
            adjusted.patience = (adjusted.patience.max(0.6) * 1.5).min(1.0);
        }
        TeamMode::Tank => {
            // Old + losing: the FO probably accepts the tank but won't
            // sacrifice as much for picks as a true rebuilder.
            adjusted.current_overall_weight *= 0.8;
            adjusted.pick_value_multiplier *= 1.25;
            adjusted.patience = (adjusted.patience * 1.2).min(1.0);
        }
    }

    // ----- Phase / date-driven shifts.
    if matches!(phase, SeasonPhase::OffSeason | SeasonPhase::FreeAgency) {
        // Off-season: longer planning horizon, more patience for any team
        // that wasn't already running max patience.
        adjusted.patience = (adjusted.patience * 1.25).min(1.0);
        if matches!(
            mode,
            TeamMode::SoftRebuild | TeamMode::FullRebuild | TeamMode::Tank
        ) {
            adjusted.patience = adjusted.patience.max(0.9);
        }
    }

    if is_pre_deadline(date) {
        match mode {
            TeamMode::Contend => {
                // Buyer urgency — looser on risk, willing to overpay.
                adjusted.risk_tolerance = (adjusted.risk_tolerance * 1.4).clamp(0.0, 1.0);
                adjusted.patience *= 0.7;
                adjusted.aggression = (adjusted.aggression * 1.2).min(1.0);
            }
            TeamMode::Tank | TeamMode::FullRebuild | TeamMode::SoftRebuild => {
                // Sellers — more flexible on perceived value (the spec
                // models lower asking price as a small gullibility bump).
                adjusted.gullibility = (adjusted.gullibility * 1.2 + 0.05).min(1.0);
            }
            TeamMode::Retool => {
                // Mild willingness to move on the margins.
                adjusted.risk_tolerance = (adjusted.risk_tolerance * 1.15).clamp(0.0, 1.0);
            }
        }
    }

    adjusted
}

fn is_pre_deadline(date: NaiveDate) -> bool {
    let Some(deadline) =
        NaiveDate::from_ymd_opt(TRADE_DEADLINE.0, TRADE_DEADLINE.1, TRADE_DEADLINE.2)
    else {
        return false;
    };
    if date > deadline {
        return false;
    }
    let window_start = deadline - Duration::days(PRE_DEADLINE_WINDOW_DAYS);
    date >= window_start
}
