use crate::{Coach, DraftPick, GMPersonality, PlayerId, TeamId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Conference {
    East,
    West,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Division {
    Atlantic,
    Central,
    Southeast,
    Northwest,
    Pacific,
    Southwest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: TeamId,
    pub abbrev: String,
    pub city: String,
    pub name: String,
    pub conference: Conference,
    pub division: Division,
    pub gm: GMPersonality,
    pub roster: Vec<PlayerId>,
    pub draft_picks: Vec<DraftPick>,
    /// Light Coach struct (sibling to GMPersonality). Default seeded if
    /// scrape data is missing — see `Coach::default_for(abbrev)`.
    #[serde(default)]
    pub coach: Coach,
}

impl Team {
    pub fn full_name(&self) -> String {
        format!("{} {}", self.city, self.name)
    }
}
