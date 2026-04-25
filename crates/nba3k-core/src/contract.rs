use crate::{Cents, SeasonId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BirdRights {
    #[default]
    Non,
    Early,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractYear {
    pub season: SeasonId,
    pub salary: Cents,
    pub guaranteed: bool,
    pub team_option: bool,
    pub player_option: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub years: Vec<ContractYear>,
    pub signed_in_season: SeasonId,
    pub bird_rights: BirdRights,
}

impl Contract {
    /// Salary for a specific season; `Cents::ZERO` if outside contract length.
    pub fn salary_for(&self, season: SeasonId) -> Cents {
        self.years
            .iter()
            .find(|y| y.season == season)
            .map(|y| y.salary)
            .unwrap_or(Cents::ZERO)
    }

    pub fn current_salary(&self, current: SeasonId) -> Cents {
        self.salary_for(current)
    }

    pub fn total_value(&self) -> Cents {
        self.years.iter().map(|y| y.salary).sum()
    }
}
