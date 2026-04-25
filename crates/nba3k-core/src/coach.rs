use serde::{Deserialize, Serialize};

/// Coaching scheme. NBA 2K's published list, 8 variants.
/// Used for `scheme_fit(player, coach)` in chemistry calc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scheme {
    Balanced,
    Defense,
    GritAndGrind,
    PaceAndSpace,
    PerimeterCentric,
    PostCentric,
    Triangle,
    SevenSeconds,
}

impl Default for Scheme {
    fn default() -> Self {
        Self::Balanced
    }
}

impl std::fmt::Display for Scheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Balanced => "Balanced",
                Self::Defense => "Defense",
                Self::GritAndGrind => "GritAndGrind",
                Self::PaceAndSpace => "PaceAndSpace",
                Self::PerimeterCentric => "PerimeterCentric",
                Self::PostCentric => "PostCentric",
                Self::Triangle => "Triangle",
                Self::SevenSeconds => "SevenSeconds",
            }
        )
    }
}

/// 5 coach axes from NBA 2K26 Coach Cards (MyTeam side, but the mental
/// model carries over). All values 0..=99.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoachAxes {
    pub strategy: f32,
    pub leadership: f32,
    pub mentorship: f32,
    pub knowledge: f32,
    pub team_management: f32,
}

impl Default for CoachAxes {
    fn default() -> Self {
        Self {
            strategy: 70.0,
            leadership: 70.0,
            mentorship: 70.0,
            knowledge: 70.0,
            team_management: 70.0,
        }
    }
}

/// Light Coach struct. Sibling to `GMPersonality` — each Team owns one.
/// No playbook depth (deferred to M7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coach {
    pub id: u32,
    pub name: String,
    pub scheme_offense: Scheme,
    pub scheme_defense: Scheme,
    pub axes: CoachAxes,
}

impl Default for Coach {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Default Coach".to_string(),
            scheme_offense: Scheme::Balanced,
            scheme_defense: Scheme::Balanced,
            axes: CoachAxes::default(),
        }
    }
}

impl Coach {
    /// Seed a default coach for a team that has no scrape data. Name is
    /// derived from the abbrev so the UI has something to show.
    pub fn default_for(abbrev: &str) -> Self {
        Self {
            id: 0,
            name: format!("{} Head Coach", abbrev),
            scheme_offense: Scheme::Balanced,
            scheme_defense: Scheme::Balanced,
            axes: CoachAxes::default(),
        }
    }
}
