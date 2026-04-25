use crate::params::SimParams;
use crate::{Engine, GameContext, RotationSlot, TeamSnapshot};
use nba3k_core::{BoxScore, GameResult, PlayerId, PlayerLine};
use rand::{Rng, RngCore};
use rand_distr::{Distribution, Normal};

/// Statistical (non play-by-play) game simulator. ~1ms per game.
#[derive(Debug, Clone)]
pub struct StatisticalEngine {
    params: SimParams,
}

impl StatisticalEngine {
    pub fn with_defaults() -> Self {
        Self { params: SimParams::default() }
    }

    pub fn with_params(params: SimParams) -> Self {
        Self { params }
    }

    pub fn params(&self) -> &SimParams {
        &self.params
    }
}

/// Team-level derived numbers from a TeamSnapshot.
#[derive(Debug, Clone, Copy)]
struct TeamProfile {
    ortg: f32,
    drtg: f32,
    pace: f32,
}

const BASE_ORTG: f32 = 108.0;
const BASE_DRTG: f32 = 108.0;
const FALLBACK_OVERALL: f32 = 75.0;
const REG_MINUTES: u16 = 240;
const OT_MINUTES: u16 = 25; // 5 min × 5 players

fn derive_profile(team: &TeamSnapshot, base_pace: f32) -> TeamProfile {
    if team.rotation.is_empty() {
        // Fall back to overall-only sim. Each rating point = ~0.4 ORtg.
        let delta = (team.overall as f32 - FALLBACK_OVERALL) * 0.4;
        return TeamProfile {
            ortg: BASE_ORTG + delta,
            drtg: BASE_DRTG - delta,
            pace: base_pace,
        };
    }

    let total_minutes_share: f32 = team.rotation.iter().map(|r| r.minutes_share).sum();
    let norm = if total_minutes_share > 0.0 { total_minutes_share } else { 1.0 };

    let mut off_acc = 0.0f32;
    let mut def_acc = 0.0f32;
    let mut pace_acc = 0.0f32;
    for slot in &team.rotation {
        let w = slot.minutes_share / norm;
        let r = &slot.ratings;
        // Offensive contribution: shooting + finishing + playmaking.
        // (No IQ in the 21-attribute schema — passing_accuracy substitutes.)
        let off_score = (r.three_point as f32 * 0.30
            + r.mid_range as f32 * 0.15
            + r.driving_layup as f32 * 0.20
            + r.ball_handle as f32 * 0.15
            + r.passing_accuracy as f32 * 0.20)
            - 70.0;
        // Defensive contribution: perimeter + interior + steal/block.
        let def_score = (r.perimeter_defense as f32 * 0.30
            + r.interior_defense as f32 * 0.30
            + r.steal as f32 * 0.20
            + r.block as f32 * 0.20)
            - 70.0;
        // Pace contribution: speed + agility (athleticism category).
        let pace_score = (r.speed as f32 + r.agility as f32) * 0.5 - 70.0;

        off_acc += w * off_score * 0.45;
        def_acc += w * def_score * 0.45;
        pace_acc += w * pace_score * 0.05;
    }
    TeamProfile {
        ortg: BASE_ORTG + off_acc,
        drtg: BASE_DRTG - def_acc,
        pace: base_pace + pace_acc,
    }
}

fn sample_possessions(home: &TeamProfile, away: &TeamProfile, sigma: f32, rng: &mut dyn RngCore) -> f32 {
    let combined = (home.pace + away.pace) * 0.5;
    let dist = Normal::new(combined as f64, sigma.max(0.1) as f64).expect("valid sigma");
    let raw = dist.sample(rng) as f32;
    raw.clamp(85.0, 120.0)
}

fn sample_score(
    own: &TeamProfile,
    opp: &TeamProfile,
    possessions: f32,
    hca: f32,
    sigma: f32,
    rng: &mut dyn RngCore,
) -> u16 {
    let expected_per_100 = own.ortg - opp.drtg + 100.0 + hca;
    let mean = expected_per_100 * possessions / 100.0;
    let dist = Normal::new(mean as f64, sigma.max(0.1) as f64).expect("valid sigma");
    let raw = dist.sample(rng) as f32;
    raw.round().clamp(60.0, 200.0) as u16
}

