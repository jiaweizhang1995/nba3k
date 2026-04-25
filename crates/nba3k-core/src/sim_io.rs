use crate::{GameId, PlayerId, SeasonId, TeamId};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerLine {
    pub player: PlayerId,
    pub minutes: u8,
    pub pts: u8,
    pub reb: u8,
    pub ast: u8,
    pub stl: u8,
    pub blk: u8,
    pub tov: u8,
    pub fg_made: u8,
    pub fg_att: u8,
    pub three_made: u8,
    pub three_att: u8,
    pub ft_made: u8,
    pub ft_att: u8,
    pub plus_minus: i8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxScore {
    pub home_lines: Vec<PlayerLine>,
    pub away_lines: Vec<PlayerLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameResult {
    pub id: GameId,
    pub season: SeasonId,
    pub date: NaiveDate,
    pub home: TeamId,
    pub away: TeamId,
    pub home_score: u16,
    pub away_score: u16,
    pub box_score: BoxScore,
    pub overtime_periods: u8,
    pub is_playoffs: bool,
}

impl GameResult {
    pub fn winner(&self) -> TeamId {
        if self.home_score >= self.away_score { self.home } else { self.away }
    }

    pub fn loser(&self) -> TeamId {
        if self.home_score >= self.away_score { self.away } else { self.home }
    }
}
