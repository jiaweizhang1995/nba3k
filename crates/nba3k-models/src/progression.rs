//! Player progression / regression — yearly track-to-peak engine.
//!
//! Borrowed from NBA 2K's "potential-as-yearly-adjusted-track-to-peak"
//! mental model (`RESEARCH-NBA2K.md` § 3). The static `Player.potential`
//! ceiling is never mutated; instead `PlayerDevelopment` carries a
//! `dynamic_potential` that gets revised yearly based on whether the
//! player is on track to hit the ceiling by `peak_end_age`.
//!
//! Per-attribute aging rates (deltas at peak/post-peak — pre-peak is
//! aggressive growth on top of these baselines):
//!
//! | Category    | Attribute         | Pre-peak | Peak  | Post-peak |
//! |-------------|-------------------|----------|-------|-----------|
//! | Athleticism | speed, vertical   | +fast    | flat  | -fast     |
//! | Athleticism | agility           | +fast    | flat  | -fast     |
//! | Athleticism | strength          | +med     | +slow | -slow     |
//! | Inside      | finishing cluster | +fast    | flat  | -med      |
//! | Inside      | post_control      | +slow    | +slow | -slow     |
//! | Shooting    | mid/3pt/ft        | +med     | +slow | -slow     |
//! | Handling    | passing_accuracy  | +med     | +slow | -slow     |
//! | Handling    | ball_handle       | +med     | flat  | -slow     |
//! | Handling    | speed_with_ball   | +fast    | flat  | -med      |
//! | Defense     | interior, perim   | +med     | flat  | -med      |
//! | Defense     | steal, block      | +med     | flat  | -med      |
//! | Rebounding  | off, def          | +med     | flat  | -med      |
//!
//! Cap: any single attribute moves at most +3 per season (gain) or
//! -4 per season (decline). Work ethic multiplies gains (0.5× at 30,
//! 1.5× at 99). Minutes-played acts as a modulator: heavy minutes
//! (>30/game) reinforce growth and accelerate post-peak decline.

use nba3k_core::{Player, PlayerId, Ratings, SeasonId};
use serde::{Deserialize, Serialize};

/// Per-player development state. Lives next to `Player`, persisted as a
/// JSON blob in the `players.dev_json` column. The static `Player.potential`
/// is the immutable ceiling; `dynamic_potential` is the live projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerDevelopment {
    pub player_id: PlayerId,
    /// Age at which a player enters their prime window (typically 25-26).
    pub peak_start_age: u8,
    /// Age at which a player exits their prime window (typically 30-31).
    pub peak_end_age: u8,
    /// Live projected ceiling. Distinct from `Player.potential`.
    pub dynamic_potential: u8,
    /// 0..=99. Multiplies progression rate.
    pub work_ethic: u8,
    /// Last season this player was run through the progression pass —
    /// guards against double-applying within a season.
    pub last_progressed_season: SeasonId,
}

impl PlayerDevelopment {
    /// Default for a new player: prime window 25-30, dynamic potential
    /// matches the static ceiling, average work ethic.
    pub fn defaults_for(player: &Player, season: SeasonId) -> Self {
        Self {
            player_id: player.id,
            peak_start_age: 25,
            peak_end_age: 30,
            dynamic_potential: player.potential,
            work_ethic: 70,
            last_progressed_season: season,
        }
    }
}

/// Signed delta to a `Ratings` struct. Each field is the per-attribute
/// change to apply in `apply_delta`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttributeDelta {
    pub close_shot: i8,
    pub driving_layup: i8,
    pub driving_dunk: i8,
    pub standing_dunk: i8,
    pub post_control: i8,
    pub mid_range: i8,
    pub three_point: i8,
    pub free_throw: i8,
    pub passing_accuracy: i8,
    pub ball_handle: i8,
    pub speed_with_ball: i8,
    pub interior_defense: i8,
    pub perimeter_defense: i8,
    pub steal: i8,
    pub block: i8,
    pub off_reb: i8,
    pub def_reb: i8,
    pub speed: i8,
    pub agility: i8,
    pub strength: i8,
    pub vertical: i8,
}