fn sample_ot_score(
    own: &TeamProfile,
    opp: &TeamProfile,
    hca: f32,
    sigma: f32,
    rng: &mut dyn RngCore,
) -> u16 {
    // 5-minute period ≈ 10.8 possessions per team.
    let possessions = 10.8f32;
    let expected_per_100 = own.ortg - opp.drtg + 100.0 + hca;
    let mean = expected_per_100 * possessions / 100.0;
    let dist = Normal::new(mean as f64, (sigma * 0.5).max(0.1) as f64).expect("valid sigma");
    let raw = dist.sample(rng) as f32;
    raw.round().clamp(0.0, 50.0) as u16
}

/// Distribute `total` integer units across `weights`, returning a Vec of u16
/// summing exactly to `total`. Uses largest-remainder method for stability.
fn distribute_u16(total: u16, weights: &[f32]) -> Vec<u16> {
    if weights.is_empty() {
        return vec![];
    }
    let sum_w: f32 = weights.iter().sum();
    if sum_w <= 0.0 {
        let mut out = vec![0u16; weights.len()];
        // Hand the whole bucket to the first slot to keep totals balanced.
        out[0] = total;
        return out;
    }
    let scaled: Vec<f32> = weights.iter().map(|w| (w / sum_w) * total as f32).collect();
    let mut floors: Vec<u16> = scaled.iter().map(|s| s.floor() as u16).collect();
    let assigned: u16 = floors.iter().sum();
    let mut remainders: Vec<(usize, f32)> = scaled
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s - s.floor()))
        .collect();
    remainders.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut leftover = total.saturating_sub(assigned);
    let mut idx = 0;
    while leftover > 0 && !remainders.is_empty() {
        let (i, _) = remainders[idx % remainders.len()];
        floors[i] += 1;
        leftover -= 1;
        idx += 1;
    }
    floors
}

fn distribute_u8(total: u16, weights: &[f32]) -> Vec<u8> {
    distribute_u16(total, weights).into_iter().map(|v| v.min(u8::MAX as u16) as u8).collect()
}

