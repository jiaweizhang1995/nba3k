use serde::{Deserialize, Serialize};

/// Coaching scheme. NBA 2K's published list, 8 variants.
/// Used for `scheme_fit(player, coach)` in chemistry calc.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scheme {
    #[default]
    Balanced,
    Defense,
    GritAndGrind,
    PaceAndSpace,
    PerimeterCentric,
    PostCentric,
    Triangle,
    SevenSeconds,
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

/// Coaches with overall below this rating are flagged as "on the hot seat"
/// in `coach show`.
pub const HOT_SEAT_THRESHOLD: u8 = 65;

impl Coach {
    /// Seed a default coach for a team that has no scrape data. Schemes are
    /// derived deterministically from the team abbrev so each franchise has
    /// some flavor (PaceAndSpace vs PostCentric vs Defense, etc.) instead
    /// of every coach defaulting to Balanced.
    pub fn default_for(abbrev: &str) -> Self {
        let hash: u32 = abbrev
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_mul(131).wrapping_add(b as u32));
        let off = match hash % 6 {
            0 => Scheme::PaceAndSpace,
            1 => Scheme::PerimeterCentric,
            2 => Scheme::PostCentric,
            3 => Scheme::SevenSeconds,
            4 => Scheme::Triangle,
            _ => Scheme::Balanced,
        };
        let def = match (hash / 7) % 4 {
            0 => Scheme::Defense,
            1 => Scheme::GritAndGrind,
            2 => Scheme::PerimeterCentric,
            _ => Scheme::Balanced,
        };
        Self {
            id: 0,
            name: format!("{} Head Coach", abbrev),
            scheme_offense: off,
            scheme_defense: def,
            axes: CoachAxes::default(),
        }
    }

    /// Average of the 5 coach axes, rounded to the nearest u8 and clamped
    /// to [0, 99].
    pub fn overall(&self) -> u8 {
        let a = &self.axes;
        let avg =
            (a.strategy + a.leadership + a.mentorship + a.knowledge + a.team_management) / 5.0;
        avg.round().clamp(0.0, 99.0) as u8
    }

    /// True when `overall()` is below `HOT_SEAT_THRESHOLD` — used by `coach
    /// show` to surface a warning.
    pub fn on_hot_seat(&self) -> bool {
        self.overall() < HOT_SEAT_THRESHOLD
    }

    /// Build a deterministic candidate coach. `key` is a per-franchise /
    /// per-season seed that varies the schemes, axes, and synthetic name so
    /// repeated firings produce distinct (but reproducible) replacements.
    pub fn generated(abbrev: &str, key: u64) -> Self {
        let mut base = Self::default_for(abbrev);

        // Stir the abbrev hash with `key` so different (team, season,
        // attempt) tuples land on different schemes/axes.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in abbrev.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= key.wrapping_add(0x9E37_79B9_7F4A_7C15);
        h = h.wrapping_mul(0x100000001b3);

        let off = SCHEMES[(h as usize) % SCHEMES.len()];
        let def = SCHEMES[((h >> 16) as usize) % SCHEMES.len()];

        // Perturb each axis by ±15 around the 70 baseline. Different shift
        // windows so the axes don't move in lockstep.
        let perturb = |shift: u32| -> f32 {
            let raw = ((h >> shift) & 0xFF) as f32 / 255.0; // 0..1
            (raw * 30.0) - 15.0
        };
        base.axes = CoachAxes {
            strategy: (70.0 + perturb(0)).clamp(40.0, 95.0),
            leadership: (70.0 + perturb(8)).clamp(40.0, 95.0),
            mentorship: (70.0 + perturb(16)).clamp(40.0, 95.0),
            knowledge: (70.0 + perturb(24)).clamp(40.0, 95.0),
            team_management: (70.0 + perturb(32)).clamp(40.0, 95.0),
        };
        base.scheme_offense = off;
        base.scheme_defense = def;
        base.name = synthetic_name(h);
        base
    }
}

const SCHEMES: [Scheme; 8] = [
    Scheme::Balanced,
    Scheme::Defense,
    Scheme::GritAndGrind,
    Scheme::PaceAndSpace,
    Scheme::PerimeterCentric,
    Scheme::PostCentric,
    Scheme::Triangle,
    Scheme::SevenSeconds,
];

const FIRST_NAMES: &[&str] = &[
    "Wilson", "Marcus", "Jamal", "Tyrone", "Doc", "Chris", "Mike", "Steve", "Erik", "Quin", "Tom",
    "Greg", "Ime", "Will", "Taylor", "Joe", "Brian", "Jerry", "Frank", "Nate",
];

const LAST_NAMES: &[&str] = &[
    "Tillis",
    "Hardaway",
    "Crawford",
    "Lue",
    "Rivers",
    "Finch",
    "Snyder",
    "Kerr",
    "Spoelstra",
    "Popovich",
    "Udoka",
    "Hardy",
    "Jenkins",
    "Mazzulla",
    "Borrego",
    "Stotts",
    "Mosley",
    "Vogel",
    "Atkinson",
    "Daigneault",
];

fn synthetic_name(h: u64) -> String {
    let first = FIRST_NAMES[(h as usize) % FIRST_NAMES.len()];
    let last = LAST_NAMES[((h >> 8) as usize) % LAST_NAMES.len()];
    format!("{} {}", first, last)
}
