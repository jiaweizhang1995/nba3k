use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GMArchetype {
    Analytics,
    OldSchool,
    StarHunter,
    Rebuilder,
    WinNow,
    Loyalist,
    Cheapskate,
    Aggressive,
    Conservative,
    Homer,
    Wildcard,
}

/// All weights are unitless multipliers/biases. Defaults are "neutral".
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GMTraits {
    /// Positive = prefers youth.
    pub age_curve_weight: f32,
    pub potential_weight: f32,
    pub current_overall_weight: f32,
    pub pick_value_multiplier: f32,
    pub salary_aversion: f32,
    pub tax_aversion: f32,
    pub risk_tolerance: f32,
    pub loyalty: f32,
    pub patience: f32,
    pub aggression: f32,
    /// Higher = worse at evaluating (Wildcard high).
    pub gullibility: f32,
    pub star_premium: f32,
    pub fit_weight: f32,
}

impl Default for GMTraits {
    fn default() -> Self {
        Self {
            age_curve_weight: 0.0,
            potential_weight: 1.0,
            current_overall_weight: 1.0,
            pick_value_multiplier: 1.0,
            salary_aversion: 1.0,
            tax_aversion: 1.0,
            risk_tolerance: 0.5,
            loyalty: 0.1,
            patience: 0.5,
            aggression: 0.5,
            gullibility: 0.1,
            star_premium: 1.0,
            fit_weight: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GMPersonality {
    pub name: String,
    pub archetype: GMArchetype,
    pub traits: GMTraits,
}

impl GMPersonality {
    pub fn from_archetype(name: impl Into<String>, archetype: GMArchetype) -> Self {
        let mut traits = GMTraits::default();
        match archetype {
            GMArchetype::Analytics => {
                traits.age_curve_weight = 0.6;
                traits.potential_weight = 1.4;
                traits.pick_value_multiplier = 1.3;
            }
            GMArchetype::OldSchool => {
                traits.age_curve_weight = -0.3;
                traits.fit_weight = 0.8;
                traits.risk_tolerance = 0.3;
            }
            GMArchetype::StarHunter => {
                traits.star_premium = 1.6;
                traits.salary_aversion = 0.6;
            }
            GMArchetype::Rebuilder => {
                traits.age_curve_weight = 0.8;
                traits.potential_weight = 1.7;
                traits.current_overall_weight = 0.6;
                traits.patience = 1.0;
            }
            GMArchetype::WinNow => {
                traits.current_overall_weight = 1.5;
                traits.pick_value_multiplier = 0.7;
                traits.patience = 0.1;
            }
            GMArchetype::Loyalist => {
                traits.loyalty = 0.6;
            }
            GMArchetype::Cheapskate => {
                traits.salary_aversion = 1.8;
                traits.tax_aversion = 2.0;
            }
            GMArchetype::Aggressive => {
                traits.aggression = 1.0;
                traits.risk_tolerance = 0.9;
            }
            GMArchetype::Conservative => {
                traits.aggression = 0.15;
                traits.risk_tolerance = 0.2;
            }
            GMArchetype::Homer => {
                traits.fit_weight = 1.2;
                traits.loyalty = 0.4;
            }
            GMArchetype::Wildcard => {
                traits.gullibility = 0.7;
                traits.risk_tolerance = 0.95;
            }
        }
        Self { name: name.into(), archetype, traits }
    }
}