#[allow(clippy::too_many_arguments)]
fn build_lines(
    rotation: &[RotationSlot],
    fallback_id_base: u32,
    points: u16,
    available_team_minutes: u16,
    plus_minus: i16,
    rng: &mut dyn RngCore,
    team_abbrev: &str,
) -> Vec<PlayerLine> {
    if rotation.is_empty() {
        // Synthesize a single placeholder line. Used only when callers haven't
        // populated rotations (early bring-up / smoke tests).
        let line = PlayerLine {
            player: PlayerId(fallback_id_base),
            minutes: (available_team_minutes / 5).min(u8::MAX as u16) as u8,
            pts: points.min(u8::MAX as u16) as u8,
            reb: 0, ast: 0, stl: 0, blk: 0, tov: 0,
            fg_made: 0, fg_att: 0,
            three_made: 0, three_att: 0,
            ft_made: 0, ft_att: 0,
            plus_minus: plus_minus.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
        };
        return vec![line];
    }

    // Realism path: per-player line from `nba3k_models::stat_projection`.
    if !realism_resources::archetype_profiles().by_archetype.is_empty() {
        return build_lines_realism(rotation, points, available_team_minutes, plus_minus, team_abbrev, rng);
    }
    // Fallback to the M2 distribution arithmetic when archetype data is absent.

    let minutes_weights: Vec<f32> = rotation.iter().map(|r| r.minutes_share.max(0.0)).collect();
    let minutes = distribute_u16(available_team_minutes, &minutes_weights);

    // Usage drives points/shots distribution. Renormalize defensively.
    let mut usage_weights: Vec<f32> = rotation
        .iter()
        .zip(minutes_weights.iter())
        .map(|(r, m)| (r.usage.max(0.01)) * m.max(0.01))
        .collect();
    let usage_sum: f32 = usage_weights.iter().sum();
    if usage_sum <= 0.0 {
        usage_weights = vec![1.0; rotation.len()];
    }

    let pts_per_player = distribute_u8(points, &usage_weights);

    // Field-goal attempts ≈ points / 1.1 (rough TS conversion).
    let total_fg_att: u16 = ((points as f32) / 1.1).round() as u16;
    let fg_att_per = distribute_u8(total_fg_att, &usage_weights);

    // 3PA share by 3-point rating.
    let three_weights: Vec<f32> = rotation
        .iter()
        .zip(usage_weights.iter())
        .map(|(r, u)| (r.ratings.three_point as f32 / 99.0).powf(1.5) * u)
        .collect();
    let total_3p_att: u16 = ((total_fg_att as f32) * 0.40).round() as u16;
    let three_att_per = distribute_u8(total_3p_att, &three_weights);

    // Free-throw attempts ≈ 0.25 × FGA.
    let total_ft_att: u16 = ((total_fg_att as f32) * 0.25).round() as u16;
    let ft_att_per = distribute_u8(total_ft_att, &usage_weights);

    // Rebounds: 22 × possessions / 100 ≈ team total ~45. Scale by points
    // (proxy for game length) to keep box-score arithmetic well-behaved.
    let total_reb: u16 = ((points as f32) * 0.42).round() as u16;
    let reb_weights: Vec<f32> = rotation
        .iter()
        .map(|r| (r.ratings.off_reb as f32 + r.ratings.def_reb as f32) * 0.5)
        .collect();
    let reb_per = distribute_u8(total_reb, &reb_weights);

    // Assists: ≈ 60% of made FGs. Weighted by playmaking × minutes.
    let total_fg_made: u16 = ((points as f32) * 0.38).round() as u16;
    let total_ast: u16 = ((total_fg_made as f32) * 0.60).round() as u16;
    let ast_weights: Vec<f32> = rotation
        .iter()
        .zip(minutes_weights.iter())
        .map(|(r, m)| r.ratings.ball_handle as f32 * m)
        .collect();
    let ast_per = distribute_u8(total_ast, &ast_weights);

    // Steals & blocks: ~7.5 STL + 4.8 BLK per team per game baseline.
    let total_stl: u16 = ((points as f32) * 0.07).round() as u16;
    let stl_weights: Vec<f32> = rotation.iter().map(|r| r.ratings.perimeter_defense as f32).collect();
    let stl_per = distribute_u8(total_stl, &stl_weights);

    let total_blk: u16 = ((points as f32) * 0.045).round() as u16;
    let blk_weights: Vec<f32> = rotation.iter().map(|r| r.ratings.interior_defense as f32).collect();
    let blk_per = distribute_u8(total_blk, &blk_weights);

    // Turnovers: ~14 per team. Weighted inverse to IQ.
    let total_tov: u16 = ((points as f32) * 0.13).round() as u16;
    let tov_weights: Vec<f32> = rotation
        .iter()
        .zip(usage_weights.iter())
        // Higher passing_accuracy → fewer turnovers per usage. (No IQ field
        // in 21-attribute schema; passing_accuracy is the closest analogue.)
        .map(|(r, u)| (110.0 - r.ratings.passing_accuracy as f32).max(1.0) * u)
        .collect();
    let tov_per = distribute_u8(total_tov, &tov_weights);

    let mut lines = Vec::with_capacity(rotation.len());
    for (i, slot) in rotation.iter().enumerate() {
        let m = minutes[i].min(u8::MAX as u16) as u8;
        let pts = pts_per_player[i];

        // Per-player FG-made = player_pts / 1.1 floor (split between 2 and 3).
        let three_att = three_att_per[i].min(fg_att_per[i]);
        let fg_att = fg_att_per[i];
        // Make rates: weighted by ratings.
        let three_make_rate = (slot.ratings.three_point as f32 / 99.0).clamp(0.20, 0.55) * 0.65;
        let two_make_rate = (slot.ratings.mid_range as f32 * 0.4
            + slot.ratings.driving_layup as f32 * 0.6) / 99.0;
        let two_make_rate = two_make_rate.clamp(0.30, 0.70);
        // Slight noise so makes aren't deterministic per player profile.
        let noise: f32 = rng.gen_range(-0.05..0.05);

        let three_made = ((three_att as f32 * (three_make_rate + noise)).round() as i16)
            .clamp(0, three_att as i16) as u8;
        let two_att = fg_att.saturating_sub(three_att);
        let two_made = ((two_att as f32 * (two_make_rate + noise)).round() as i16)
            .clamp(0, two_att as i16) as u8;
        let fg_made = three_made.saturating_add(two_made);

        // FT made = whatever's needed to hit player's points; clamp to attempts.
        let scored_field = (two_made as u16) * 2 + (three_made as u16) * 3;
        let ft_att = ft_att_per[i];
        let ft_made_target = (pts as u16).saturating_sub(scored_field);
        let ft_made = ft_made_target.min(ft_att as u16) as u8;

        // Per-player +/- ≈ team diff × (minutes / 48).
        let pm_player = (plus_minus as f32 * (m as f32 / 48.0)).round() as i16;
        let pm_clamped = pm_player.clamp(i8::MIN as i16, i8::MAX as i16) as i8;

        lines.push(PlayerLine {
            player: slot.player,
            minutes: m,
            pts,
            reb: reb_per[i],
            ast: ast_per[i],
            stl: stl_per[i],
            blk: blk_per[i],
            tov: tov_per[i],
            fg_made,
            fg_att,
            three_made,
            three_att,
            ft_made,
            ft_att,
            plus_minus: pm_clamped,
        });
    }

    lines
}

