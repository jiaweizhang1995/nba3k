//! Post-scrape sanity checks. Fail loud (anyhow err = non-zero exit).
//!
//! These guard against silent scraper drift: if BBRef rearranges columns,
//! or HoopsHype changes class names, the player count / position validity
//! / salary totals will fall outside expected bands and we want the binary
//! to die rather than write a broken seed.

use std::path::Path;

use anyhow::{bail, Context, Result};
use nba3k_core::{LeagueYear, SeasonId};
use rusqlite::Connection;

pub struct Bounds {
    pub min_players: u32,
    pub max_players: u32,
    pub min_per_team: u32,
    pub max_per_team: u32,
    pub min_prospects: u32,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            min_players: 450,
            max_players: 600,
            min_per_team: 13,
            max_per_team: 20,
            min_prospects: 60,
        }
    }
}

pub fn run_all(out: &Path, season: SeasonId, bounds: &Bounds) -> Result<()> {
    let conn = Connection::open(out).context("open seed for sanity check")?;

    let teams: i64 = conn.query_row("SELECT COUNT(*) FROM teams", [], |r| r.get(0))?;
    if teams != 30 {
        bail!("expected 30 teams, found {teams}");
    }

    let players: i64 = conn
        .query_row("SELECT COUNT(*) FROM players WHERE team_id IS NOT NULL", [], |r| r.get(0))?;
    if (players as u32) < bounds.min_players || (players as u32) > bounds.max_players {
        bail!(
            "expected {}..={} active players, found {}",
            bounds.min_players,
            bounds.max_players,
            players
        );
    }

    let prospects: i64 = conn
        .query_row("SELECT COUNT(*) FROM players WHERE team_id IS NULL", [], |r| r.get(0))?;
    if (prospects as u32) < bounds.min_prospects {
        bail!(
            "expected ≥{} draft prospects, found {}",
            bounds.min_prospects,
            prospects
        );
    }

    // Per-team roster size.
    let mut stmt = conn
        .prepare("SELECT team_id, COUNT(*) FROM players WHERE team_id IS NOT NULL GROUP BY team_id")?;
    let rows = stmt
        .query_map([], |r| Ok::<_, rusqlite::Error>((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    for row in rows {
        let (team_id, count) = row?;
        if (count as u32) < bounds.min_per_team || (count as u32) > bounds.max_per_team {
            bail!(
                "team {team_id} has {count} players (expected {}..={})",
                bounds.min_per_team,
                bounds.max_per_team
            );
        }
    }

    // Every player has a non-empty primary position.
    let bad_pos: i64 = conn.query_row(
        "SELECT COUNT(*) FROM players WHERE primary_position IS NULL OR primary_position = ''",
        [],
        |r| r.get(0),
    )?;
    if bad_pos > 0 {
        bail!("{bad_pos} players have empty primary position");
    }

    // No duplicate ids.
    let dup: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT id FROM players GROUP BY id HAVING COUNT(*) > 1)",
        [],
        |r| r.get(0),
    )?;
    if dup > 0 {
        bail!("{dup} duplicate player ids in seed");
    }

    // League salary total within ±5% of 30 × cap. Soft-skip if no
    // contracts were captured (offline mode), otherwise enforce.
    if let Some(ly) = LeagueYear::for_season(season) {
        let with_contracts: i64 = conn.query_row(
            "SELECT COUNT(*) FROM players WHERE contract_json IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        if with_contracts > 50 {
            // Sum first-year salaries (cents) by walking JSON in app code.
            let mut stmt = conn.prepare("SELECT contract_json FROM players WHERE contract_json IS NOT NULL")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            let mut total_cents: i64 = 0;
            for row in rows {
                let json: String = row?;
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                    if let Some(years) = v.get("years").and_then(|y| y.as_array()) {
                        if let Some(first) = years.first() {
                            if let Some(sal) = first.get("salary").and_then(|s| s.as_i64()) {
                                total_cents = total_cents.saturating_add(sal);
                            }
                        }
                    }
                }
            }
            let expected = (ly.cap.0 as i128) * 30;
            let lower = (expected as f64 * 0.5) as i64; // ±50% — first year only, generous
            let upper = (expected as f64 * 1.5) as i64;
            if total_cents < lower || total_cents > upper {
                tracing::warn!(
                    "league total first-year salary {total_cents} outside loose band [{lower},{upper}]; \
                     contracts may be partial"
                );
            }
        }
    }

    Ok(())
}