impl AttributeDelta {
    /// Sum of all field magnitudes — used for tests and dynamic_potential.
    pub fn sum_signed(&self) -> i32 {
        self.close_shot as i32
            + self.driving_layup as i32
            + self.driving_dunk as i32
            + self.standing_dunk as i32
            + self.post_control as i32
            + self.mid_range as i32
            + self.three_point as i32
            + self.free_throw as i32
            + self.passing_accuracy as i32
            + self.ball_handle as i32
            + self.speed_with_ball as i32
            + self.interior_defense as i32
            + self.perimeter_defense as i32
            + self.steal as i32
            + self.block as i32
            + self.off_reb as i32
            + self.def_reb as i32
            + self.speed as i32
            + self.agility as i32
            + self.strength as i32
            + self.vertical as i32
    }

    /// Apply the delta to a Ratings struct in-place, clamping each field
    /// to 0..=99.
    pub fn apply(&self, r: &mut Ratings) {
        fn add(field: &mut u8, delta: i8) {
            let v = *field as i16 + delta as i16;
            *field = v.clamp(0, 99) as u8;
        }
        add(&mut r.close_shot, self.close_shot);
        add(&mut r.driving_layup, self.driving_layup);
        add(&mut r.driving_dunk, self.driving_dunk);
        add(&mut r.standing_dunk, self.standing_dunk);
        add(&mut r.post_control, self.post_control);
        add(&mut r.mid_range, self.mid_range);
        add(&mut r.three_point, self.three_point);
        add(&mut r.free_throw, self.free_throw);
        add(&mut r.passing_accuracy, self.passing_accuracy);
        add(&mut r.ball_handle, self.ball_handle);
        add(&mut r.speed_with_ball, self.speed_with_ball);
        add(&mut r.interior_defense, self.interior_defense);
        add(&mut r.perimeter_defense, self.perimeter_defense);
        add(&mut r.steal, self.steal);
        add(&mut r.block, self.block);
        add(&mut r.off_reb, self.off_reb);
        add(&mut r.def_reb, self.def_reb);
        add(&mut r.speed, self.speed);
        add(&mut r.agility, self.agility);
        add(&mut r.strength, self.strength);
        add(&mut r.vertical, self.vertical);
    }
}

// Per-attribute aging speed: how fast each attribute moves in the
// pre-peak/peak/post-peak windows. The unit is "fraction of the global
// gain budget assigned to this attribute". Sums roughly to 1.0 across
// the 21 fields.
#[derive(Clone, Copy)]
struct AgingProfile {
    /// Pre-peak growth share.
    grow: f32,
    /// Peak-window growth share (small).
    plateau: f32,
    /// Post-peak decline share.
    decline: f32,
}

const ATHLETIC_FAST: AgingProfile = AgingProfile { grow: 0.85, plateau: 0.05, decline: 1.20 };
const ATHLETIC_STR: AgingProfile = AgingProfile { grow: 0.55, plateau: 0.30, decline: 0.55 };
const FINISH_FAST: AgingProfile = AgingProfile { grow: 0.85, plateau: 0.10, decline: 0.85 };
const POST_SLOW: AgingProfile = AgingProfile { grow: 0.45, plateau: 0.45, decline: 0.45 };
const SHOOT_SLOW: AgingProfile = AgingProfile { grow: 0.60, plateau: 0.40, decline: 0.45 };
const PASS_SLOW: AgingProfile = AgingProfile { grow: 0.55, plateau: 0.40, decline: 0.40 };
const HANDLE_MED: AgingProfile = AgingProfile { grow: 0.65, plateau: 0.20, decline: 0.55 };
const SPEEDBALL_FAST: AgingProfile = AgingProfile { grow: 0.80, plateau: 0.05, decline: 0.95 };
const DEFENSE_MED: AgingProfile = AgingProfile { grow: 0.70, plateau: 0.10, decline: 0.80 };
const REB_MED: AgingProfile = AgingProfile { grow: 0.65, plateau: 0.10, decline: 0.70 };