/// Realism path — per-player box score generated by `nba3k_models::stat_projection`.
/// Each player's line is independently sampled from their archetype profile +
/// star uplift; we then PTS-reconcile to the simmed team score so box arithmetic
/// matches the headline result.
fn build_lines_realism(
    rotation: &[RotationSlot],
    team_score: u16,
    available_team_minutes: u16,
    plus_minus: i16,
    team_abbrev: &str,
    rng: &mut dyn RngCore,
) -> Vec<PlayerLine> {
    use nba3k_core::Player;
    use nba3k_models::stat_projection::{
        infer_archetype, project_player_line, StatProjectionInput,
    };

    let profiles = realism_resources::archetype_profiles();
    let weights = realism_resources::stat_projection_weights();
    let roster = realism_resources::star_roster();

    // Distribute team minutes across the rotation (sum = available_team_minutes).
    let minutes_weights: Vec<f32> = rotation.iter().map(|r| r.minutes_share.max(0.0)).collect();
    let minutes_per = distribute_u16(available_team_minutes, &minutes_weights);

    // team_pace = (team possessions per game) × (48 / team_minutes). In a 240-min
    // regulation game with ~100 possessions, that's ~100. Use a stable default.
    let team_pace = 100.0_f32;

    // Distribute usage shares so they sum to ~1.0 (one team's possessions).
    let usage_sum: f32 = rotation.iter().map(|r| r.usage.max(0.01)).sum();
    let usage_norm = if usage_sum > 0.0 { 1.0 / usage_sum } else { 1.0 };

    let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).expect("date");

    let mut lines = Vec::with_capacity(rotation.len());
    for (i, slot) in rotation.iter().enumerate() {
        let synthesized = synthesize_player(slot);
        let archetype = infer_archetype(&synthesized);
        let usage_share = (slot.usage.max(0.01) * usage_norm).clamp(0.05, 0.45);
        let minutes = minutes_per[i].min(u8::MAX as u16) as u8;

        let input = StatProjectionInput {
            player: &synthesized,
            minutes,
            team_pace,
            usage_share,
            archetype: &archetype,
            date,
            team_abbrev,
        };

        let mut line = project_player_line(input, profiles, weights, roster, rng);

        // +/- attributed by minutes share against team diff.
        let pm_player = (plus_minus as f32 * (minutes as f32 / 48.0)).round() as i16;
        line.plus_minus = pm_player.clamp(i8::MIN as i16, i8::MAX as i16) as i8;
        lines.push(line);
    }

    // Reconcile total PTS to the simmed `team_score`. Scale each player's
    // points proportionally; rebuild FG/FT made counts to match.
    let raw_total: u32 = lines.iter().map(|l| l.pts as u32).sum();
    if raw_total > 0 {
        let scale = team_score as f64 / raw_total as f64;
        let pts_cap = weights.single_game_pts_cap as f64;
        for line in &mut lines {
            let scaled = (line.pts as f64 * scale).round().min(pts_cap);
            let new_pts = scaled.clamp(0.0, 99.0) as u8;
            scale_points_inplace(line, new_pts);
        }
    } else if !lines.is_empty() {
        // No realism-side scoring (stars all 0 — shouldn't happen). Fall
        // through: dump the team score onto the highest-usage slot.
        let target_idx = rotation
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.usage.partial_cmp(&b.1.usage).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let pts = team_score.min(99) as u8;
        scale_points_inplace(&mut lines[target_idx], pts);
    }

    lines
}

