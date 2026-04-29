use crate::{Contract, PlayerId, TeamId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Position {
    PG,
    SG,
    SF,
    PF,
    C,
}

impl Position {
    pub fn all() -> [Self; 5] {
        [Self::PG, Self::SG, Self::SF, Self::PF, Self::C]
    }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use `f.pad` so format width / fill / align specs (e.g.
        // `{:<2}` in the trade-builder asset row) work as expected.
        // Plain `write!(f, "{}", ...)` would discard them and leave
        // the single-char `C` row a column short of the others.
        f.pad(match self {
            Self::PG => "PG",
            Self::SG => "SG",
            Self::SF => "SF",
            Self::PF => "PF",
            Self::C => "C",
        })
    }
}

/// 21-attribute 6-category rating schema borrowed from NBA 2K.
/// All fields are 0..=99. See `RESEARCH-NBA2K.md` § 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Ratings {
    // ---- Inside Scoring (5)
    pub close_shot: u8,
    pub driving_layup: u8,
    pub driving_dunk: u8,
    pub standing_dunk: u8,
    pub post_control: u8,
    // ---- Ranged Shooting (3)
    pub mid_range: u8,
    pub three_point: u8,
    pub free_throw: u8,
    // ---- Handling (3)
    pub passing_accuracy: u8,
    pub ball_handle: u8,
    pub speed_with_ball: u8,
    // ---- Defense (4)
    pub interior_defense: u8,
    pub perimeter_defense: u8,
    pub steal: u8,
    pub block: u8,
    // ---- Rebounding (2)
    pub off_reb: u8,
    pub def_reb: u8,
    // ---- Athleticism (4)
    pub speed: u8,
    pub agility: u8,
    pub strength: u8,
    pub vertical: u8,
}

impl Ratings {
    /// Position-aware weighted overall. Mirrors the 2KLab heat-map intuition
    /// without copying their numeric weights — guards weight handling +
    /// shooting; bigs weight rebounding + interior; wings sit between.
    pub fn overall_for(&self, pos: Position) -> u8 {
        let inside = (self.close_shot as u32
            + self.driving_layup as u32
            + self.driving_dunk as u32
            + self.standing_dunk as u32
            + self.post_control as u32)
            / 5;
        let shooting =
            (self.mid_range as u32 + self.three_point as u32 + self.free_throw as u32) / 3;
        let handling =
            (self.passing_accuracy as u32 + self.ball_handle as u32 + self.speed_with_ball as u32)
                / 3;
        let defense = (self.interior_defense as u32
            + self.perimeter_defense as u32
            + self.steal as u32
            + self.block as u32)
            / 4;
        let rebounding = (self.off_reb as u32 + self.def_reb as u32) / 2;
        let athletic =
            (self.speed as u32 + self.agility as u32 + self.strength as u32 + self.vertical as u32)
                / 4;

        // Rebalanced 2026-04-26 (M19.1 realism patch): centers were over-weighted
        // on rebounding+interior-defense (53% combined) which inverted role-player
        // bigs above wing stars. New weights flatten the C bias and bump shooting
        // for SF/PF so volume scorers (Tatum, Booker) land where reality puts them.
        let (w_in, w_sh, w_ha, w_de, w_re, w_at) = match pos {
            Position::PG => (5, 27, 28, 18, 5, 17),
            Position::SG => (10, 32, 18, 18, 6, 16),
            Position::SF => (15, 26, 14, 18, 12, 15),
            Position::PF => (20, 18, 8, 22, 18, 14),
            Position::C => (22, 12, 8, 20, 18, 20),
        };
        let total = inside * w_in
            + shooting * w_sh
            + handling * w_ha
            + defense * w_de
            + rebounding * w_re
            + athletic * w_at;
        (total / 100).min(99) as u8
    }

    /// Position-agnostic overall — flat average across all 21 attributes.
    /// Use only when position is genuinely unknown; prefer `overall_for`.
    pub fn overall_estimate(&self) -> u8 {
        self.overall_for(Position::SF)
    }

