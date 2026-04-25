use crate::{DraftPickId, PlayerId, Position, Ratings, SeasonId, TeamId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protection {
    /// Pick conveys to the owning team only if outside top-N.
    TopNProtected(u8),
    /// Pick conveys only if outside lottery (top 14).
    LotteryProtected,
    Unprotected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPick {
    pub id: DraftPickId,
    pub original_team: TeamId,
    pub current_owner: TeamId,
    pub season: SeasonId,
    pub round: u8,
    pub protections: Protection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftProspect {
    pub id: PlayerId,
    pub name: String,
    pub mock_rank: u8,
    pub age: u8,
    pub position: Position,
    pub ratings: Ratings,
    pub potential: u8,
    pub draft_class: SeasonId,
}
