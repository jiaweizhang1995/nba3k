//! `stats.nba.com` augmentation via Python `nba_api` shellout.
//!
//! There is no actively maintained Rust-native client (the `nba_api` crate
//! has 0 stars / 0 releases). We shell out to Python and parse JSON.
//!
//! If `python3 -c "import nba_api"` fails, this module returns
//! `MissingPython` and the caller surfaces a clean remediation message.

use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::cache::{json_ttl, Cache};

#[derive(Debug)]
pub enum NbaApiStatus {
    Available,
    MissingPython,
    MissingPackage,
}

pub fn probe() -> NbaApiStatus {
    let py = Command::new("python3").arg("--version").output();
    if py.is_err() {
        return NbaApiStatus::MissingPython;
    }
    let check = Command::new("python3")
        .args(["-c", "import nba_api"])
        .output();
    match check {
        Ok(o) if o.status.success() => NbaApiStatus::Available,
        _ => NbaApiStatus::MissingPackage,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerAdvanced {
    pub name: String,
    /// Usage rate (0..=1).
    pub usage: f32,
    /// True shooting %.
    pub ts_pct: f32,
}

/// Fetch a JSON dump of league-wide advanced stats for the given season
/// (e.g. "2025-26"). Returns the raw text bytes; the caller parses.
///
/// Caches under `nba_api/league_dash_advanced_<season>.json`.
pub fn fetch_league_advanced(cache: &Cache, season: &str) -> Result<Option<Vec<u8>>> {
    let key = format!("league_dash_advanced_{season}");
    if let Some(b) = cache.get("nba_api", &key, "json", json_ttl()) {
        return Ok(Some(b));
    }

    if !matches!(probe(), NbaApiStatus::Available) {
        return Ok(None);
    }

    let script = include_str!("../../py/league_dash_advanced.py");
    let out = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(season)
        .output()
        .context("spawn python3")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        tracing::warn!(stderr=?err, "nba_api script failed; continuing without it");
        return Ok(None);
    }
    cache.put("nba_api", &key, "json", &out.stdout)?;
    Ok(Some(out.stdout))
}

pub fn parse_league_advanced(bytes: &[u8]) -> Result<Vec<PlayerAdvanced>> {
    let raw: serde_json::Value = serde_json::from_slice(bytes).context("parse json")?;
    // The script normalizes to `[{name, usage, ts_pct}, ...]`.
    let arr = raw
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("expected json array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        let name = v
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let usage = v.get("usage").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
        let ts_pct = v.get("ts_pct").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
        if !name.is_empty() {
            out.push(PlayerAdvanced {
                name,
                usage,
                ts_pct,
            });
        }
    }
    Ok(out)
}
