//! Standard-mode CBA validator (Worker C).
//!
//! Surface:
//! - `validate(offer, league) -> Result<(), CbaViolation>` — entry point.
//! - Sub-checks each its own `pub fn` for testability.
//! - Asymmetric trade-kicker math: sender uses pre-kicker, receiver uses
//!   post-kicker prorated. See RESEARCH.md item 7.
//!
//! v1 simplifications (see decision log in `phases/M3-trade.md`):
//! - "Aggregation" is detected as `players_out.len() >= 2` for the team.
//! - Per-player aggregation cooldown is a stub (returns `Ok`) until
//!   `acquired_on` is persisted on `Player`.
//! - Hard-cap triggers are gated on the team's *current* apron tier rather
//!   than tracking which exception/sign-and-trade triggered the cap. The
//!   second-apron restrictions (no aggregation, no cash) are enforced
//!   strictly because those follow purely from current salary, not from
//!   triggers.
//! - Salary tier classification uses the *sending* team's pre-trade total
//!   salary as the discriminator (cap / apron_1 / apron_2). Each team is
//!   evaluated independently, both must pass.

use crate::snapshot::LeagueSnapshot;
use nba3k_core::{Cents, Contract, ContractYear, Player, PlayerId, SeasonId, TeamId, TradeOffer};

#[derive(Debug, Clone, thiserror::Error)]
pub enum CbaViolation {
    #[error("salary matching failed: out={out_dollars} in={in_dollars} tier={tier}")]
    SalaryMatching {
        team: TeamId,
        out_dollars: i64,
        in_dollars: i64,
        tier: String,
    },
    #[error("hard-cap trigger: team would exceed apron after trade")]
    HardCapTrigger { team: TeamId, apron: u8 },
    #[error("no-trade clause held by player {0}")]
    NoTradeClause(PlayerId),
    #[error("cash limit exceeded: {amount_dollars} sent/received this season")]
    CashLimitExceeded { team: TeamId, amount_dollars: i64 },
    #[error("aggregation cooldown: player {player} acquired too recently")]
    AggregationCooldown { team: TeamId, player: PlayerId },
    #[error("roster size out of bounds: post-trade size = {size}")]
    RosterSize { team: TeamId, size: u32 },
    #[error("apron 2 forbids cash and aggregation")]
    Apron2Restriction { team: TeamId },
}

/// Salary-matching tier the *sending* team falls into based on its pre-trade
/// total salary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SalaryTier {
    UnderCap,
    NonApron,
    Apron1,
    Apron2,
}

impl SalaryTier {
    fn label(self) -> &'static str {
        match self {
            Self::UnderCap => "under_cap",
            Self::NonApron => "non_apron",
            Self::Apron1 => "apron_1",
            Self::Apron2 => "apron_2",
        }
    }
}

/// Threshold above which the non-apron tier flips from 200%+$250K to
/// 125%+$250K. Per post-2023 CBA: $7.5M outgoing.
const NON_APRON_TIER_BREAK: Cents = Cents(750_000_000);
/// Flat add-on across non-apron sub-tiers: $250,000.
const NON_APRON_FLAT_ADD: Cents = Cents(25_000_000);

pub fn validate(offer: &TradeOffer, league: &LeagueSnapshot) -> Result<(), CbaViolation> {
    // NTC first — cheap and a hard reject regardless of tier.
    check_no_trade_clauses(offer, league)?;
    // Cash limits — direction-sensitive.
    check_cash_limits(offer, league)?;
    // Apron 2 restrictions (no aggregation, no cash) before salary matching
    // since they short-circuit the matching tier entirely.
    check_apron_2_restrictions(offer, league)?;
    // Salary matching per side.
    for &team in offer.assets_by_team.keys() {
        check_salary_matching(team, offer, league)?;
    }
    // Aggregation cooldown stub (always Ok in v1).
    for &team in offer.assets_by_team.keys() {
        check_aggregation_cooldown(team, offer, league)?;
    }
    // Roster size 13-15 post-trade.
    for &team in offer.assets_by_team.keys() {
        check_roster_size(team, offer, league)?;
    }
    // Hard-cap trigger gate (v1: just verifies neither side rises above
    // apron_2 after the trade, since apron_2 is a hard cap).
    for &team in offer.assets_by_team.keys() {
        check_hard_cap(team, offer, league)?;
    }
    Ok(())
}

/// Sender-side salary for matching: pre-kicker. Sums each outgoing player's
/// current cap hit (no kicker bump), plus any cash sent.
pub fn outgoing_salary_pre_kicker(
    team: TeamId,
    offer: &TradeOffer,
    league: &LeagueSnapshot,
) -> Cents {
    let Some(side) = offer.assets_by_team.get(&team) else {
        return Cents::ZERO;
    };
    let mut total = side.cash_out;
    for pid in &side.players_out {
        if let Some(p) = league.player(*pid) {
            total += current_salary(p, league.current_season);
        }
    }
    total
}

