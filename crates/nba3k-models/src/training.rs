//! Training camp / dev points — once-per-offseason attribute bumps.
//!
//! Mirrors NBA 2K MyGM's "training camp" mechanic: the user picks a focus
//! area for one player and gets a deterministic +1..2 bump in that bucket.
//! +2 to the highest-current attribute in the cluster (the hand the
//! player already shoots with), +1 to the rest. All values capped at 99.
//!
//! Pure function — no I/O. Caller is responsible for one-shot persistence
//! (the per-season "training_used" guard lives in the CLI / store layer).

use nba3k_core::Player;

/// Training focus categories. Names match the CLI tokens (case-insensitive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingFocus {
    Shoot,
    Inside,
    Defense,
    Rebound,
    Athletic,
    Handle,
}

impl TrainingFocus {
    /// Parse a CLI token into a focus. Accepts the short forms used in
    /// `phases/M10-stretch.md` (`shoot`, `inside`, `def`, `reb`, `ath`,
    /// `handle`) plus the long forms (`defense`, `rebound`, `athletic`).
    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "shoot" | "shooting" => Some(Self::Shoot),
            "inside" => Some(Self::Inside),
            "def" | "defense" => Some(Self::Defense),
            "reb" | "rebound" | "rebounding" => Some(Self::Rebound),
            "ath" | "athletic" | "athleticism" => Some(Self::Athletic),
            "handle" | "handling" => Some(Self::Handle),
            _ => None,
        }
    }

    /// Human label used in the CLI output line.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Shoot => "shoot",
            Self::Inside => "inside",
            Self::Defense => "defense",
            Self::Rebound => "rebound",
            Self::Athletic => "athletic",
            Self::Handle => "handle",
        }
    }
}

/// Result of applying a training focus. `attributes_changed` lists the
/// affected attribute names (canonical `Ratings` field names) and the
/// signed delta actually applied (post-99-cap). `new_overall` is the
/// position-aware overall recomputed after the bump.
#[derive(Debug, Clone)]
pub struct TrainingDelta {
    pub attributes_changed: Vec<(&'static str, i8)>,
    pub new_overall: u8,
}

/// Mutate `player.ratings` per the focus mapping, recompute `player.overall`
/// for the player's primary position, and return the per-attribute delta
/// summary. The mapping (per `phases/M10-stretch.md`):
///
/// - Shoot    → mid_range / three_point / free_throw
/// - Inside   → close_shot / driving_layup / post_control
/// - Defense  → interior_defense / perimeter_defense / steal / block
/// - Rebound  → off_reb / def_reb
/// - Athletic → speed / agility / vertical / strength
/// - Handle   → passing_accuracy / ball_handle / speed_with_ball
///
/// The highest-current attribute in the cluster gets +2, the rest get +1.
/// Each attribute is capped at 99, so a player already at 99 in their best
/// shooting attribute will record a 0 delta there.
pub fn apply_training_focus(player: &mut Player, focus: TrainingFocus) -> TrainingDelta {
    let cluster: &[&'static str] = match focus {
        TrainingFocus::Shoot => &["mid_range", "three_point", "free_throw"],
        TrainingFocus::Inside => &["close_shot", "driving_layup", "post_control"],
        TrainingFocus::Defense => &["interior_defense", "perimeter_defense", "steal", "block"],
        TrainingFocus::Rebound => &["off_reb", "def_reb"],
        TrainingFocus::Athletic => &["speed", "agility", "vertical", "strength"],
        TrainingFocus::Handle => &["passing_accuracy", "ball_handle", "speed_with_ball"],
    };

    // Find the highest-current attribute in the cluster. Tie-break by the
    // declaration order in `cluster` so the result is fully deterministic.
    let mut top_idx = 0usize;
    let mut top_val: u8 = read_attr(player, cluster[0]);
    for (i, name) in cluster.iter().enumerate().skip(1) {
        let v = read_attr(player, name);
        if v > top_val {
            top_idx = i;
            top_val = v;
        }
    }

    let mut changes: Vec<(&'static str, i8)> = Vec::with_capacity(cluster.len());
    for (i, name) in cluster.iter().enumerate() {
        let bump: u8 = if i == top_idx { 2 } else { 1 };
        let before = read_attr(player, name);
        let after = before.saturating_add(bump).min(99);
        write_attr(player, name, after);
        changes.push((*name, (after as i16 - before as i16) as i8));
    }

    player.overall = player.ratings.overall_for(player.primary_position);
    TrainingDelta {
        attributes_changed: changes,
        new_overall: player.overall,
    }
}

fn read_attr(player: &Player, name: &str) -> u8 {
    let r = &player.ratings;
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
        _ => unreachable!("unknown attribute name in training cluster: {}", name),
    }
}

fn write_attr(player: &mut Player, name: &str, val: u8) {
    let r = &mut player.ratings;
    match name {
        "close_shot" => r.close_shot = val,
        "driving_layup" => r.driving_layup = val,
        "driving_dunk" => r.driving_dunk = val,
        "standing_dunk" => r.standing_dunk = val,
        "post_control" => r.post_control = val,
        "mid_range" => r.mid_range = val,
        "three_point" => r.three_point = val,
        "free_throw" => r.free_throw = val,
        "passing_accuracy" => r.passing_accuracy = val,
        "ball_handle" => r.ball_handle = val,
        "speed_with_ball" => r.speed_with_ball = val,
        "interior_defense" => r.interior_defense = val,
        "perimeter_defense" => r.perimeter_defense = val,
        "steal" => r.steal = val,
        "block" => r.block = val,
        "off_reb" => r.off_reb = val,
        "def_reb" => r.def_reb = val,
        "speed" => r.speed = val,
        "agility" => r.agility = val,
        "strength" => r.strength = val,
        "vertical" => r.vertical = val,
        _ => unreachable!("unknown attribute name in training cluster: {}", name),
    }
}