/// Build a minimal `Player` carrying the fields `stat_projection` actually reads
/// (name, position, age, overall, potential, ratings, injury). All other fields
/// are sane defaults.
fn synthesize_player(slot: &RotationSlot) -> nba3k_core::Player {
    nba3k_core::Player {
        id: slot.player,
        name: slot.name.clone(),
        primary_position: slot.position,
        secondary_position: None,
        age: slot.age,
        overall: slot.overall,
        potential: slot.potential,
        ratings: slot.ratings,
        contract: None,
        team: None,
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: nba3k_core::PlayerRole::default(),
        morale: 0.5,
    }
}

/// Rewrite a PlayerLine's PTS + reconcile FG/FT counts to match.
fn scale_points_inplace(line: &mut PlayerLine, new_pts: u8) {
    line.pts = new_pts;
    // Keep three-attempts as-is; rebuild made counts to hit `new_pts`.
    let three_attempts_made = line.three_made.min(line.three_att);
    let three_pts = three_attempts_made as u16 * 3;
    let remaining = (new_pts as u16).saturating_sub(three_pts);
    // Two-pointers fill in next.
    let two_made = (remaining / 2).min(line.fg_att.saturating_sub(three_attempts_made) as u16) as u8;
    let two_pts = two_made as u16 * 2;
    let ft_pts = (new_pts as u16).saturating_sub(three_pts).saturating_sub(two_pts);
    let ft_made = ft_pts.min(line.ft_att as u16) as u8;
    line.fg_made = three_attempts_made.saturating_add(two_made);
    line.three_made = three_attempts_made;
    line.ft_made = ft_made;
}

mod realism_resources {
    //! Lazy-loaded archetype profiles + weights + star roster. Same pattern
    //! used by the trade engine — single load per process.

    use nba3k_models::stat_projection::{
        load_archetype_profiles, ArchetypeProfiles, ARCHETYPE_PROFILES_PATH,
    };
    use nba3k_models::star_protection::{load_star_roster, StarRoster, STAR_ROSTER_PATH};
    use nba3k_models::weights::{load_or_default, RealismWeights, StatProjectionWeights};
    use std::path::Path;
    use std::sync::OnceLock;

    pub const REALISM_WEIGHTS_PATH: &str = "data/realism_weights.toml";

    static ARCHETYPES: OnceLock<ArchetypeProfiles> = OnceLock::new();
    static STAR_ROSTER: OnceLock<StarRoster> = OnceLock::new();
    static WEIGHTS: OnceLock<RealismWeights> = OnceLock::new();