    /// Bridge constructor taking the legacy M2 8-attribute shape. Spreads
    /// each legacy value across its 21-attribute cluster. Used by test
    /// fixtures while M5 lands; new code should populate fields directly.
    #[allow(clippy::too_many_arguments)]
    pub fn legacy(
        shooting_3: u8,
        shooting_mid: u8,
        finishing: u8,
        playmaking: u8,
        rebound: u8,
        defense_perimeter: u8,
        defense_interior: u8,
        athletic: u8,
    ) -> Self {
        Self {
            close_shot: finishing,
            driving_layup: finishing,
            driving_dunk: finishing,
            standing_dunk: finishing,
            post_control: finishing,
            mid_range: shooting_mid,
            three_point: shooting_3,
            free_throw: shooting_mid,
            passing_accuracy: playmaking,
            ball_handle: playmaking,
            speed_with_ball: ((athletic as u32 + playmaking as u32) / 2) as u8,
            interior_defense: defense_interior,
            perimeter_defense: defense_perimeter,
            steal: defense_perimeter,
            block: defense_interior,
            off_reb: rebound,
            def_reb: rebound,
            speed: athletic,
            agility: athletic,
            strength: athletic,
            vertical: athletic,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjuryStatus {
    pub description: String,
    pub games_remaining: u16,
    pub severity: InjurySeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InjurySeverity {
    DayToDay,
    ShortTerm,
    LongTerm,
    SeasonEnding,
}

/// Player role per NBA 2K MyGM/MyNBA. Drives morale, promised-PT contract
/// clauses, and chemistry calc (role-vs-archetype mismatch). The variant
/// `Prospect` exists for draftees / two-way G-League call-ups.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlayerRole {
    Star,
    Starter,
    SixthMan,
    #[default]
    RolePlayer,
    BenchWarmer,
    Prospect,
}

impl std::fmt::Display for PlayerRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Star => "Star",
                Self::Starter => "Starter",
                Self::SixthMan => "SixthMan",
                Self::RolePlayer => "RolePlayer",
                Self::BenchWarmer => "BenchWarmer",
                Self::Prospect => "Prospect",
            }
        )
    }
}

fn default_morale() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub primary_position: Position,
    pub secondary_position: Option<Position>,
    pub age: u8,
    pub overall: u8,
    pub potential: u8,
    pub ratings: Ratings,
    pub contract: Option<Contract>,
    pub team: Option<TeamId>,
    pub injury: Option<InjuryStatus>,
    pub no_trade_clause: bool,
    /// Trade kicker percentage (0..=15 typically).
    pub trade_kicker_pct: Option<u8>,
    /// 2K-style role tag. Default `RolePlayer`. Mismatched roles
    /// (e.g. Star slotted as BenchWarmer) drive morale + chemistry penalties.
    #[serde(default)]
    pub role: PlayerRole,
    /// 0.0..=1.0. Updated by season events (PT below role expectation,
    /// role mismatch, contract incident). Default 0.5.
    #[serde(default = "default_morale")]
    pub morale: f32,
}

impl Player {
    /// Mutate role. Triggers morale drift when the new role is below the
    /// player's standing (Star demoted to BenchWarmer = -0.4 morale).
    pub fn set_role(&mut self, new_role: PlayerRole) {
        let drift = role_morale_drift(self.role, new_role);
        self.role = new_role;
        self.morale = (self.morale + drift).clamp(0.0, 1.0);
    }

    /// Bound morale to `[0.0, 1.0]`. Use after manual mutation.
    pub fn clamp_morale(&mut self) {
        self.morale = self.morale.clamp(0.0, 1.0);
    }
}

/// Morale shift when a player's role changes. Demotion = negative,
/// promotion = small positive. Demotions hurt ~2× more than promotions help —
/// matches 2K MyGM published behavior where Star→BenchWarmer is a
/// near-instant trade-request trigger. Same-role assignment is a no-op.
pub fn role_morale_drift(old: PlayerRole, new: PlayerRole) -> f32 {
    if old == new {
        return 0.0;
    }
    let rank = |r: PlayerRole| -> i32 {
        match r {
            PlayerRole::Star => 5,
            PlayerRole::Starter => 4,
            PlayerRole::SixthMan => 3,
            PlayerRole::RolePlayer => 2,
            PlayerRole::BenchWarmer => 1,
            PlayerRole::Prospect => 2,
        }
    };
    let rank_delta = rank(new) - rank(old);
    if rank_delta < 0 {
        // Star (5) → Bench (1) = -4 ranks → -0.40 morale (matches 2K).
        (rank_delta as f32) * 0.10
    } else if rank_delta > 0 {
        // Bench (1) → Star (5) = +4 ranks → +0.40 morale (symmetric).
        (rank_delta as f32) * 0.10
    } else {
        0.0
    }
}

/// Per-player season-to-date aggregates (M32). Lives in the
/// `player_season_stats` table; populated by the "Start From Today"
/// importer from ESPN's `byathlete` stats endpoint. When present, the
/// `records --scope season` command prefers these rows over on-the-fly
/// aggregation from box scores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSeasonStats {
    pub player_id: PlayerId,
    pub season_year: u16,
    pub gp: u16,
    pub mpg: f32,
    pub ppg: f32,
    pub rpg: f32,
    pub apg: f32,
    pub spg: f32,
    pub bpg: f32,
    pub fg_pct: f32,
    pub three_pct: f32,
    pub ft_pct: f32,
    pub ts_pct: f32,
    pub usage: f32,
}
