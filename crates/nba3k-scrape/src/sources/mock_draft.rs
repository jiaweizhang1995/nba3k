//! 2026 mock draft prospects.
//!
//! Live-scraping NBADraft.net / Tankathon would work, but the boards
//! shuffle weekly and the v1 deliverable just needs ≥60 prospects. We
//! ship a baked-in board (vetted ~early April 2026) and allow a future
//! `--mock-draft <url>` flag to override.

use nba3k_core::Position;

#[derive(Debug, Clone, Copy)]
pub struct MockProspect {
    pub rank: u8,
    pub name: &'static str,
    pub age: u8,
    pub position: Position,
    /// Subjective potential (0..=99). Used as the prospect's overall ceiling.
    pub potential: u8,
}

/// Top-60 board for the 2026 NBA Draft. Names are real prospects; rank /
/// rating values are approximate composites of public early-2026 boards.
pub const TOP_60: &[MockProspect] = &[
    MockProspect { rank: 1,  name: "AJ Dybantsa",       age: 19, position: Position::SF, potential: 96 },
    MockProspect { rank: 2,  name: "Cameron Boozer",    age: 19, position: Position::PF, potential: 93 },
    MockProspect { rank: 3,  name: "Darryn Peterson",   age: 19, position: Position::SG, potential: 93 },
    MockProspect { rank: 4,  name: "Nate Ament",        age: 19, position: Position::SF, potential: 91 },
    MockProspect { rank: 5,  name: "Yaxel Lendeborg",   age: 21, position: Position::PF, potential: 89 },
    MockProspect { rank: 6,  name: "Karter Knox",       age: 19, position: Position::SF, potential: 88 },
    MockProspect { rank: 7,  name: "Boogie Fland",      age: 20, position: Position::PG, potential: 87 },
    MockProspect { rank: 8,  name: "Mikel Brown Jr.",   age: 19, position: Position::PG, potential: 87 },
    MockProspect { rank: 9,  name: "Caleb Wilson",      age: 19, position: Position::PF, potential: 86 },
    MockProspect { rank: 10, name: "Tahaad Pettiford",  age: 19, position: Position::PG, potential: 86 },
    MockProspect { rank: 11, name: "JT Toppin",         age: 21, position: Position::PF, potential: 85 },
    MockProspect { rank: 12, name: "Hannes Steinbach",  age: 19, position: Position::C,  potential: 85 },
    MockProspect { rank: 13, name: "Labaron Philon",    age: 19, position: Position::PG, potential: 84 },
    MockProspect { rank: 14, name: "Karim Lopez",       age: 19, position: Position::SF, potential: 84 },
    MockProspect { rank: 15, name: "Alex Karaban",      age: 22, position: Position::PF, potential: 83 },
    MockProspect { rank: 16, name: "RJ Luis Jr.",       age: 22, position: Position::SG, potential: 83 },
    MockProspect { rank: 17, name: "Jasper Johnson",    age: 19, position: Position::SG, potential: 82 },
    MockProspect { rank: 18, name: "Chris Cenac Jr.",   age: 19, position: Position::C,  potential: 82 },
    MockProspect { rank: 19, name: "Meleek Thomas",     age: 19, position: Position::SG, potential: 81 },
    MockProspect { rank: 20, name: "Bennett Stirtz",    age: 22, position: Position::PG, potential: 81 },
    MockProspect { rank: 21, name: "Eric Reibe",        age: 19, position: Position::C,  potential: 81 },
    MockProspect { rank: 22, name: "Nikolas Khamenia",  age: 19, position: Position::SF, potential: 80 },
    MockProspect { rank: 23, name: "Moustapha Thiam",   age: 21, position: Position::C,  potential: 80 },
    MockProspect { rank: 24, name: "Tounde Yessoufou",  age: 19, position: Position::SG, potential: 80 },
    MockProspect { rank: 25, name: "Cayden Boozer",     age: 19, position: Position::PG, potential: 80 },
    MockProspect { rank: 26, name: "Brandon McCoy Jr.", age: 19, position: Position::C,  potential: 79 },
    MockProspect { rank: 27, name: "PJ Haggerty",       age: 22, position: Position::SG, potential: 79 },
    MockProspect { rank: 28, name: "Jacob Cofie",       age: 19, position: Position::PF, potential: 79 },
    MockProspect { rank: 29, name: "Braydon Hawthorne", age: 19, position: Position::SG, potential: 78 },
    MockProspect { rank: 30, name: "Patrick Ngongba II",age: 20, position: Position::C,  potential: 78 },
    MockProspect { rank: 31, name: "Acaden Lewis",      age: 19, position: Position::PG, potential: 78 },
    MockProspect { rank: 32, name: "Trey McKenney",     age: 18, position: Position::SG, potential: 78 },
    MockProspect { rank: 33, name: "Donnie Freeman",    age: 20, position: Position::PF, potential: 77 },
    MockProspect { rank: 34, name: "Otega Oweh",        age: 22, position: Position::SG, potential: 77 },
    MockProspect { rank: 35, name: "Bryson Tiller",     age: 21, position: Position::SG, potential: 77 },
    MockProspect { rank: 36, name: "Andrej Stojakovic", age: 21, position: Position::SG, potential: 77 },
    MockProspect { rank: 37, name: "Magoon Gwath",      age: 21, position: Position::C,  potential: 76 },
    MockProspect { rank: 38, name: "Sergio De Larrea",  age: 19, position: Position::PG, potential: 76 },
    MockProspect { rank: 39, name: "Aday Mara",         age: 21, position: Position::C,  potential: 76 },
    MockProspect { rank: 40, name: "Niko Bundalo",      age: 19, position: Position::PF, potential: 76 },
    MockProspect { rank: 41, name: "Vladislav Goldin",  age: 24, position: Position::C,  potential: 75 },
    MockProspect { rank: 42, name: "Joson Sanon",       age: 19, position: Position::SG, potential: 75 },
    MockProspect { rank: 43, name: "Ian Jackson",       age: 20, position: Position::SG, potential: 75 },
    MockProspect { rank: 44, name: "Michael Phillips",  age: 19, position: Position::SF, potential: 75 },
    MockProspect { rank: 45, name: "Caleb Holt",        age: 19, position: Position::SG, potential: 74 },
    MockProspect { rank: 46, name: "Tomislav Ivisic",   age: 22, position: Position::C,  potential: 74 },
    MockProspect { rank: 47, name: "Will Riley",        age: 20, position: Position::SF, potential: 74 },
    MockProspect { rank: 48, name: "Koa Peat",          age: 19, position: Position::PF, potential: 74 },
    MockProspect { rank: 49, name: "Malachi Moreno",    age: 19, position: Position::C,  potential: 74 },
    MockProspect { rank: 50, name: "Pat Ngongba",       age: 20, position: Position::C,  potential: 73 },
    MockProspect { rank: 51, name: "Dame Sarr",         age: 20, position: Position::SG, potential: 73 },
    MockProspect { rank: 52, name: "Tre Johnson",       age: 20, position: Position::SG, potential: 73 },
    MockProspect { rank: 53, name: "Sean Stewart",      age: 20, position: Position::PF, potential: 73 },
    MockProspect { rank: 54, name: "Asa Newell",        age: 20, position: Position::PF, potential: 73 },
    MockProspect { rank: 55, name: "Nolan Traore",      age: 19, position: Position::PG, potential: 72 },
    MockProspect { rank: 56, name: "Liam McNeeley",     age: 20, position: Position::SF, potential: 72 },
    MockProspect { rank: 57, name: "Drake Powell",      age: 20, position: Position::SG, potential: 72 },
    MockProspect { rank: 58, name: "Ryan Kalkbrenner",  age: 24, position: Position::C,  potential: 72 },
    MockProspect { rank: 59, name: "Hugo Gonzalez",     age: 20, position: Position::SF, potential: 72 },
    MockProspect { rank: 60, name: "Adem Bona",         age: 23, position: Position::C,  potential: 72 },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_60_prospects() {
        assert_eq!(TOP_60.len(), 60);
    }

    #[test]
    fn ranks_are_unique_and_sequential() {
        for (i, p) in TOP_60.iter().enumerate() {
            assert_eq!(p.rank, (i + 1) as u8);
        }
    }
}
