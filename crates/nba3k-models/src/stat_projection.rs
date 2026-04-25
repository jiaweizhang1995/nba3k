//! Worker C — per-player stat projection model.
//!
//! Replaces the per-game box-score distribution in
//! `nba3k_sim::engine::statistical`. Stars produce star lines.
//! Triple-doubles emerge from high-usage primary creators with
//! rebound bonuses, not random rolls.
//!
//! Pipeline per player:
//!   1. Resolve archetype profile (per-100-team-possessions baselines).
//!   2. Apply star uplift if the player is franchise-tagged.
//!   3. Apply primary-creator REB + AST bonus tied to usage above 0.25.
//!   4. Compute on-court team-possessions and a usage scaling factor.
//!   5. Sample each box-score stat from a normal with mean = baseline ×
//!      possession scale × stat-specific factor and a hybrid sigma that
//!      reads as Poisson-floor at low means and superdispersed at high
//!      means (real basketball is noisy).
//!   6. Reconcile FG made / 3PT made / FT made to the sampled PTS so the
//!      box arithmetic remains consistent.
//!
//! See `phases/M4-realism.md` "Worker C" for the full spec.

use crate::star_protection::StarRoster;
use crate::weights::StatProjectionWeights;
use chrono::NaiveDate;
use nba3k_core::{Player, PlayerLine, Position};
use rand::RngCore;
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-archetype profile loaded from `data/archetype_profiles.toml`.
/// Each row pins per-100-team-possession rates and a "default usage" so the
/// engine can scale up/down for players who run heavier or lighter loads
/// than the archetype prototype.
#[derive(Debug, Clone, Default)]
pub struct ArchetypeProfiles {
    pub by_archetype: HashMap<String, ArchetypeProfile>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ArchetypeProfile {
    /// Typical usage rate for this archetype. Players who run a higher usage
    /// than this scale up usage-driven stats; lower scales down.
    #[serde(default = "default_usage_default")]
    pub default_usage: f32,
    pub pts_per_100: f32,
    pub reb_per_100: f32,
    pub ast_per_100: f32,
    pub stl_per_100: f32,
    pub blk_per_100: f32,
    pub tov_per_100: f32,
    pub three_pa_per_100: f32,
    pub fta_per_100: f32,
}

fn default_usage_default() -> f32 {
    0.20
}

impl Default for ArchetypeProfile {
    fn default() -> Self {
        // Neutral fallback used when the file is absent or the archetype is
        // unknown. Tuned to produce mid-rotation role-player lines (not zero,
        // not stars) so downstream callers never see panics or empty boxes.
        Self {
            default_usage: 0.20,
            pts_per_100: 18.0,
            reb_per_100: 6.0,
            ast_per_100: 3.0,
            stl_per_100: 1.0,
            blk_per_100: 0.5,
            tov_per_100: 2.0,
            three_pa_per_100: 4.5,
            fta_per_100: 3.5,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StatProjectionInput<'a> {
    pub player: &'a Player,
    pub minutes: u8,
    pub team_pace: f32,
    pub usage_share: f32,
    pub archetype: &'a str,
    pub date: NaiveDate,
    /// The team this projection is for. Used to look up franchise-tag uplift
    /// in the star roster. The roster is keyed by team abbreviation, so the
    /// caller must pass the abbrev (e.g. "LAL").
    pub team_abbrev: &'a str,
}

/// Load archetype profiles from a TOML file. Missing file → empty profile
/// map (caller falls back to a neutral baseline). Malformed file → error.
pub fn load_archetype_profiles(path: &std::path::Path) -> crate::ModelResult<ArchetypeProfiles> {
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ArchetypeProfiles::default());
        }
        Err(e) => return Err(e.into()),
    };
    let parsed: HashMap<String, ArchetypeProfile> = toml::from_str(&text)?;
    Ok(ArchetypeProfiles { by_archetype: parsed })
}

/// Default location of the archetype profile TOML, relative to the workspace.
pub const ARCHETYPE_PROFILES_PATH: &str = "data/archetype_profiles.toml";