/// Receiver-side salary for matching: post-kicker. For each incoming player
/// holding a kicker, the bump (pct of remaining base) is prorated over the
/// remaining guaranteed years and added to year-1 cap hit. Cash received is
/// included.
///
/// Multi-team offers (`len() >= 3`) route assets via round-robin: team `i`'s
/// outgoing flows to team `(i+1) % n` (same convention as
/// `apply_accepted_trade`). For salary matching that means each team's
/// "incoming" is the assets sent by exactly *one* prior team, not the union
/// of every other side. The 2-team case degenerates correctly: prev(team[1])
/// = team[0] and prev(team[0]) = team[1], so each team sees the other's
/// outgoing — same behavior as the original simple sum.
pub fn incoming_salary_post_kicker(
    team: TeamId,
    offer: &TradeOffer,
    league: &LeagueSnapshot,
) -> Cents {
    let Some(origin) = incoming_origin(team, offer) else {
        return Cents::ZERO;
    };
    let Some(other_side) = offer.assets_by_team.get(&origin) else {
        return Cents::ZERO;
    };
    let mut total = other_side.cash_out;
    for pid in &other_side.players_out {
        if let Some(p) = league.player(*pid) {
            total += incoming_year_one_cap_hit(p, league.current_season);
        }
    }
    total
}

/// Round-robin origin: the team whose outgoing assets are routed to `team`
/// under `apply_accepted_trade`'s `(i+1) % n` rule. Returns `None` if `team`
/// is not part of the offer.
pub fn incoming_origin(team: TeamId, offer: &TradeOffer) -> Option<TeamId> {
    let teams: Vec<TeamId> = offer.assets_by_team.keys().copied().collect();
    let n = teams.len();
    if n < 2 {
        return None;
    }
    let idx = teams.iter().position(|t| *t == team)?;
    let prev = (idx + n - 1) % n;
    Some(teams[prev])
}

/// Year-1 cap hit on the new team for a single incoming player, including a
/// prorated kicker bump if the player has one.
fn incoming_year_one_cap_hit(player: &Player, current: SeasonId) -> Cents {
    let base = current_salary(player, current);
    let pct = player.trade_kicker_pct.unwrap_or(0);
    if pct == 0 {
        return base;
    }
    let Some(contract) = player.contract.as_ref() else {
        return base;
    };
    let remaining = remaining_guaranteed_base(contract, current);
    if remaining.0 == 0 {
        return base;
    }
    let years = guaranteed_years_remaining(contract, current).max(1) as i64;
    // Total kicker = pct% of remaining guaranteed base.
    let total_kicker_cents = (remaining.0.saturating_mul(pct as i64)) / 100;
    let prorated = total_kicker_cents / years;
    base + Cents(prorated)
}

fn current_salary(player: &Player, current: SeasonId) -> Cents {
    player
        .contract
        .as_ref()
        .map(|c| c.current_salary(current))
        .unwrap_or(Cents::ZERO)
}

fn remaining_guaranteed_base(contract: &Contract, current: SeasonId) -> Cents {
    contract
        .years
        .iter()
        .filter(|y| year_counts_for_kicker(y, current))
        .map(|y| y.salary)
        .sum()
}

fn guaranteed_years_remaining(contract: &Contract, current: SeasonId) -> u32 {
    contract
        .years
        .iter()
        .filter(|y| year_counts_for_kicker(y, current))
        .count() as u32
}

/// A contract year contributes to kicker base only if it is in the current
/// season or later AND is guaranteed AND not subject to an unexercised
/// option (player or team option). RESEARCH.md item 7.
fn year_counts_for_kicker(y: &ContractYear, current: SeasonId) -> bool {
    y.season.0 >= current.0 && y.guaranteed && !y.team_option && !y.player_option
}

/// Sum of all current cap hits on `team`'s roster. v1: ignores dead-cap and
/// non-roster holds — close enough for the apron tier classifier.
pub fn team_total_salary(team: TeamId, league: &LeagueSnapshot) -> Cents {
    league
        .roster(team)
        .iter()
        .map(|p| current_salary(p, league.current_season))
        .sum()
}

pub fn classify_salary_tier(team: TeamId, league: &LeagueSnapshot) -> SalaryTier {
    let total = team_total_salary(team, league);
    let ly = league.league_year;
    if total < ly.cap {
        SalaryTier::UnderCap
    } else if total < ly.apron_1 {
        SalaryTier::NonApron
    } else if total < ly.apron_2 {
        SalaryTier::Apron1
    } else {
        SalaryTier::Apron2
    }
}