    pub fn archetype_profiles() -> &'static ArchetypeProfiles {
        ARCHETYPES.get_or_init(|| {
            load_archetype_profiles(Path::new(ARCHETYPE_PROFILES_PATH)).unwrap_or_default()
        })
    }

    pub fn star_roster() -> &'static StarRoster {
        STAR_ROSTER.get_or_init(|| {
            load_star_roster(Path::new(STAR_ROSTER_PATH)).unwrap_or_default()
        })
    }

    fn weights() -> &'static RealismWeights {
        WEIGHTS.get_or_init(|| {
            load_or_default(Path::new(REALISM_WEIGHTS_PATH)).unwrap_or_default()
        })
    }

    pub fn stat_projection_weights() -> &'static StatProjectionWeights {
        &weights().stat_projection
    }
}

fn apply_injury_rolls(
    rotation: &[RotationSlot],
    rate: f32,
    rng: &mut dyn RngCore,
) -> u32 {
    if rate <= 0.0 || rotation.is_empty() {
        return 0;
    }
    let mut count = 0u32;
    for _ in rotation {
        if rng.gen::<f32>() < rate {
            count += 1;
        }
    }
    count
}

impl Engine for StatisticalEngine {
    fn name(&self) -> &'static str {
        "statistical"
    }

    fn simulate_game(
        &self,
        home: &TeamSnapshot,
        away: &TeamSnapshot,
        ctx: &GameContext,
        rng: &mut dyn RngCore,
    ) -> GameResult {
        let p = &self.params;

        let home_profile = derive_profile(home, p.pace_mean);
        let away_profile = derive_profile(away, p.pace_mean);

        // B2B fatigue penalty: shave a touch off offense.
        let mut home_prof = home_profile;
        let mut away_prof = away_profile;
        if ctx.home_back_to_back {
            home_prof.ortg -= 1.5;
        }
        if ctx.away_back_to_back {
            away_prof.ortg -= 1.5;
        }

        let possessions = sample_possessions(&home_prof, &away_prof, p.pace_sigma, rng);

        let hca = if home.home_court_advantage > 0.0 {
            home.home_court_advantage
        } else {
            p.home_court_advantage
        };

        let mut home_score = sample_score(&home_prof, &away_prof, possessions, hca, p.score_sigma, rng);
        let mut away_score = sample_score(&away_prof, &home_prof, possessions, 0.0, p.score_sigma, rng);

        // OT recursion if tied. Cap retries to avoid runaway loops.
        let mut overtimes: u8 = 0;
        while home_score == away_score && overtimes < p.max_overtimes {
            let h_ot = sample_ot_score(&home_prof, &away_prof, hca * 0.25, p.score_sigma, rng);
            let a_ot = sample_ot_score(&away_prof, &home_prof, 0.0, p.score_sigma, rng);
            home_score = home_score.saturating_add(h_ot);
            away_score = away_score.saturating_add(a_ot);
            overtimes += 1;
        }
        // If still tied after max OTs, give the home team a single point
        // (deterministic tiebreak — extremely rare path).
        if home_score == away_score {
            home_score = home_score.saturating_add(1);
        }

        let team_minutes = REG_MINUTES + OT_MINUTES * overtimes as u16;
        let diff = home_score as i32 - away_score as i32;
        let home_pm = diff.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let away_pm = (-diff).clamp(i16::MIN as i32, i16::MAX as i32) as i16;

        let home_lines = build_lines(&home.rotation, 1, home_score, team_minutes, home_pm, rng, &home.abbrev);
        let away_lines = build_lines(&away.rotation, 2, away_score, team_minutes, away_pm, rng, &away.abbrev);

        // Injury rolls — currently surfaced only via the box (no struct slot).
        // Rolling here keeps the RNG stream advance deterministic per seed and
        // reserves the stream for the future injury-application pipeline.
        let _ = apply_injury_rolls(&home.rotation, p.injury_rate_per_game, rng);
        let _ = apply_injury_rolls(&away.rotation, p.injury_rate_per_game, rng);

        GameResult {
            id: ctx.game_id,
            season: ctx.season,
            date: ctx.date,
            home: home.id,
            away: away.id,
            home_score,
            away_score,
            box_score: BoxScore { home_lines, away_lines },
            overtime_periods: overtimes,
            is_playoffs: ctx.is_playoffs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RotationSlot, TeamSnapshot};
    use chrono::NaiveDate;
    use nba3k_core::{GameId, Position, Ratings, SeasonId, TeamId};
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn uniform_ratings(base: u8) -> Ratings {
        Ratings {
            close_shot: base, driving_layup: base, driving_dunk: base,
            standing_dunk: base, post_control: base,
            mid_range: base, three_point: base, free_throw: base,
            passing_accuracy: base, ball_handle: base, speed_with_ball: base,
            interior_defense: base, perimeter_defense: base, steal: base, block: base,
            off_reb: base, def_reb: base,
            speed: base, agility: base, strength: base, vertical: base,
        }
    }

    fn fair_team(id: u8, abbrev: &str, base: u8) -> TeamSnapshot {
        let ratings = uniform_ratings(base);
        let positions = [Position::PG, Position::SG, Position::SF, Position::PF, Position::C,
                         Position::PG, Position::SG, Position::C];
        let minutes_share = [1.0, 0.95, 0.95, 0.85, 0.85, 0.45, 0.45, 0.50];
        let usage = [0.22, 0.20, 0.18, 0.14, 0.14, 0.05, 0.04, 0.03];
        let rotation: Vec<RotationSlot> = (0..8)
            .map(|i| RotationSlot {
                player: nba3k_core::PlayerId(((id as u32) * 100) + i as u32),
                name: format!("{}{}", abbrev, i),
                position: positions[i],
                minutes_share: minutes_share[i],
                usage: usage[i],
                ratings,
                age: 27,
                overall: base,
                potential: base,
            })
            .collect();
        TeamSnapshot {
            id: TeamId(id),
            abbrev: abbrev.to_string(),
            overall: base,
            home_court_advantage: 2.0,
            rotation,
        }
    }

    fn ctx(seed_n: u64) -> GameContext {
        GameContext {
            game_id: GameId(seed_n),
            season: SeasonId(2026),
            date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
            is_playoffs: false,
            home_back_to_back: false,
            away_back_to_back: false,
        }
    }

    #[test]
    fn scores_in_range_and_minutes_invariant_1000_sims() {
        let engine = StatisticalEngine::with_defaults();
        let home = fair_team(1, "AAA", 75);
        let away = fair_team(2, "BBB", 75);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let c = ctx(1);

        let mut home_wins = 0u32;
        let mut away_wins = 0u32;
        for _ in 0..1000 {
            let r = engine.simulate_game(&home, &away, &c, &mut rng);
            assert!(r.home_score >= 60 && r.home_score <= 200, "home_score out of range: {}", r.home_score);
            assert!(r.away_score >= 60 && r.away_score <= 200, "away_score out of range: {}", r.away_score);
            assert_ne!(r.home_score, r.away_score, "tie should have been resolved");

            let expected_team_minutes = 240u32 + (r.overtime_periods as u32) * 25;
            let home_min: u32 = r.box_score.home_lines.iter().map(|l| l.minutes as u32).sum();
            let away_min: u32 = r.box_score.away_lines.iter().map(|l| l.minutes as u32).sum();
            assert_eq!(home_min, expected_team_minutes, "home minutes mismatch");
            assert_eq!(away_min, expected_team_minutes, "away minutes mismatch");

            if r.home_score > r.away_score { home_wins += 1; } else { away_wins += 1; }
        }

        // Loose home-advantage sanity bound: between 40% and 60%.
        let home_rate = home_wins as f32 / (home_wins + away_wins) as f32;
        assert!(home_rate >= 0.40 && home_rate <= 0.60,
            "home win rate {} out of [0.40, 0.60]", home_rate);
    }

    #[test]
    fn deterministic_same_seed_same_result() {
        let engine = StatisticalEngine::with_defaults();
        let home = fair_team(3, "CCC", 80);
        let away = fair_team(4, "DDD", 78);
        let c = ctx(7);

        let mut rng_a = ChaCha8Rng::seed_from_u64(1234);
        let mut rng_b = ChaCha8Rng::seed_from_u64(1234);
        let r_a = engine.simulate_game(&home, &away, &c, &mut rng_a);
        let r_b = engine.simulate_game(&home, &away, &c, &mut rng_b);

        assert_eq!(r_a.home_score, r_b.home_score);
        assert_eq!(r_a.away_score, r_b.away_score);
        assert_eq!(r_a.overtime_periods, r_b.overtime_periods);
        assert_eq!(r_a.box_score.home_lines.len(), r_b.box_score.home_lines.len());
        for (la, lb) in r_a.box_score.home_lines.iter().zip(r_b.box_score.home_lines.iter()) {
            assert_eq!(la.player, lb.player);
            assert_eq!(la.minutes, lb.minutes);
            assert_eq!(la.pts, lb.pts);
            assert_eq!(la.reb, lb.reb);
            assert_eq!(la.ast, lb.ast);
            assert_eq!(la.stl, lb.stl);
            assert_eq!(la.blk, lb.blk);
            assert_eq!(la.tov, lb.tov);
            assert_eq!(la.fg_made, lb.fg_made);
            assert_eq!(la.fg_att, lb.fg_att);
        }
    }

    #[test]
    fn pick_engine_returns_default() {
        let e = crate::pick_engine("statistical");
        assert_eq!(e.name(), "statistical");
        let e2 = crate::pick_engine("unknown-name");
        assert_eq!(e2.name(), "statistical");
    }

    #[test]
    fn fallback_no_rotation_still_works() {
        let engine = StatisticalEngine::with_defaults();
        let home = TeamSnapshot {
            id: TeamId(10), abbrev: "X".into(), overall: 75,
            home_court_advantage: 2.0, rotation: vec![],
        };
        let away = TeamSnapshot {
            id: TeamId(11), abbrev: "Y".into(), overall: 75,
            home_court_advantage: 2.0, rotation: vec![],
        };
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        let r = engine.simulate_game(&home, &away, &ctx(1), &mut rng);
        assert!(r.home_score >= 60 && r.home_score <= 200);
        assert!(!r.box_score.home_lines.is_empty());
    }

    #[test]
    fn sim_params_default_loadable_from_toml() {
        let p = SimParams::from_toml_str(
            r#"
            pace_mean = 99.0
            pace_sigma = 3.0
            score_sigma = 9.0
            home_court_advantage = 2.8
            injury_rate_per_game = 0.005
            max_overtimes = 4
            usage_distribution_alpha = 1.4
            "#,
        ).unwrap();
        assert_eq!(p.max_overtimes, 4);
    }

    #[test]
    fn ships_default_sim_params_toml() {
        // Walk up from CARGO_MANIFEST_DIR (crates/nba3k-sim) to the workspace
        // root and find data/sim_params.toml. The file must exist and parse.
        let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let path = std::path::PathBuf::from(manifest)
            .parent().unwrap()
            .parent().unwrap()
            .join("data/sim_params.toml");
        let p = SimParams::from_toml(&path).expect("data/sim_params.toml must parse");
        assert!(p.pace_mean > 80.0 && p.pace_mean < 120.0);
    }

    #[test]
    fn one_season_perf_under_5s_release() {
        // 1230 games (~ one regular season). Only meaningful in --release.
        // In dev this is just a correctness check that nothing panics.
        let engine = StatisticalEngine::with_defaults();
        let home = fair_team(20, "HHH", 78);
        let away = fair_team(21, "AAA", 77);
        let c = ctx(2);
        let mut rng = ChaCha8Rng::seed_from_u64(2025);
        let start = std::time::Instant::now();
        for _ in 0..1230 {
            let _ = engine.simulate_game(&home, &away, &c, &mut rng);
        }
        let elapsed = start.elapsed();
        if cfg!(not(debug_assertions)) {
            assert!(elapsed.as_secs() < 5, "1230 sims took {:?}, want <5s", elapsed);
        }
    }
}