/// Generate a single-game `PlayerLine` for one player. Pure: same input +
/// same RNG state always yields the same line.
pub fn project_player_line(
    input: StatProjectionInput,
    profiles: &ArchetypeProfiles,
    weights: &StatProjectionWeights,
    roster: &StarRoster,
    rng: &mut dyn RngCore,
) -> PlayerLine {
    let StatProjectionInput {
        player,
        minutes,
        team_pace,
        usage_share,
        archetype,
        team_abbrev,
        date: _,
    } = input;

    // ---- Resolve archetype profile (or sane neutral fallback).
    let base = profiles
        .by_archetype
        .get(archetype)
        .copied()
        .unwrap_or_default();

    // ---- Star uplift: franchise-tagged players get a real boost. The
    // boost is on top of the archetype baseline so a tagged Luka stacks
    // well above an untagged starter at the same OVR. We also gate by an
    // OVR threshold so the file never accidentally over-promotes a faded
    // veteran whose name still happens to be on the list.
    let is_franchise = roster.is_tagged(team_abbrev, &player.name);
    let meets_star_ovr = player.overall >= weights.star_uplift_threshold_ovr;
    let star_active = is_franchise && meets_star_ovr;

    let mut pts_rate = base.pts_per_100;
    let mut reb_rate = base.reb_per_100;
    let mut ast_rate = base.ast_per_100;
    let stl_rate = base.stl_per_100;
    let blk_rate = base.blk_per_100;
    let tov_rate = base.tov_per_100;
    let three_pa_rate = base.three_pa_per_100;
    let fta_rate = base.fta_per_100;

    if star_active {
        pts_rate += weights.star_uplift_pts;
        reb_rate += weights.star_uplift_reb;
        ast_rate += weights.star_uplift_ast;
    }

    // ---- Primary-creator triple-double bonus.
    // High-usage initiators get an extra REB + AST bump scaled by usage
    // above 0.25. This is the single mechanic that makes triple-doubles
    // emerge from creators (Luka, Jokic, Giannis) and not from spot-up
    // wings — exactly the user's stated complaint with M3.
    let creator_usage_excess = (usage_share - 0.25).max(0.0);
    if creator_usage_excess > 0.0 {
        reb_rate += creator_usage_excess * weights.creator_reb_bonus_per_excess;
        ast_rate += creator_usage_excess * weights.creator_ast_bonus_per_excess;
    }

    // ---- Possession + usage scaling.
    let on_court_team_poss = (team_pace.max(60.0) * (minutes as f32 / 48.0)).max(0.0);
    let poss_scale = on_court_team_poss / 100.0;

    let usage_factor = if base.default_usage > 0.01 {
        (usage_share.max(0.01) / base.default_usage).clamp(0.05, 3.0).powf(0.8)
    } else {
        1.0
    };

    // ---- Injury throttle. If the player is dinged, scale all means down.
    // Day-to-day = light haircut, season-ending = tiny minutes cap. The
    // sim engine already culls truly injured players, but we double-check
    // here so a forced projection never returns a star line for a player
    // listed as "out".
    let injury_scale = injury_scale_factor(player);

    // ---- Sample means. Variance is hybrid: max(sqrt(mean) * 1.2, mean * 0.30).
    // At low means (4 PTS) sigma ≈ 2.4 → many zeros. At high means (32 PTS)
    // sigma ≈ 9.6 → real game-to-game volatility. This is what makes the
    // triple-double rate emerge naturally from the right players.
    let mean_pts = pts_rate * poss_scale * usage_factor * injury_scale;
    let mean_ast = ast_rate * poss_scale * usage_factor * injury_scale;
    let mean_tov = tov_rate * poss_scale * usage_factor * injury_scale;
    let mean_three_pa = three_pa_rate * poss_scale * usage_factor * injury_scale;
    let mean_fta = fta_rate * poss_scale * usage_factor * injury_scale;
    let mean_reb = reb_rate * poss_scale * injury_scale;
    let mean_stl = stl_rate * poss_scale * injury_scale;
    let mean_blk = blk_rate * poss_scale * injury_scale;

    // Sample each stat. Order matters for determinism — keep stable.
    let pts = sample_count(mean_pts, rng).min(80);
    let reb = sample_count(mean_reb, rng).min(35);
    let ast = sample_count(mean_ast, rng).min(25);
    let stl = sample_count(mean_stl, rng).min(10);
    let blk = sample_count(mean_blk, rng).min(10);
    let tov = sample_count(mean_tov, rng).min(15);
    let three_att_sampled = sample_count(mean_three_pa, rng).min(25);
    let ft_att_sampled = sample_count(mean_fta, rng).min(25);

    // ---- Reconcile shooting line so total points equal the sampled PTS.
    // Approach:
    //   1. FT make rate from finishing rating (proxy for free-throw skill —
    //      we don't have a dedicated FT rating in core::Ratings).
    //   2. Pick FT made from a binomial-ish around mean_ft_make.
    //   3. Remaining points come from FG. Split into 3PT and 2PT given
    //      the player's 3PT shooting rating.
    //   4. Sanity-clamp so all components fit u8.

    let ft_make_rate = ((player.ratings.mid_range as f32 + 50.0) / 99.0)
        .clamp(0.55, 0.92);
    let ft_made = binomial_count(ft_att_sampled, ft_make_rate, rng).min(ft_att_sampled);
    let ft_pts = ft_made as u16;

    let needed_field_pts = (pts as u16).saturating_sub(ft_pts);

    // 3PT shooting volume target (already sampled). Given baseline make rate
    // for the player.
    let three_make_rate = (player.ratings.three_point as f32 / 99.0)
        .clamp(0.20, 0.55) * 0.65 + 0.05;
    let three_made_target = binomial_count(three_att_sampled, three_make_rate, rng);

    // First fit 3-point points within needed_field_pts (each 3 = 3 pts).
    let max_3_for_pts = (needed_field_pts / 3) as u16;
    let three_made = (three_made_target as u16).min(max_3_for_pts).min(255) as u8;
    let three_pts = (three_made as u16) * 3;

    // Remainder from 2-point makes.
    let two_pts_remaining = needed_field_pts.saturating_sub(three_pts);
    let two_made = (two_pts_remaining / 2).min(255) as u8;
    let two_pts = (two_made as u16) * 2;

    // It's possible PTS is odd → we lose 1 pt to integer division. Patch
    // that by adding a free throw (most common single-point scenario in
    // real box scores) if we have headroom.
    let actual_pts = ft_pts + three_pts + two_pts;
    let mut ft_made = ft_made;
    let mut ft_att = ft_att_sampled.max(ft_made);
    let pts_shortfall = (pts as u16).saturating_sub(actual_pts);
    if pts_shortfall > 0 {
        let extra_ft = pts_shortfall.min(255) as u8;
        ft_made = ft_made.saturating_add(extra_ft);
        ft_att = ft_att.saturating_add(extra_ft);
    }

    // FG attempts: 3PA fixed by sample; 2PA from missed makes plus the
    // makes themselves. Estimate via player's general FG% so the volume
    // looks reasonable.
    let two_make_rate = ((player.ratings.mid_range as f32 * 0.40
        + player.ratings.driving_layup as f32 * 0.60) / 99.0)
        .clamp(0.30, 0.70);
    let two_att = if two_made > 0 {
        ((two_made as f32 / two_make_rate.max(0.30)).round() as u16).min(40) as u8
    } else if two_make_rate > 0.0 {
        // Player attempted some 2s without making any — keep volume realistic.
        let mean_two_att = mean_pts / 2.4;
        let mut s = sample_count(mean_two_att, rng);
        s = s.saturating_sub(three_att_sampled);
        s.min(20)
    } else {
        0
    };

    let three_att = three_att_sampled.max(three_made);
    let fg_made = three_made.saturating_add(two_made);
    let fg_att = three_att.saturating_add(two_att);

    PlayerLine {
        player: player.id,
        minutes: minutes,
        pts: (ft_pts + three_pts + two_pts).min(u8::MAX as u16) as u8,
        reb,
        ast,
        stl,
        blk,
        tov,
        fg_made,
        fg_att: fg_att.max(fg_made),
        three_made,
        three_att,
        ft_made,
        ft_att: ft_att.max(ft_made),
        plus_minus: 0, // assigned by the sim engine post-hoc from team diff
    }
}

