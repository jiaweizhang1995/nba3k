pub mod bbref;
pub mod espn;
pub mod hoophype;
pub mod mock_draft;
pub mod nba_api;

use nba3k_core::Position;

#[derive(Debug, Clone)]
pub struct RawPlayerStats {
    pub name: String,
    /// "PG" / "SG" / etc.
    pub primary_position: Position,
    pub secondary_position: Option<Position>,
    pub age: u8,
    pub games: f32,
    pub minutes_per_game: f32,
    pub pts: f32,
    pub trb: f32,
    pub ast: f32,
    pub stl: f32,
    pub blk: f32,
    pub tov: f32,
    pub fg_pct: f32,
    pub three_pct: f32,
    pub ft_pct: f32,
    /// Estimated usage rate, 0..=1. Optional — fall back to fixed default if missing.
    pub usage: Option<f32>,
}

pub fn normalize_player_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphabetic())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Map a raw position string ("PG", "SG-SF", "G", "F-C") to a primary +
/// optional secondary `Position`. Used by both BBRef and nba_api parsers.
pub fn parse_position(s: &str) -> (Position, Option<Position>) {
    let upper = s.trim().to_uppercase();
    let parts: Vec<&str> = upper
        .split(['-', '/', ' '])
        .filter(|p| !p.is_empty())
        .collect();
    let mut codes: Vec<Position> = parts.iter().filter_map(|p| code_to_position(p)).collect();
    if codes.is_empty() {
        codes.push(Position::SF);
    }
    let primary = codes[0];
    let secondary = codes.get(1).copied();
    (primary, secondary)
}

fn code_to_position(s: &str) -> Option<Position> {
    Some(match s {
        "PG" => Position::PG,
        "SG" => Position::SG,
        "SF" => Position::SF,
        "PF" => Position::PF,
        "C" => Position::C,
        // Generic single-letter — nba_api sometimes returns just G/F
        "G" => Position::SG,
        "F" => Position::SF,
        _ => return None,
    })
}
