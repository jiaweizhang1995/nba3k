//! `data/rating_overrides.toml` — hand-tuneable rating, contract, and
//! kicker overrides keyed by player name. Optional; absent file = no
//! overrides applied.
//!
//! Schema (all fields optional):
//!
//! ```toml
//! [[player]]
//! name = "Jayson Tatum"
//! overall = 95          # force overall to 95
//! potential = 96
//! no_trade_clause = true
//! trade_kicker_pct = 15
//! ```
//!
//! The trade engine in M3 reads `no_trade_clause` and `trade_kicker_pct`
//! off the saved `Player`; this is where they enter the system.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlayerOverride {
    pub name: String,
    pub overall: Option<u8>,
    pub potential: Option<u8>,
    pub no_trade_clause: Option<bool>,
    pub trade_kicker_pct: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct OverridesFile {
    #[serde(default)]
    player: Vec<PlayerOverride>,
}

#[derive(Debug, Default)]
pub struct OverridesIndex {
    by_name: HashMap<String, PlayerOverride>,
}

impl OverridesIndex {
    pub fn load_or_empty(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read overrides {path:?}"))?;
        let file: OverridesFile = toml::from_str(&text).context("parse overrides toml")?;
        let mut by_name = HashMap::with_capacity(file.player.len());
        for o in file.player {
            by_name.insert(o.name.to_lowercase(), o);
        }
        Ok(Self { by_name })
    }

    pub fn get(&self, name: &str) -> Option<&PlayerOverride> {
        self.by_name.get(&name.to_lowercase())
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}