/// Decide a player's archetype from their position + ratings spread.
/// Used by the sim engine when the upstream data hasn't tagged the player.
pub fn infer_archetype(player: &Player) -> String {
    let r = &player.ratings;
    let three = r.three_point as i32;
    let make = r.ball_handle as i32;
    let finish = r.driving_layup as i32;
    let reb = (r.off_reb as i32 + r.def_reb as i32) / 2;
    let pass = r.passing_accuracy as i32;
    let def_int = r.interior_defense as i32;
    let def_per = r.perimeter_defense as i32;

    match player.primary_position {
        Position::PG => {
            // Distributor needs both high passing AND high ball_handle —
            // a primary creator. Curry (high three_point, mediocre passing)
            // falls through to PG-scorer, which is correct.
            if pass >= 80 && make >= 75 {
                "PG-distributor".to_string()
            } else {
                "PG-scorer".to_string()
            }
        }
        Position::SG => {
            // Shooter vs slasher: 3PT-heavy or finishing-heavy?
            if three >= finish + 4 {
                "SG-shooter".to_string()
            } else {
                "SG-slasher".to_string()
            }
        }
        Position::SF => {
            // Creator (high playmaking) vs 3-and-D (high defense + 3PT, low make).
            if make >= 78 || (finish >= 80 && make >= 70) {
                "SF-creator".to_string()
            } else {
                "SF-3andD".to_string()
            }
        }
        Position::PF => {
            // Stretch (3PT shooter) vs banger (rebound + paint defense).
            if three >= 70 {
                "PF-stretch".to_string()
            } else if reb >= def_per || def_int >= 75 {
                "PF-banger".to_string()
            } else {
                "PF-stretch".to_string()
            }
        }
        Position::C => {
            // Stretch (3PT) vs finisher (paint + boards + blocks).
            if three >= 65 {
                "C-stretch".to_string()
            } else {
                "C-finisher".to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internals — sampling helpers.
// ---------------------------------------------------------------------------

/// Sample a non-negative count from a normal-with-Poisson-floor variance
/// model, then truncate to u8. Mean ≈ `mean`; sigma blended between Poisson
/// (sqrt scaling) at low means and proportional (CV ~30%) at high means.
fn sample_count(mean: f32, rng: &mut dyn RngCore) -> u8 {
    if mean <= 0.0 {
        return 0;
    }
    let poisson_sigma = mean.sqrt() * 1.2;
    let proportional_sigma = mean * 0.30;
    let sigma = poisson_sigma.max(proportional_sigma).max(0.5);
    let dist = Normal::new(mean as f64, sigma as f64).expect("valid sigma");
    let raw = dist.sample(rng) as f32;
    let clipped = raw.max(0.0).round();
    clipped.min(u8::MAX as f32) as u8
}

/// Sample a count of "successes" out of `trials` at a per-trial probability.
/// Uses normal approximation with continuity correction — good enough for
/// box-score arithmetic and dramatically faster than rejection sampling.
fn binomial_count(trials: u8, p: f32, rng: &mut dyn RngCore) -> u8 {
    if trials == 0 || p <= 0.0 {
        return 0;
    }
    if p >= 1.0 {
        return trials;
    }
    let n = trials as f32;
    let mean = n * p;
    let sigma = (n * p * (1.0 - p)).sqrt().max(0.4);
    let dist = Normal::new(mean as f64, sigma as f64).expect("valid sigma");
    let raw = dist.sample(rng) as f32;
    raw.round().clamp(0.0, n) as u8
}

/// Multiplier on all stat means based on injury status. Day-to-day shaves
/// 25% off; longer absences scale more aggressively. The sim engine should
/// already cull season-ending injuries from the rotation; this is a
/// belt-and-suspenders for any caller that forces a projection through
/// (e.g., previewing the box of a player listed as questionable).
fn injury_scale_factor(player: &Player) -> f32 {
    let Some(inj) = &player.injury else {
        return 1.0;
    };
    use nba3k_core::InjurySeverity::*;
    match inj.severity {
        DayToDay => 0.75,
        ShortTerm => 0.55,
        LongTerm => 0.30,
        SeasonEnding => 0.10,
    }
}
