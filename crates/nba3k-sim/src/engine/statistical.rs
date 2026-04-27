use crate::params::SimParams;
use crate::{Engine, GameContext, RotationSlot, TeamSnapshot};
use nba3k_core::{BoxScore, GameResult, InjuryStatus, InjurySeverity, PlayerId, PlayerLine, Ratings};
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
        // Fall back to overall-only sim. Each rating point = ~0.6 ORtg.
        let delta = (team.overall as f32 - FALLBACK_OVERALL) * 0.6;
        return TeamProfile {
            ortg: BASE_ORTG + delta,
            drtg: BASE_DRTG - delta,
            pace: base_pace,
        };
    }

    // M19.2: replace the per-attribute weighted sum with a 9-feature
    // team-quality vector (see `engine::team_quality`). The new model captures
    // structural signals the linear sum couldn't: perimeter_containment (MIN
    // not AVG), top-3 star concentration, position-aware spacing, top-2 product
    // for rim protection. Coefficients are hand-tuned to NBA 2024-25 anchors.
    use crate::engine::team_quality::{ratings_from_vector, vector_from_rotation, QualityToRatingWeights};
    let v = vector_from_rotation(&team.rotation);
    let weights = QualityToRatingWeights::default();
    let (ortg, drtg) = ratings_from_vector(&v, &weights);

    // Pace adjustment from rotation athleticism; small magnitude.
    let total_minutes_share: f32 = team.rotation.iter().map(|r| r.minutes_share).sum();
    let norm = if total_minutes_share > 0.0 { total_minutes_share } else { 1.0 };
    let pace_acc: f32 = team
        .rotation
        .iter()
        .map(|slot| {
            let w = slot.minutes_share / norm;
            let r = &slot.ratings;
            let pace_score = (r.speed as f32 + r.agility as f32) * 0.5 - 70.0;
            w * pace_score * 0.05
        })
        .sum();

    TeamProfile {
        ortg,
        drtg,
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
    // M19.2 fix: original formula `own.ortg - opp.drtg + 100` was sign-inverted —
    // a low opp.drtg (good defense, NBA convention = lower is better) reduced
    // own subtraction therefore RAISED the expected score, the opposite of what
    // good defense should do. Corrected to `own.ortg + opp.drtg - LEAGUE_AVG`
    // which gives the right behavior:
    //   opp DRtg low (elite D) → I score less than my ORtg
    //   opp DRtg high (sieve)  → I score more than my ORtg
    //   neutral (own.ortg = opp.drtg = 115) → I score 115.
    const LEAGUE_AVG_RTG: f32 = 115.0;
    let expected_per_100 = own.ortg + opp.drtg - LEAGUE_AVG_RTG + hca;
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
    const LEAGUE_AVG_RTG: f32 = 115.0;
    let expected_per_100 = own.ortg + opp.drtg - LEAGUE_AVG_RTG + hca;
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

// ---- Per-game shooting-percentage calibration ------------------------------
//
// Per-player game FG/3P/FT% are sampled from a tight Normal centered on the
// player's rating-driven mean. The bands match NBA reality:
//   FG% : mean ~0.45, band [0.30, 0.65]
//   3P% : mean ~0.35, band [0.20, 0.50]
//   FT% : mean ~0.80, band [0.65, 0.95]
//
// `mid_range` + `free_throw` drive overall FG%; `three_point` drives 3P%;
// `free_throw` drives FT%. Per-game sigma = 0.05 — tight enough to keep a
// season aggregate inside the band, wide enough that a hot/cold game shows.

/// Per-player FG% mean as a function of their shooting ratings. Output range
/// roughly [0.38, 0.55] — peak shooters sit near the top, weak shooters near
/// the bottom. Per-game samples then add ±0.05 sigma.
fn fg_pct_mean(r: &Ratings) -> f32 {
    // Composite of mid_range, free_throw, close_shot, driving_layup —
    // the "make-shots" cluster. Center at rating 75 → 0.45 league avg.
    let composite = (r.mid_range as f32 * 0.30
        + r.free_throw as f32 * 0.20
        + r.close_shot as f32 * 0.25
        + r.driving_layup as f32 * 0.25) - 75.0;
    (0.45 + composite * 0.005).clamp(0.38, 0.55)
}

/// Per-player 3P% mean. Center at rating 75 → 0.35.
fn three_pct_mean(r: &Ratings) -> f32 {
    let lift = (r.three_point as f32 - 75.0) * 0.005;
    (0.35 + lift).clamp(0.28, 0.45)
}

/// Per-player FT% mean. Center at rating 75 → 0.80.
fn ft_pct_mean(r: &Ratings) -> f32 {
    let lift = (r.free_throw as f32 - 75.0) * 0.006;
    (0.80 + lift).clamp(0.70, 0.92)
}

/// Sample one game's FG% from `Normal(mean, 0.05)`, clamped to NBA-realistic
/// per-game band [0.30, 0.65].
fn sample_game_fg_pct(r: &Ratings, rng: &mut dyn RngCore) -> f32 {
    let mean = fg_pct_mean(r);
    let dist = Normal::new(mean as f64, 0.05).expect("valid sigma");
    (dist.sample(rng) as f32).clamp(0.30, 0.65)
}

fn sample_game_three_pct(r: &Ratings, rng: &mut dyn RngCore) -> f32 {
    let mean = three_pct_mean(r);
    let dist = Normal::new(mean as f64, 0.05).expect("valid sigma");
    (dist.sample(rng) as f32).clamp(0.20, 0.50)
}

fn sample_game_ft_pct(r: &Ratings, rng: &mut dyn RngCore) -> f32 {
    let mean = ft_pct_mean(r);
    let dist = Normal::new(mean as f64, 0.05).expect("valid sigma");
    (dist.sample(rng) as f32).clamp(0.65, 0.95)
}

/// Rebuild a PlayerLine's shooting line so per-game FG/3P/FT% are sampled
/// from realistic per-game distributions tied to the player's ratings.
///
/// Strategy:
///   1. Sample target fg_pct, three_pct, ft_pct from the player's ratings.
///   2. Hold approximate FT and 3PT volume from the existing line (these are
///      structural — usage and shot diet, not efficiency).
///   3. Derive `fg_att` so that `fg_pct * fg_att * 2` plus the 3PT lift and
///      FT contribution lands near `target_pts`.
///   4. Compute `fg_made = round(fg_pct * fg_att)` — never sample fg_made
///      independently. This is the calibration anchor M11-C requires.
///   5. Recompute pts = 2*two_made + 3*three_made + ft_made.
fn rebuild_shooting_line(
    line: &mut PlayerLine,
    ratings: &Ratings,
    target_pts: u8,
    rng: &mut dyn RngCore,
) {
    if target_pts == 0 {
        line.pts = 0;
        line.fg_made = 0;
        line.fg_att = 0;
        line.three_made = 0;
        line.three_att = 0;
        line.ft_made = 0;
        line.ft_att = 0;
        return;
    }

    let fg_pct = sample_game_fg_pct(ratings, rng);
    let three_pct = sample_game_three_pct(ratings, rng);
    let ft_pct = sample_game_ft_pct(ratings, rng);

    // 3PA share of FGA: scaled by player's three_point appetite. Shooters take
    // ~0.50 of their FGA from 3; bigs take ~0.05. Linear in three_point rating.
    let three_share = ((ratings.three_point as f32 - 50.0) / 100.0).clamp(0.05, 0.55);

    // FT rate ≈ 0.25 × FGA — league average. Hold whatever the upstream
    // sampler produced if reasonable; otherwise estimate from target_pts.
    let ft_att = line.ft_att;
    let ft_made = ((ft_att as f32 * ft_pct).round() as u16).min(ft_att as u16) as u8;
    let ft_pts = ft_made as u16;

    // Solve for fg_att given target_pts:
    //   pts = ft_pts + 2 * (1 - share) * fg_pct * fga + 3 * share * three_pct * fga
    // Let blended_pts_per_fga = 2 * (1 - share) * fg_pct + 3 * share * three_pct
    let blended = 2.0 * (1.0 - three_share) * fg_pct + 3.0 * three_share * three_pct;
    let needed_field_pts = (target_pts as f32 - ft_pts as f32).max(0.0);
    let fg_att = if blended > 0.05 {
        (needed_field_pts / blended).round().clamp(0.0, 60.0) as u16
    } else {
        ((target_pts as f32) / 1.1).round() as u16
    };
    let fg_att = fg_att.min(60) as u8;

    let three_att = ((fg_att as f32 * three_share).round() as u16).min(fg_att as u16) as u8;
    let two_att = fg_att.saturating_sub(three_att);

    let three_made = ((three_att as f32 * three_pct).round() as u16).min(three_att as u16) as u8;
    let two_made = ((two_att as f32 * fg_pct).round() as u16).min(two_att as u16) as u8;
    let fg_made = three_made.saturating_add(two_made);

    let pts_total = (two_made as u16) * 2 + (three_made as u16) * 3 + ft_pts;

    line.pts = pts_total.min(u8::MAX as u16) as u8;
    line.fg_made = fg_made;
    line.fg_att = fg_att.max(fg_made);
    line.three_made = three_made;
    line.three_att = three_att.max(three_made);
    line.ft_made = ft_made;
    line.ft_att = ft_att.max(ft_made);
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

    // Field-goal attempts ≈ points / 1.1 (rough TS conversion). Only used
    // to seed FT volume; per-player FGA is derived inside `rebuild_shooting_line`
    // from the sampled fg_pct.
    let total_fg_att: u16 = ((points as f32) / 1.1).round() as u16;

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

        // Build shooting line with rating-driven per-game FG/3P/FT% (M11-C).
        // Seed FT attempts from the upstream usage distribution; FG attempts
        // get re-derived from `pts` and the sampled fg_pct so per-game FG%
        // lands in NBA-realistic ranges.
        let mut line = PlayerLine {
            player: slot.player,
            minutes: m,
            pts: 0,
            reb: 0, ast: 0, stl: 0, blk: 0, tov: 0,
            fg_made: 0, fg_att: 0,
            three_made: 0, three_att: 0,
            ft_made: 0, ft_att: ft_att_per[i],
            plus_minus: 0,
        };
        rebuild_shooting_line(&mut line, &slot.ratings, pts, rng);
        let fg_made = line.fg_made;
        let fg_att = line.fg_att;
        let three_made = line.three_made;
        let three_att = line.three_att;
        let ft_made = line.ft_made;
        let ft_att = line.ft_att;
        let pts = line.pts;

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
    // points proportionally; **then** rebuild the shooting line so per-game
    // FG/3P/FT% are sampled from rating-driven Normals (M11-C calibration).
    //
    // Scale CAP at 1.20: archetype baselines target real NBA distribution,
    // and an unbounded scale up can dump the entire residual team_score onto
    // slot-1, inflating top-scorer PPG (Tatum 34, SGA 40) when raw_total is
    // low. Capping at 1.20 + spreading leftover across bench keeps top-slot
    // share realistic (~24% of team, not 33%).
    let raw_total: u32 = lines.iter().map(|l| l.pts as u32).sum();
    let pts_cap = weights.single_game_pts_cap as f64;
    if raw_total > 0 {
        let raw_scale = team_score as f64 / raw_total as f64;
        let scale = raw_scale.min(1.20);
        for (line, slot) in lines.iter_mut().zip(rotation.iter()) {
            let scaled = (line.pts as f64 * scale).round().min(pts_cap);
            let new_pts = scaled.clamp(0.0, 99.0) as u8;
            rebuild_shooting_line(line, &slot.ratings, new_pts, rng);
        }
        // If we capped, spread the leftover team_score evenly across bench
        // (slots 4-7 by minutes_share — middle of rotation, not deep bench).
        let after_total: u32 = lines.iter().map(|l| l.pts as u32).sum();
        let leftover = (team_score as i32 - after_total as i32).max(0) as u32;
        if leftover > 0 && lines.len() > 3 {
            let bench_slots: Vec<usize> = (3..lines.len().min(7)).collect();
            if !bench_slots.is_empty() {
                let per = (leftover as usize / bench_slots.len()) as u8;
                for &i in &bench_slots {
                    let new_pts = lines[i].pts.saturating_add(per).min(pts_cap as u8);
                    rebuild_shooting_line(&mut lines[i], &rotation[i].ratings, new_pts, rng);
                }
            }
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
        rebuild_shooting_line(&mut lines[target_idx], &rotation[target_idx].ratings, pts, rng);
    }

    // PTS may now drift from the simmed team_score by a few points (rounding
    // through the FG% pipeline). Distribute the residual onto the highest-PTS
    // line as FT to preserve the headline score without smearing FG%.
    let new_total: u32 = lines.iter().map(|l| l.pts as u32).sum();
    let target = team_score as i32;
    let drift = target - new_total as i32;
    if drift != 0 && !lines.is_empty() {
        let idx = lines
            .iter()
            .enumerate()
            .max_by_key(|(_, l)| l.pts)
            .map(|(i, _)| i)
            .unwrap_or(0);
        apply_pts_drift(&mut lines[idx], drift);
    }

    lines
}

/// Add (or subtract) up to `drift` points from a line via FT only — keeps
/// FG% untouched. Drift is bounded by the natural FG% pipeline rounding so
/// 1-3 points per line is the typical magnitude.
fn apply_pts_drift(line: &mut PlayerLine, drift: i32) {
    if drift > 0 {
        let bump = drift.min(20) as u8;
        line.pts = line.pts.saturating_add(bump);
        line.ft_made = line.ft_made.saturating_add(bump);
        line.ft_att = line.ft_att.saturating_add(bump);
    } else if drift < 0 {
        let cut = (-drift).min(line.pts as i32) as u8;
        // Remove from FT first, then 2P (each = 2 pts), then 3P (each = 3 pts).
        let mut remaining = cut as u16;
        let ft_cut = (line.ft_made as u16).min(remaining);
        line.ft_made = line.ft_made.saturating_sub(ft_cut as u8);
        line.pts = line.pts.saturating_sub(ft_cut as u8);
        remaining = remaining.saturating_sub(ft_cut);
        if remaining >= 2 && line.fg_made > line.three_made {
            let two_makes_to_remove = (remaining / 2).min((line.fg_made - line.three_made) as u16) as u8;
            line.fg_made = line.fg_made.saturating_sub(two_makes_to_remove);
            line.pts = line.pts.saturating_sub(two_makes_to_remove * 2);
        }
    }
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

/// Roll a single injury for a player who logged `minutes` in a game.
///
/// Probability scales with workload: 0.5% baseline, +0.5% for every 10 minutes
/// played above 30. A 48-min game → ~1.4%. Severity mix on hit: DayToDay 70%,
/// ShortTerm 25%, LongTerm 5%. Returns `None` if no injury rolled.
fn roll_one_injury(minutes: u8, rng: &mut dyn RngCore) -> Option<InjuryStatus> {
    if minutes == 0 {
        return None;
    }
    let extra = (minutes as f32 - 30.0).max(0.0);
    let p = 0.005 + (extra / 10.0) * 0.005;
    if rng.gen::<f32>() >= p {
        return None;
    }
    let s: f32 = rng.gen();
    let (severity, games_remaining, description) = if s < 0.70 {
        let games = rng.gen_range(1..=3);
        (InjurySeverity::DayToDay, games, day_to_day_desc(rng))
    } else if s < 0.95 {
        let games = rng.gen_range(5..=15);
        (InjurySeverity::ShortTerm, games, short_term_desc(rng))
    } else {
        let games = rng.gen_range(20..=50);
        (InjurySeverity::LongTerm, games, long_term_desc(rng))
    };
    Some(InjuryStatus { description, games_remaining, severity })
}

fn day_to_day_desc(rng: &mut dyn RngCore) -> String {
    const POOL: &[&str] = &[
        "ankle sprain",
        "sore knee",
        "lower back tightness",
        "bruised hip",
        "wrist soreness",
    ];
    POOL[rng.gen_range(0..POOL.len())].to_string()
}

fn short_term_desc(rng: &mut dyn RngCore) -> String {
    const POOL: &[&str] = &[
        "strained hamstring",
        "high ankle sprain",
        "groin strain",
        "calf strain",
        "AC joint sprain",
    ];
    POOL[rng.gen_range(0..POOL.len())].to_string()
}

fn long_term_desc(rng: &mut dyn RngCore) -> String {
    const POOL: &[&str] = &[
        "torn meniscus",
        "stress fracture in foot",
        "torn plantar fascia",
        "labrum tear",
        "broken hand",
    ];
    POOL[rng.gen_range(0..POOL.len())].to_string()
}

/// Tick down an injury by one game/day. Returns the updated status — `None`
/// when `games_remaining` reaches zero so the caller can clear the slot.
pub fn tick_injury(status: &InjuryStatus) -> Option<InjuryStatus> {
    if status.games_remaining <= 1 {
        None
    } else {
        Some(InjuryStatus {
            description: status.description.clone(),
            games_remaining: status.games_remaining - 1,
            severity: status.severity,
        })
    }
}

/// Public injury-roll API. After `simulate_game`, callers feed the resulting
/// box score back through this function to discover which players picked up
/// new injuries. Caller is responsible for persisting via `Store::upsert_player`.
///
/// Each player who logged minutes gets one roll; probability scales with
/// minutes (see `roll_one_injury`). Output preserves source-of-truth ordering
/// (home lines first, then away lines) so downstream news/log output is stable.
pub fn roll_injuries_from_box(
    box_score: &BoxScore,
    rng: &mut dyn RngCore,
) -> Vec<(PlayerId, InjuryStatus)> {
    let mut out = Vec::new();
    for line in box_score.home_lines.iter().chain(box_score.away_lines.iter()) {
        if line.minutes == 0 {
            continue;
        }
        if let Some(inj) = roll_one_injury(line.minutes, rng) {
            out.push((line.player, inj));
        }
    }
    out
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

        // Injury rolls happen in `roll_injuries_from_box`, called by the CLI
        // after `simulate_game` so it can persist via `Store::upsert_player`.
        // We still touch `injury_rate_per_game` so the param survives loaders.
        let _ = p.injury_rate_per_game;

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
