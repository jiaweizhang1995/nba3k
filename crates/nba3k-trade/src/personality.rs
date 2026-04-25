//! Personality file loader (Worker B).
//!
//! `data/personalities.toml` is the canonical source of GM identity for every
//! NBA team. Each entry pins a `GMArchetype` and may override individual
//! `GMTraits` fields. We build a `GMPersonality` per team by starting from the
//! archetype defaults (`GMPersonality::from_archetype`) and patching the
//! overrides on top.
//!
//! If the TOML file is missing on disk, callers should fall back to
//! `embedded_personalities()` — every team gets a `Conservative` archetype so
//! the trade engine always has *something* to consult.

use crate::{TradeError, TradeResult};
use nba3k_core::{GMArchetype, GMPersonality, GMTraits};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// All 30 NBA team abbreviations as used by the seed data and CLI.
pub const NBA_TEAM_ABBREVS: [&str; 30] = [
    "ATL", "BOS", "BRK", "CHO", "CHI", "CLE", "DAL", "DEN", "DET", "GSW", "HOU", "IND", "LAC",
    "LAL", "MEM", "MIA", "MIL", "MIN", "NOP", "NYK", "OKC", "ORL", "PHI", "PHO", "POR", "SAC",
    "SAS", "TOR", "UTA", "WAS",
];

/// Fully-resolved personalities keyed by team abbreviation.
#[derive(Debug, Default, Clone)]
pub struct PersonalitiesFile {
    pub by_abbrev: HashMap<String, GMPersonality>,
}

impl PersonalitiesFile {
    pub fn get(&self, abbrev: &str) -> Option<&GMPersonality> {
        self.by_abbrev.get(abbrev)
    }

    pub fn len(&self) -> usize {
        self.by_abbrev.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_abbrev.is_empty()
    }
}

/// Raw TOML schema. Each top-level key is a team abbreviation.
#[derive(Debug, Deserialize)]
struct RawFile(HashMap<String, RawEntry>);

#[derive(Debug, Deserialize)]
struct RawEntry {
    archetype: GMArchetype,
    #[serde(default)]
    gm_name: Option<String>,
    #[serde(default)]
    traits: Option<RawTraitsOverride>,
}

/// Trait overrides — every field optional so the TOML only mentions deltas
/// from the archetype default. Mirror of `GMTraits` minus the totality.
#[derive(Debug, Deserialize, Default)]
struct RawTraitsOverride {
    age_curve_weight: Option<f32>,
    potential_weight: Option<f32>,
    current_overall_weight: Option<f32>,
    pick_value_multiplier: Option<f32>,
    salary_aversion: Option<f32>,
    tax_aversion: Option<f32>,
    risk_tolerance: Option<f32>,
    loyalty: Option<f32>,
    patience: Option<f32>,
    aggression: Option<f32>,
    gullibility: Option<f32>,
    star_premium: Option<f32>,
    fit_weight: Option<f32>,
}

impl RawTraitsOverride {
    fn apply(&self, base: &mut GMTraits) {
        if let Some(v) = self.age_curve_weight {
            base.age_curve_weight = v;
        }
        if let Some(v) = self.potential_weight {
            base.potential_weight = v;
        }
        if let Some(v) = self.current_overall_weight {
            base.current_overall_weight = v;
        }
        if let Some(v) = self.pick_value_multiplier {
            base.pick_value_multiplier = v;
        }
        if let Some(v) = self.salary_aversion {
            base.salary_aversion = v;
        }
        if let Some(v) = self.tax_aversion {
            base.tax_aversion = v;
        }
        if let Some(v) = self.risk_tolerance {
            base.risk_tolerance = v;
        }
        if let Some(v) = self.loyalty {
            base.loyalty = v;
        }
        if let Some(v) = self.patience {
            base.patience = v;
        }
        if let Some(v) = self.aggression {
            base.aggression = v;
        }
        if let Some(v) = self.gullibility {
            base.gullibility = v;
        }
        if let Some(v) = self.star_premium {
            base.star_premium = v;
        }
        if let Some(v) = self.fit_weight {
            base.fit_weight = v;
        }
    }
}

/// Load and validate the TOML file. Errors if any of the 30 NBA team abbrevs
/// is missing — partial files are not allowed because the trade engine
/// expects a personality lookup for every league member.
pub fn load_personalities(path: &Path) -> TradeResult<PersonalitiesFile> {
    let raw_text = std::fs::read_to_string(path)?;
    parse_personalities(&raw_text)
}

fn parse_personalities(text: &str) -> TradeResult<PersonalitiesFile> {
    let raw: RawFile = toml::from_str(text)?;
    let mut by_abbrev: HashMap<String, GMPersonality> = HashMap::with_capacity(raw.0.len());
    for (abbrev, entry) in raw.0 {
        let display_name = entry
            .gm_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&abbrev)
            .to_string();
        let mut personality = GMPersonality::from_archetype(display_name, entry.archetype);
        if let Some(overrides) = entry.traits.as_ref() {
            overrides.apply(&mut personality.traits);
        }
        by_abbrev.insert(abbrev, personality);
    }

    for required in NBA_TEAM_ABBREVS {
        if !by_abbrev.contains_key(required) {
            return Err(TradeError::MissingData(format!(
                "personalities.toml is missing entry for team `{}`",
                required
            )));
        }
    }

    Ok(PersonalitiesFile { by_abbrev })
}

/// Pure-Rust fallback used when the TOML file is missing on disk. Every team
/// gets the same `Conservative` archetype — boring but defensible default.
pub fn embedded_personalities() -> PersonalitiesFile {
    let mut by_abbrev = HashMap::with_capacity(NBA_TEAM_ABBREVS.len());
    for abbrev in NBA_TEAM_ABBREVS {
        let personality = GMPersonality::from_archetype(abbrev, GMArchetype::Conservative);
        by_abbrev.insert(abbrev.to_string(), personality);
    }
    PersonalitiesFile { by_abbrev }
}

/// Try to load from `path`; if the file is missing, return the embedded
/// fallback. Other errors (malformed TOML, missing teams) still propagate.
pub fn load_or_embedded(path: &Path) -> TradeResult<PersonalitiesFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_personalities(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(embedded_personalities()),
        Err(e) => Err(TradeError::Io(e)),
    }
}

/// Look up a team's personality, falling back to a `Conservative` archetype
/// keyed by the abbrev itself if (somehow) the file is incomplete.
pub fn personality_for(abbrev: &str, file: &PersonalitiesFile) -> GMPersonality {
    file.by_abbrev
        .get(abbrev)
        .cloned()
        .unwrap_or_else(|| GMPersonality::from_archetype(abbrev, GMArchetype::Conservative))
}
