use crate::{SeasonId, TeamId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeasonPhase {
    PreSeason,
    Regular,
    TradeDeadlinePassed,
    Playoffs,
    OffSeason,
    Draft,
    FreeAgency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameMode {
    Standard,
    God,
    Hardcore,
    Sandbox,
}

impl Default for GameMode {
    fn default() -> Self {
        Self::Standard
    }
}

impl GameMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "standard" | "std" => Some(Self::Standard),
            "god" => Some(Self::God),
            "hardcore" | "hc" => Some(Self::Hardcore),
            "sandbox" | "sb" => Some(Self::Sandbox),
            _ => None,
        }
    }

    pub fn enforces_cba(self) -> bool {
        matches!(self, Self::Standard | Self::Hardcore)
    }
}

impl std::fmt::Display for GameMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Standard => "standard",
                Self::God => "god",
                Self::Hardcore => "hardcore",
                Self::Sandbox => "sandbox",
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonState {
    pub season: SeasonId,
    pub phase: SeasonPhase,
    /// Sim day index from start of season.
    pub day: u32,
    pub user_team: TeamId,
    pub mode: GameMode,
    /// Seedable RNG for deterministic replays.
    pub rng_seed: u64,
}