fn profiles() -> [(&'static str, AgingProfile); 21] {
    [
        ("close_shot", FINISH_FAST),
        ("driving_layup", FINISH_FAST),
        ("driving_dunk", ATHLETIC_FAST),
        ("standing_dunk", FINISH_FAST),
        ("post_control", POST_SLOW),
        ("mid_range", SHOOT_SLOW),
        ("three_point", SHOOT_SLOW),
        ("free_throw", SHOOT_SLOW),
        ("passing_accuracy", PASS_SLOW),
        ("ball_handle", HANDLE_MED),
        ("speed_with_ball", SPEEDBALL_FAST),
        ("interior_defense", DEFENSE_MED),
        ("perimeter_defense", DEFENSE_MED),
        ("steal", DEFENSE_MED),
        ("block", DEFENSE_MED),
        ("off_reb", REB_MED),
        ("def_reb", REB_MED),
        ("speed", ATHLETIC_FAST),
        ("agility", ATHLETIC_FAST),
        ("strength", ATHLETIC_STR),
        ("vertical", ATHLETIC_FAST),
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AgePhase {
    PrePeak,
    AtPeak,
    PostPeak,
}

fn age_phase(age: u8, dev: &PlayerDevelopment) -> AgePhase {
    if age < dev.peak_start_age {
        AgePhase::PrePeak
    } else if age <= dev.peak_end_age {
        AgePhase::AtPeak
    } else {
        AgePhase::PostPeak
    }
}

/// Convert minutes-per-season into a 0.0..=1.5 modulator. 35 min/game over
/// 82 games (~2870 minutes) is a workhorse — yields ~1.0. Light minutes
/// (<800) damp progression.
fn minutes_modulator(mins_played: u32) -> f32 {
    if mins_played < 200 {
        0.30
    } else if mins_played < 800 {
        0.55
    } else if mins_played < 1500 {
        0.80
    } else if mins_played < 2400 {
        1.00
    } else {
        1.15
    }
}

fn work_ethic_modulator(we: u8) -> f32 {
    // 30 -> 0.55, 70 -> 1.00, 99 -> 1.45
    0.55 + (we.saturating_sub(30) as f32) * (0.90 / 69.0)
}

/// How aggressive growth is for this player given age. Returns the total
/// raw "points budget" (before per-attribute weighting) that progression
/// can spread across the 21 attributes. Expressed in "attribute-points
/// per season" — needs to clear ~15-25 to move OVR by 2-3 points given
/// integer-bucket overall_for math.
fn growth_budget(age: u8, dev: &PlayerDevelopment) -> f32 {
    match age_phase(age, dev) {
        // Steeper growth far from peak; tapers as player approaches peak.
        AgePhase::PrePeak => {
            let years_to_peak = dev.peak_start_age.saturating_sub(age) as f32;
            // ~12 pts per year far from peak, ~7 pts at the boundary —
            // landed by tuning against the M5 acceptance test (22yo OVR-78
            // workhorse gains +2-3 OVR).
            6.0 + years_to_peak * 1.4
        }
        // Mild gain at peak — rookie-contract-extension years.
        AgePhase::AtPeak => 2.5,
        AgePhase::PostPeak => 0.0,
    }
}

/// How aggressive decline is for this player given age. Returns the total
/// raw "points budget" that regression deducts.
fn decline_budget(age: u8, dev: &PlayerDevelopment) -> f32 {
    match age_phase(age, dev) {
        AgePhase::PrePeak | AgePhase::AtPeak => 0.0,
        AgePhase::PostPeak => {
            let years_past_peak = age.saturating_sub(dev.peak_end_age) as f32;
            // 1 yr past: ~2 pts; 5 yrs past: ~7 pts
            1.5 + years_past_peak * 1.1
        }
    }
}

fn read_field(r: &Ratings, name: &str) -> u8 {
    match name {
        "close_shot" => r.close_shot,
        "driving_layup" => r.driving_layup,
        "driving_dunk" => r.driving_dunk,
        "standing_dunk" => r.standing_dunk,
        "post_control" => r.post_control,
        "mid_range" => r.mid_range,
        "three_point" => r.three_point,
        "free_throw" => r.free_throw,
        "passing_accuracy" => r.passing_accuracy,
        "ball_handle" => r.ball_handle,
        "speed_with_ball" => r.speed_with_ball,
        "interior_defense" => r.interior_defense,
        "perimeter_defense" => r.perimeter_defense,
        "steal" => r.steal,
        "block" => r.block,
        "off_reb" => r.off_reb,
        "def_reb" => r.def_reb,
        "speed" => r.speed,
        "agility" => r.agility,
        "strength" => r.strength,
        "vertical" => r.vertical,
        _ => 0,
    }
}

fn write_field(d: &mut AttributeDelta, name: &str, val: i8) {
    match name {
        "close_shot" => d.close_shot = val,
        "driving_layup" => d.driving_layup = val,
        "driving_dunk" => d.driving_dunk = val,
        "standing_dunk" => d.standing_dunk = val,
        "post_control" => d.post_control = val,
        "mid_range" => d.mid_range = val,
        "three_point" => d.three_point = val,
        "free_throw" => d.free_throw = val,
        "passing_accuracy" => d.passing_accuracy = val,
        "ball_handle" => d.ball_handle = val,
        "speed_with_ball" => d.speed_with_ball = val,
        "interior_defense" => d.interior_defense = val,
        "perimeter_defense" => d.perimeter_defense = val,
        "steal" => d.steal = val,
        "block" => d.block = val,
        "off_reb" => d.off_reb = val,
        "def_reb" => d.def_reb = val,
        "speed" => d.speed = val,
        "agility" => d.agility = val,
        "strength" => d.strength = val,
        "vertical" => d.vertical = val,
        _ => {}
    }
}

/// Compute the per-attribute progression delta for one season. Combines
/// age-phase budget, work_ethic + minutes modulators, and per-attribute
/// growth share. Caps each attribute at +3 (gain) / -4 (decline).
///
/// `current_age` is the age the player will be *during* the upcoming
/// season — pass `player.age + 1` if calling at season-end before the
/// age tick.
pub fn progress_player(
    player: &Player,
    dev: &PlayerDevelopment,
    mins_played: u32,
    current_age: u8,
) -> AttributeDelta {
    let phase = age_phase(current_age, dev);
    let we = work_ethic_modulator(dev.work_ethic);
    let mins = minutes_modulator(mins_played);

    let mut delta = AttributeDelta::default();

    if phase == AgePhase::PostPeak {
        // Past peak — defer to regress_player. Keep callers honest by
        // returning zero so they can call regress_player explicitly.
        return delta;
    }

    let budget = growth_budget(current_age, dev) * we * mins;
    if budget <= 0.0 {
        return delta;
    }

    // Bias growth toward attributes the player already has (a 70 ball_handle
    // grows into the 75-80 range faster than a 40 ball_handle catches up to
    // playmaking). Concretely: scale the per-attribute share by a function
    // of the current value.
    let mut weights: Vec<(&'static str, f32)> = Vec::with_capacity(21);
    let mut wsum = 0.0_f32;
    for (name, profile) in profiles().iter() {
        let share = match phase {
            AgePhase::PrePeak => profile.grow,
            AgePhase::AtPeak => profile.plateau,
            AgePhase::PostPeak => 0.0,
        };
        // Skill bias: (current / 60), clamped to [0.4, 1.6]. Lifts the
        // floor so a developmental player still grows a little in their
        // weak areas.
        let cur = read_field(&player.ratings, name) as f32;
        let bias = (cur / 60.0).clamp(0.4, 1.6);
        let w = share * bias;
        weights.push((name, w));
        wsum += w;
    }

    // Headroom guard: progression is choked off as a player approaches
    // their dynamic_potential. A player already at their projected ceiling
    // doesn't keep gaining — matches 2K's track-to-peak behavior.
    let est_overall = player.ratings.overall_for(player.primary_position) as i32;
    let headroom = (dev.dynamic_potential as i32 - est_overall).max(0) as f32;
    let headroom_mult = (headroom / 8.0).clamp(0.20, 1.0);

    if wsum <= 0.0 {
        return delta;
    }
    let scaled_budget = budget * headroom_mult;

    // Convert the float budget into integer points using
    // largest-remainder allocation. Total points to distribute is the
    // rounded budget (e.g. 5.84 → 6 points). This avoids the failure
    // mode where 21 sub-1.0 fractions all round to zero.
    let total_points = scaled_budget.round().max(0.0) as u32;
    if total_points == 0 {
        return delta;
    }

    let mut frac: Vec<(usize, f32)> = Vec::with_capacity(21);
    let mut allocated: Vec<u32> = vec![0; 21];
    let mut allocated_sum: u32 = 0;
    for (i, (_, w)) in weights.iter().enumerate() {
        let exact = total_points as f32 * (w / wsum);
        let floor = exact.floor() as u32;
        allocated[i] = floor;
        allocated_sum += floor;
        frac.push((i, exact - floor as f32));
    }
    // Distribute the remaining points to the attributes with the
    // largest fractional remainder.
    let mut remaining = total_points.saturating_sub(allocated_sum);
    frac.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (i, _) in frac {
        if remaining == 0 {
            break;
        }
        allocated[i] += 1;
        remaining -= 1;
    }

    for (i, (name, _)) in weights.iter().enumerate() {
        let v = (allocated[i] as i32).clamp(0, 3) as i8;
        if v == 0 {
            continue;
        }
        let cur = read_field(&player.ratings, name) as i16;
        let allowed = (99_i16 - cur).max(0) as i8;
        write_field(&mut delta, name, v.min(allowed));
    }
    delta
}

/// Compute the per-attribute regression delta for one season. Athleticism
/// declines first, IQ-adjacent skills (post_control, passing) decline last
/// — well-documented NBA aging-curve consensus.
pub fn regress_player(
    player: &Player,
    dev: &PlayerDevelopment,
    current_age: u8,
) -> AttributeDelta {
    let phase = age_phase(current_age, dev);
    if phase != AgePhase::PostPeak {
        return AttributeDelta::default();
    }

    let budget = decline_budget(current_age, dev);
    if budget <= 0.0 {
        return AttributeDelta::default();
    }

    let mut weights: Vec<(&'static str, f32)> = Vec::with_capacity(21);
    let mut wsum = 0.0_f32;
    for (name, profile) in profiles().iter() {
        let share = profile.decline;
        // Floor bias: a 20 attribute can't lose much more — scale by
        // current/60 again so high attributes shed faster.
        let cur = read_field(&player.ratings, name) as f32;
        let bias = (cur / 60.0).clamp(0.30, 1.50);
        let w = share * bias;
        weights.push((name, w));
        wsum += w;
    }
    if wsum <= 0.0 {
        return AttributeDelta::default();
    }

    let total_points = budget.round().max(0.0) as u32;
    if total_points == 0 {
        return AttributeDelta::default();
    }

    let mut frac: Vec<(usize, f32)> = Vec::with_capacity(21);
    let mut allocated: Vec<u32> = vec![0; 21];
    let mut allocated_sum: u32 = 0;
    for (i, (_, w)) in weights.iter().enumerate() {
        let exact = total_points as f32 * (w / wsum);
        let floor = exact.floor() as u32;
        allocated[i] = floor;
        allocated_sum += floor;
        frac.push((i, exact - floor as f32));
    }
    let mut remaining = total_points.saturating_sub(allocated_sum);
    frac.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (i, _) in frac {
        if remaining == 0 {
            break;
        }
        allocated[i] += 1;
        remaining -= 1;
    }

    let mut delta = AttributeDelta::default();
    for (i, (name, _)) in weights.iter().enumerate() {
        let v = (allocated[i] as i32).clamp(0, 4) as i8;
        if v == 0 {
            continue;
        }
        let cur = read_field(&player.ratings, name) as i16;
        let allowed = (cur - 1).max(0) as i8;
        let drop = v.min(allowed);
        write_field(&mut delta, name, -drop);
    }
    delta
}

/// Update `dev.dynamic_potential` based on whether the player is on track
/// to hit their ceiling by `peak_end_age`. Implements the 2K
/// "track-to-peak" check: pre-peak players who are growing can keep their
/// projection; pre-peak players falling behind get revised down; past
/// peak_end the projection collapses to current overall.
///
/// Returns the new `dynamic_potential` value.
pub fn update_dynamic_potential(
    player: &Player,
    dev: &PlayerDevelopment,
    current_age: u8,
) -> u8 {
    let cur_ovr = player.ratings.overall_for(player.primary_position);
    let phase = age_phase(current_age, dev);

    match phase {
        AgePhase::PrePeak => {
            // Years left to grow (inclusive of current).
            let years_left = (dev.peak_end_age.saturating_sub(current_age) as i32).max(1);
            // What we'd need to gain per year to still hit dynamic_potential.
            let gap = (dev.dynamic_potential as i32 - cur_ovr as i32).max(0);
            let needed_per_year = gap as f32 / years_left as f32;
            // Realistic per-year overall gain ranges roughly 0.3..=2.5.
            // If `needed_per_year` is outside that, projection has slipped.
            if needed_per_year > 2.5 {
                // Falling behind — shave 1-3 off projection (more for bigger gaps).
                let shave = ((needed_per_year - 2.5) * 1.5).round().clamp(1.0, 3.0) as u8;
                dev.dynamic_potential.saturating_sub(shave)
            } else if needed_per_year < 0.0_f32.max(0.0) {
                // Already above projection — bump it slightly.
                dev.dynamic_potential.saturating_add(1).min(player.potential)
            } else {
                dev.dynamic_potential
            }
        }
        AgePhase::AtPeak => {
            // At-peak: ceiling = max(current, dynamic_potential). Allow
            // +1 if player is exceeding it.
            if cur_ovr > dev.dynamic_potential {
                cur_ovr.min(player.potential)
            } else {
                dev.dynamic_potential
            }
        }
        AgePhase::PostPeak => {
            // Past peak — projection collapses to current overall, never
            // re-grows.
            cur_ovr
        }
    }
}

/// Apply a full season-end progression step for a single player. Mutates
/// `player.ratings`, `player.overall`, and `dev` in place. Returns the
/// summed signed attribute delta (useful for tests/telemetry).
///
/// `mins_played` is the player's total minutes for the season just played.
/// `next_age` is the age the player will be for the *upcoming* season —
/// pass `player.age + 1` since the season-end pass is what advances age.
pub fn apply_progression_step(
    player: &mut Player,
    dev: &mut PlayerDevelopment,
    mins_played: u32,
    next_age: u8,
    season: SeasonId,
) -> i32 {
    let delta = match age_phase(next_age, dev) {
        AgePhase::PostPeak => regress_player(player, dev, next_age),
        _ => progress_player(player, dev, mins_played, next_age),
    };
    delta.apply(&mut player.ratings);
    player.overall = player.ratings.overall_for(player.primary_position);
    dev.dynamic_potential = update_dynamic_potential(player, dev, next_age);
    dev.last_progressed_season = season;
    delta.sum_signed()
}
