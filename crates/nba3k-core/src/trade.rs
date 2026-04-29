use crate::{Cents, DraftPickId, PlayerId, TeamId, TradeId};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TradeAssets {
    pub players_out: Vec<PlayerId>,
    pub picks_out: Vec<DraftPickId>,
    pub cash_out: Cents,
}

impl TradeAssets {
    pub fn is_empty(&self) -> bool {
        self.players_out.is_empty() && self.picks_out.is_empty() && self.cash_out == Cents::ZERO
    }
}

/// Multi-team capable from day one. v1 enforces `assets_by_team.len() == 2`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeOffer {
    pub id: TradeId,
    pub initiator: TeamId,
    /// IndexMap preserves insertion order — initiator first.
    pub assets_by_team: IndexMap<TeamId, TradeAssets>,
    /// Negotiation round (1-indexed).
    pub round: u8,
    /// Parent in counter-offer chain.
    pub parent: Option<TradeId>,
}

impl TradeOffer {
    pub fn is_two_team(&self) -> bool {
        self.assets_by_team.len() == 2
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Verdict {
    Accept,
    Reject(RejectReason),
    Counter(TradeOffer),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RejectReason {
    InsufficientValue,
    CbaViolation(String),
    NoTradeClause(PlayerId),
    BadFaith,
    OutOfRoundCap,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEvaluation {
    /// Net perceived gain for the evaluator, $-equiv.
    pub net_value: Cents,
    pub verdict: Verdict,
    pub confidence: f32,
    pub commentary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NegotiationState {
    Open {
        chain: Vec<TradeOffer>,
    },
    Accepted(TradeOffer),
    Rejected {
        final_offer: TradeOffer,
        reason: RejectReason,
    },
    Stalled,
}