/// Maximum incoming salary `team` may take back given its outgoing total
/// and tier. Computed from `LeagueYear` constants.
pub fn max_incoming_for_tier(
    tier: SalaryTier,
    outgoing: Cents,
    team: TeamId,
    league: &LeagueSnapshot,
) -> Cents {
    match tier {
        SalaryTier::UnderCap => {
            // Under cap: receiving ≤ outgoing + cap room.
            let cap_room =
                Cents((league.league_year.cap.0 - team_total_salary(team, league).0).max(0));
            outgoing + cap_room
        }
        SalaryTier::NonApron => {
            if outgoing <= NON_APRON_TIER_BREAK {
                Cents(outgoing.0.saturating_mul(2)) + NON_APRON_FLAT_ADD
            } else {
                // 125% + $250K
                Cents((outgoing.0.saturating_mul(125)) / 100) + NON_APRON_FLAT_ADD
            }
        }
        SalaryTier::Apron1 => Cents((outgoing.0.saturating_mul(110)) / 100),
        SalaryTier::Apron2 => outgoing,
    }
}

pub fn check_salary_matching(
    team: TeamId,
    offer: &TradeOffer,
    league: &LeagueSnapshot,
) -> Result<(), CbaViolation> {
    let tier = classify_salary_tier(team, league);
    let out = outgoing_salary_pre_kicker(team, offer, league);
    let inc = incoming_salary_post_kicker(team, offer, league);
    let limit = max_incoming_for_tier(tier, out, team, league);
    if inc.0 > limit.0 {
        return Err(CbaViolation::SalaryMatching {
            team,
            out_dollars: out.as_dollars(),
            in_dollars: inc.as_dollars(),
            tier: tier.label().to_string(),
        });
    }
    Ok(())
}

pub fn check_no_trade_clauses(
    offer: &TradeOffer,
    league: &LeagueSnapshot,
) -> Result<(), CbaViolation> {
    for side in offer.assets_by_team.values() {
        for pid in &side.players_out {
            if let Some(p) = league.player(*pid) {
                if p.no_trade_clause {
                    return Err(CbaViolation::NoTradeClause(*pid));
                }
            }
        }
    }
    Ok(())
}

pub fn check_cash_limits(offer: &TradeOffer, league: &LeagueSnapshot) -> Result<(), CbaViolation> {
    let limit = league.league_year.max_trade_cash;
    for (team, side) in &offer.assets_by_team {
        if side.cash_out > limit {
            return Err(CbaViolation::CashLimitExceeded {
                team: *team,
                amount_dollars: side.cash_out.as_dollars(),
            });
        }
    }
    Ok(())
}

pub fn check_apron_2_restrictions(
    offer: &TradeOffer,
    league: &LeagueSnapshot,
) -> Result<(), CbaViolation> {
    for (team, side) in &offer.assets_by_team {
        if classify_salary_tier(*team, league) != SalaryTier::Apron2 {
            continue;
        }
        // Apron 2 teams: no cash sent, no aggregation (≥2 outgoing players).
        if side.cash_out > Cents::ZERO || side.players_out.len() >= 2 {
            return Err(CbaViolation::Apron2Restriction { team: *team });
        }
    }
    Ok(())
}

/// v1 stub: no `acquired_on` persisted on `Player` yet, so we always pass.
/// Once that field exists, gate aggregation (≥2 outgoing players) on each
/// player having been on roster ≥60 sim days.
pub fn check_aggregation_cooldown(
    _team: TeamId,
    _offer: &TradeOffer,
    _league: &LeagueSnapshot,
) -> Result<(), CbaViolation> {
    Ok(())
}

pub fn check_roster_size(
    team: TeamId,
    offer: &TradeOffer,
    league: &LeagueSnapshot,
) -> Result<(), CbaViolation> {
    let current_size = league.roster(team).len() as i64;
    let outgoing = offer
        .assets_by_team
        .get(&team)
        .map(|s| s.players_out.len() as i64)
        .unwrap_or(0);
    let incoming: i64 = offer
        .assets_by_team
        .iter()
        .filter(|(t, _)| **t != team)
        .map(|(_, s)| s.players_out.len() as i64)
        .sum();
    let post = current_size - outgoing + incoming;
    // 2025-26 CBA: 15 standard contracts + 3 two-way = 18 total roster spots.
    // Floor of 13 enforces the league minimum carry rule.
    if !(13..=18).contains(&post) {
        return Err(CbaViolation::RosterSize {
            team,
            size: post.max(0) as u32,
        });
    }
    Ok(())
}

/// v1: hard-cap gate is "post-trade total must not exceed apron_2 if it
/// didn't before." Full trigger tracking (sign-and-trade, taxpayer MLE,
/// BAE etc.) is deferred — see decision log.
pub fn check_hard_cap(
    team: TeamId,
    offer: &TradeOffer,
    league: &LeagueSnapshot,
) -> Result<(), CbaViolation> {
    let pre = team_total_salary(team, league);
    let out = outgoing_salary_pre_kicker(team, offer, league);
    let inc = incoming_salary_post_kicker(team, offer, league);
    let post = Cents(pre.0.saturating_sub(out.0).saturating_add(inc.0));
    if pre < league.league_year.apron_2 && post >= league.league_year.apron_2 {
        return Err(CbaViolation::HardCapTrigger { team, apron: 2 });
    }
    Ok(())
}
