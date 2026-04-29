//! Tunable weights for every model. Every weights struct ships hardcoded
//! defaults; the TOML file at `data/realism_weights.toml` is purely an
//! override layer. Missing file = use defaults.
//!
//! Workers should add to their respective struct. Keep keys flat-named so
//! TOML stays readable.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerValueWeights {
    pub age_peak: f32,
    pub star_threshold_ovr: u8,
    pub star_premium_max: f32,
    pub salary_aversion_default: f32,
    pub loyalty_bonus_default: f32,
}

impl Default for PlayerValueWeights {
    fn default() -> Self {
        Self {
            age_peak: 27.0,
            star_threshold_ovr: 88,
            star_premium_max: 1.6,
            salary_aversion_default: 1.0,
            loyalty_bonus_default: 0.20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractValueWeights {
    pub future_year_discount_pct: f32,
    pub option_value_player: f32,
    pub option_value_team: f32,
    pub expiring_premium_pct: f32,
}

impl Default for ContractValueWeights {
    fn default() -> Self {
        Self {
            future_year_discount_pct: 0.08,
            option_value_player: 0.10,
            option_value_team: -0.05,
            expiring_premium_pct: 0.15,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamContextWeights {
    pub young_age_threshold: f32,
    pub veteran_age_threshold: f32,
    pub star_ovr: u8,
    pub keeper_ovr: u8,
}

impl Default for TeamContextWeights {
    fn default() -> Self {
        Self {
            young_age_threshold: 25.0,
            veteran_age_threshold: 29.0,
            star_ovr: 88,
            keeper_ovr: 82,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarProtectionWeights {
    pub franchise_tag_value: f32,
    pub top_ovr_bump: f32,
    pub young_ascending_bump: f32,
    pub recent_signing_bump: f32,
    pub absolute_threshold: f32, // ≥ this = hard reject
    pub premium_threshold: f32,  // [premium, absolute) = require value premium
}

impl Default for StarProtectionWeights {
    fn default() -> Self {
        Self {
            franchise_tag_value: 1.0,
            top_ovr_bump: 0.4,
            young_ascending_bump: 0.3,
            recent_signing_bump: 0.2,
            absolute_threshold: 0.85,
            premium_threshold: 0.60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetFitWeights {
    pub positional_need_max: f32,
    pub skill_overlap_penalty: f32,
    pub rotation_saturation_penalty: f32,
}

impl Default for AssetFitWeights {
    fn default() -> Self {
        Self {
            positional_need_max: 0.20,
            skill_overlap_penalty: 0.15,
            rotation_saturation_penalty: 0.10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeAcceptanceWeights {
    pub accept_probability_intercept: f64,
    pub accept_probability_slope: f64,
    pub gullibility_noise_pct: f64,
    pub top_k_reasons: usize,
}

impl Default for TradeAcceptanceWeights {
    fn default() -> Self {
        Self {
            accept_probability_intercept: 0.0,
            accept_probability_slope: 8.0,
            gullibility_noise_pct: 0.08,
            top_k_reasons: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatProjectionWeights {
    pub star_uplift_threshold_ovr: u8,
    pub star_uplift_pts: f32,
    pub star_uplift_reb: f32,
    pub star_uplift_ast: f32,
    pub triple_double_floor_minutes: u8,
    /// Primary-creator bonus on per-100 ast for each unit of usage above 0.25.
    /// Real NBA: top creators (Doncic, Halliburton) get +5 ast/100 at peak usage.
    #[serde(default = "default_creator_ast_bonus")]
    pub creator_ast_bonus_per_excess: f32,
    /// Primary-creator rebound bonus mirroring `creator_ast_bonus_per_excess`.
    #[serde(default = "default_creator_reb_bonus")]
    pub creator_reb_bonus_per_excess: f32,
    /// Hard ceiling on a single player's PTS in a single game (stops PTS-
    /// reconciliation from launching a 60-PPG average across a season).
    #[serde(default = "default_pts_cap")]
    pub single_game_pts_cap: u8,
}

fn default_creator_ast_bonus() -> f32 {
    5.0
}
fn default_creator_reb_bonus() -> f32 {
    6.0
}
fn default_pts_cap() -> u8 {
    55
}

impl Default for StatProjectionWeights {
    fn default() -> Self {
        Self {
            star_uplift_threshold_ovr: 88,
            star_uplift_pts: 4.0,
            star_uplift_reb: 1.5,
            star_uplift_ast: 1.5,
            triple_double_floor_minutes: 32,
            creator_ast_bonus_per_excess: default_creator_ast_bonus(),
            creator_reb_bonus_per_excess: default_creator_reb_bonus(),
            single_game_pts_cap: default_pts_cap(),
        }
    }
}

/// Aggregate file. Each section is optional; missing → defaults.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RealismWeightsFile {
    #[serde(default)]
    pub player_value: Option<PlayerValueWeights>,
    #[serde(default)]
    pub contract_value: Option<ContractValueWeights>,
    #[serde(default)]
    pub team_context: Option<TeamContextWeights>,
    #[serde(default)]
    pub star_protection: Option<StarProtectionWeights>,
    #[serde(default)]
    pub asset_fit: Option<AssetFitWeights>,
    #[serde(default)]
    pub trade_acceptance: Option<TradeAcceptanceWeights>,
    #[serde(default)]
    pub stat_projection: Option<StatProjectionWeights>,
}

#[derive(Debug, Clone)]
pub struct RealismWeights {
    pub player_value: PlayerValueWeights,
    pub contract_value: ContractValueWeights,
    pub team_context: TeamContextWeights,
    pub star_protection: StarProtectionWeights,
    pub asset_fit: AssetFitWeights,
    pub trade_acceptance: TradeAcceptanceWeights,
    pub stat_projection: StatProjectionWeights,
}

impl Default for RealismWeights {
    fn default() -> Self {
        Self {
            player_value: PlayerValueWeights::default(),
            contract_value: ContractValueWeights::default(),
            team_context: TeamContextWeights::default(),
            star_protection: StarProtectionWeights::default(),
            asset_fit: AssetFitWeights::default(),
            trade_acceptance: TradeAcceptanceWeights::default(),
            stat_projection: StatProjectionWeights::default(),
        }
    }
}

impl RealismWeights {
    pub fn merge_overrides(file: RealismWeightsFile) -> Self {
        let d = Self::default();
        Self {
            player_value: file.player_value.unwrap_or(d.player_value),
            contract_value: file.contract_value.unwrap_or(d.contract_value),
            team_context: file.team_context.unwrap_or(d.team_context),
            star_protection: file.star_protection.unwrap_or(d.star_protection),
            asset_fit: file.asset_fit.unwrap_or(d.asset_fit),
            trade_acceptance: file.trade_acceptance.unwrap_or(d.trade_acceptance),
            stat_projection: file.stat_projection.unwrap_or(d.stat_projection),
        }
    }
}

/// Load weights from TOML. Missing file → defaults. Malformed → error.
pub fn load_or_default(path: &Path) -> crate::ModelResult<RealismWeights> {
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RealismWeights::default()),
        Err(e) => return Err(e.into()),
    };
    let file: RealismWeightsFile = toml::from_str(&text)?;
    Ok(RealismWeights::merge_overrides(file))
}
