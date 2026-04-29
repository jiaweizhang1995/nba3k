//! Static 30-team table for the modern NBA. Hardcoded because BBRef IDs are
//! stable strings and we want determinism + zero-network bootstrap. The
//! `id` field becomes `TeamId(u8)` in the seed DB.

use nba3k_core::{Coach, Conference, Division, GMArchetype, GMPersonality, Team, TeamId};

#[derive(Debug, Clone, Copy)]
pub struct TeamRow {
    pub id: u8,
    /// BBRef-style three-letter abbreviation.
    pub abbrev: &'static str,
    pub city: &'static str,
    pub name: &'static str,
    pub conf: Conference,
    pub div: Division,
}

/// 30 NBA teams, ordered alphabetically by abbreviation for determinism.
pub const TEAMS: &[TeamRow] = &[
    TeamRow {
        id: 1,
        abbrev: "ATL",
        city: "Atlanta",
        name: "Hawks",
        conf: Conference::East,
        div: Division::Southeast,
    },
    TeamRow {
        id: 2,
        abbrev: "BOS",
        city: "Boston",
        name: "Celtics",
        conf: Conference::East,
        div: Division::Atlantic,
    },
    TeamRow {
        id: 3,
        abbrev: "BRK",
        city: "Brooklyn",
        name: "Nets",
        conf: Conference::East,
        div: Division::Atlantic,
    },
    TeamRow {
        id: 4,
        abbrev: "CHO",
        city: "Charlotte",
        name: "Hornets",
        conf: Conference::East,
        div: Division::Southeast,
    },
    TeamRow {
        id: 5,
        abbrev: "CHI",
        city: "Chicago",
        name: "Bulls",
        conf: Conference::East,
        div: Division::Central,
    },
    TeamRow {
        id: 6,
        abbrev: "CLE",
        city: "Cleveland",
        name: "Cavaliers",
        conf: Conference::East,
        div: Division::Central,
    },
    TeamRow {
        id: 7,
        abbrev: "DAL",
        city: "Dallas",
        name: "Mavericks",
        conf: Conference::West,
        div: Division::Southwest,
    },
    TeamRow {
        id: 8,
        abbrev: "DEN",
        city: "Denver",
        name: "Nuggets",
        conf: Conference::West,
        div: Division::Northwest,
    },
    TeamRow {
        id: 9,
        abbrev: "DET",
        city: "Detroit",
        name: "Pistons",
        conf: Conference::East,
        div: Division::Central,
    },
    TeamRow {
        id: 10,
        abbrev: "GSW",
        city: "Golden State",
        name: "Warriors",
        conf: Conference::West,
        div: Division::Pacific,
    },
    TeamRow {
        id: 11,
        abbrev: "HOU",
        city: "Houston",
        name: "Rockets",
        conf: Conference::West,
        div: Division::Southwest,
    },
    TeamRow {
        id: 12,
        abbrev: "IND",
        city: "Indiana",
        name: "Pacers",
        conf: Conference::East,
        div: Division::Central,
    },
    TeamRow {
        id: 13,
        abbrev: "LAC",
        city: "Los Angeles",
        name: "Clippers",
        conf: Conference::West,
        div: Division::Pacific,
    },
    TeamRow {
        id: 14,
        abbrev: "LAL",
        city: "Los Angeles",
        name: "Lakers",
        conf: Conference::West,
        div: Division::Pacific,
    },
    TeamRow {
        id: 15,
        abbrev: "MEM",
        city: "Memphis",
        name: "Grizzlies",
        conf: Conference::West,
        div: Division::Southwest,
    },
    TeamRow {
        id: 16,
        abbrev: "MIA",
        city: "Miami",
        name: "Heat",
        conf: Conference::East,
        div: Division::Southeast,
    },
    TeamRow {
        id: 17,
        abbrev: "MIL",
        city: "Milwaukee",
        name: "Bucks",
        conf: Conference::East,
        div: Division::Central,
    },
    TeamRow {
        id: 18,
        abbrev: "MIN",
        city: "Minnesota",
        name: "Timberwolves",
        conf: Conference::West,
        div: Division::Northwest,
    },
    TeamRow {
        id: 19,
        abbrev: "NOP",
        city: "New Orleans",
        name: "Pelicans",
        conf: Conference::West,
        div: Division::Southwest,
    },
    TeamRow {
        id: 20,
        abbrev: "NYK",
        city: "New York",
        name: "Knicks",
        conf: Conference::East,
        div: Division::Atlantic,
    },
    TeamRow {
        id: 21,
        abbrev: "OKC",
        city: "Oklahoma City",
        name: "Thunder",
        conf: Conference::West,
        div: Division::Northwest,
    },
    TeamRow {
        id: 22,
        abbrev: "ORL",
        city: "Orlando",
        name: "Magic",
        conf: Conference::East,
        div: Division::Southeast,
    },
    TeamRow {
        id: 23,
        abbrev: "PHI",
        city: "Philadelphia",
        name: "76ers",
        conf: Conference::East,
        div: Division::Atlantic,
    },
    TeamRow {
        id: 24,
        abbrev: "PHO",
        city: "Phoenix",
        name: "Suns",
        conf: Conference::West,
        div: Division::Pacific,
    },
    TeamRow {
        id: 25,
        abbrev: "POR",
        city: "Portland",
        name: "Trail Blazers",
        conf: Conference::West,
        div: Division::Northwest,
    },
    TeamRow {
        id: 26,
        abbrev: "SAC",
        city: "Sacramento",
        name: "Kings",
        conf: Conference::West,
        div: Division::Pacific,
    },
    TeamRow {
        id: 27,
        abbrev: "SAS",
        city: "San Antonio",
        name: "Spurs",
        conf: Conference::West,
        div: Division::Southwest,
    },
    TeamRow {
        id: 28,
        abbrev: "TOR",
        city: "Toronto",
        name: "Raptors",
        conf: Conference::East,
        div: Division::Atlantic,
    },
    TeamRow {
        id: 29,
        abbrev: "UTA",
        city: "Utah",
        name: "Jazz",
        conf: Conference::West,
        div: Division::Northwest,
    },
    TeamRow {
        id: 30,
        abbrev: "WAS",
        city: "Washington",
        name: "Wizards",
        conf: Conference::East,
        div: Division::Southeast,
    },
];

pub fn build_team(row: TeamRow) -> Team {
    Team {
        id: TeamId(row.id),
        abbrev: row.abbrev.to_string(),
        city: row.city.to_string(),
        name: row.name.to_string(),
        conference: row.conf,
        division: row.div,
        gm: GMPersonality::from_archetype(format!("{} GM", row.city), GMArchetype::Conservative),
        roster: vec![],
        draft_picks: vec![],
        coach: Coach::default_for(row.abbrev),
    }
}

pub fn lookup_by_abbrev(abbrev: &str) -> Option<TeamRow> {
    TEAMS
        .iter()
        .copied()
        .find(|t| t.abbrev.eq_ignore_ascii_case(abbrev))
}
