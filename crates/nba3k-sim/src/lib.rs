//! Game simulation engines.

use nba3k_core::{GameId, GameResult, PlayerId, Position, Ratings, SeasonId, TeamId};
use rand::RngCore;

pub mod engine;
pub mod params;

pub use engine::statistical::{roll_injuries_from_box, tick_injury, StatisticalEngine};
pub use params::SimParams;

/// One slot in a team's rotation. The StatisticalEngine reads `minutes_share`,
/// `usage`, and `ratings` to derive both team-level ratings and per-player
/// box-score distribution.
#[derive(Debug, Clone)]
pub struct RotationSlot {
    pub player: PlayerId,
    /// Player name — needed for franchise-tag lookup in the realism engine.
    pub name: String,
    pub position: Position,
    /// Fraction of available team minutes this player is expected to play
    /// (sum across rotation should be ~5.0 — five players on the floor).
    pub minutes_share: f32,
    /// Usage rate (fraction of team possessions this player finishes).
    /// Sum across rotation should be ~1.0 per 5-man-on-floor unit; engine
    /// renormalizes defensively.
    pub usage: f32,
    pub ratings: Ratings,
    pub age: u8,
    /// Overall rating — needed for the realism engine's star uplift gate.
    pub overall: u8,
    pub potential: u8,
}

#[derive(Debug, Clone)]
pub struct TeamSnapshot {
    pub id: TeamId,
    pub abbrev: String,
    pub overall: u8,
    pub home_court_advantage: f32,
    /// Top-of-rotation players (typically 8). Empty rotation falls back to
    /// `overall`-only sim — useful for smoke tests before rosters are wired.
    pub rotation: Vec<RotationSlot>,
}

#[derive(Debug, Clone)]
pub struct GameContext {
    pub game_id: GameId,
    pub season: SeasonId,
    pub date: chrono::NaiveDate,
    pub is_playoffs: bool,
    pub home_back_to_back: bool,
    pub away_back_to_back: bool,
}

pub trait Engine: Send + Sync {
    fn name(&self) -> &'static str;
    fn simulate_game(
        &self,
        home: &TeamSnapshot,
        away: &TeamSnapshot,
        ctx: &GameContext,
        rng: &mut dyn RngCore,
    ) -> GameResult;
}

/// Selects an engine implementation by short name. Unknown names fall back
/// to the default statistical engine.
pub fn pick_engine(name: &str) -> Box<dyn Engine> {
    match name.to_ascii_lowercase().as_str() {
        "statistical" | "stat" | "default" => Box::new(StatisticalEngine::with_defaults()),
        _ => Box::new(StatisticalEngine::with_defaults()),
    }
}
