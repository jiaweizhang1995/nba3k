use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ParamsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimParams {
    pub pace_mean: f32,
    pub pace_sigma: f32,
    pub score_sigma: f32,
    pub home_court_advantage: f32,
    pub injury_rate_per_game: f32,
    pub max_overtimes: u8,
    pub usage_distribution_alpha: f32,
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            pace_mean: 99.0,
            pace_sigma: 3.0,
            score_sigma: 7.5,
            home_court_advantage: 2.0,
            injury_rate_per_game: 0.005,
            max_overtimes: 4,
            usage_distribution_alpha: 1.4,
        }
    }
}

impl SimParams {
    pub fn from_toml(path: impl AsRef<Path>) -> Result<Self, ParamsError> {
        let raw = std::fs::read_to_string(path)?;
        Self::from_toml_str(&raw)
    }

    pub fn from_toml_str(s: &str) -> Result<Self, ParamsError> {
        Ok(toml::from_str(s)?)
    }
}
