//! Contract generation. Maps a player's OVR (and age) to a length-and-salary
//! tier modeled on NBA market behavior:
//!
//! | OVR    | tier              | $/yr (mid)   | length  |
//! |--------|-------------------|--------------|---------|
//! | ≥ 90   | max / supermax    | $55M         | 4 yr    |
//! | 85-89  | borderline-max    | $30M         | 3-4 yr  |
//! | 80-84  | top role player   | $15M         | 3 yr    |
//! | 70-79  | rotation player   | $5M          | 2-3 yr  |
//! | < 70   | veteran-min       | $2M          | 1-2 yr  |
//!
//! Length scales inversely with age: younger players (< 26) get an extra
//! year inside their tier, players ≥ 32 lose a year (floor 1).
//!
//! All amounts use `Cents`. `1 dollar = Cents(100)`, so `$30M` is
//! `Cents(30_000_000_00)`.
//!
//! Output options (`team_option`, `player_option`) are left at their
//! conservative defaults — full guarantee, no options. M11 just needs
//! base salary cap totals to land in a realistic range; option logic
//! belongs to the FA negotiation engine in a later milestone.

use nba3k_core::{BirdRights, Cents, Contract, ContractYear, Player, SeasonId};

/// Generate a contract for `player` starting in `season`. Length and
/// per-year salary are flat across the deal — escalators are out of scope.
pub fn generate_contract(player: &Player, season: SeasonId) -> Contract {
    let (base_years, salary) = tier_for(player.overall);
    let years = adjust_length_for_age(base_years, player.age);

    let contract_years: Vec<ContractYear> = (0..years)
        .map(|i| ContractYear {
            season: SeasonId(season.0 + i as u16),
            salary,
            guaranteed: true,
            team_option: false,
            player_option: false,
        })
        .collect();

    Contract {
        years: contract_years,
        signed_in_season: season,
        bird_rights: BirdRights::Non,
    }
}

/// Maps OVR to a `(base_length, salary)` tuple. The base length is the
/// midpoint length for the tier; `adjust_length_for_age` shifts ±1.
fn tier_for(ovr: u8) -> (u8, Cents) {
    match ovr {
        90..=u8::MAX => (4, Cents(55_000_000_00)),
        85..=89 => (4, Cents(30_000_000_00)),
        80..=84 => (3, Cents(15_000_000_00)),
        70..=79 => (3, Cents(5_000_000_00)),
        _ => (2, Cents(2_000_000_00)),
    }
}

fn adjust_length_for_age(base: u8, age: u8) -> u8 {
    let shifted = match age {
        0..=25 => base.saturating_add(1),
        26..=31 => base,
        _ => base.saturating_sub(1),
    };
    shifted.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba3k_core::{PlayerId, Position, Ratings};

    fn player(ovr: u8, age: u8) -> Player {
        Player {
            id: PlayerId(1),
            name: "Test".into(),
            primary_position: Position::SF,
            secondary_position: None,
            age,
            overall: ovr,
            potential: ovr,
            ratings: Ratings::default(),
            contract: None,
            team: None,
            injury: None,
            no_trade_clause: false,
            trade_kicker_pct: None,
            role: nba3k_core::PlayerRole::default(),
            morale: 0.5,
        }
    }

    #[test]
    fn star_gets_max() {
        let c = generate_contract(&player(95, 27), SeasonId(2026));
        assert!(c.years[0].salary >= Cents(40_000_000_00));
        assert!(c.years.len() >= 3);
    }

    #[test]
    fn vet_min() {
        let c = generate_contract(&player(65, 33), SeasonId(2026));
        assert!(c.years[0].salary <= Cents(3_000_000_00));
    }
}
