//! Worker D — asset fit model.
//!
//! Position + skill fit for an *incoming* player from the receiving
//! team's perspective. Captures "we don't need another center".
//!
//! Three components, each emits a Reason:
//!   - positional_need: receiving team's rotation depth at incoming's
//!     primary position. Sparse depth = bonus, deep depth = penalty.
//!   - skill_overlap: number of high-OVR same-position rotation players.
//!     Each one above OVR 80 adds a redundancy penalty.
//!   - rotation_saturation: if the team's top-8 minutes are saturated
//!     by players already at or above the incoming's OVR, fit drops.
//!
//! Output `Score.value` is in cents (positive = good fit bonus,
//! negative = redundancy penalty). Magnitudes are tied to the player's
//! own positional baseline so a star fit bonus dwarfs a role-player one.
//!
//! See `phases/M4-realism.md` "Worker D" for the full spec.

use crate::weights::AssetFitWeights;
use crate::Score;
use nba3k_core::{LeagueSnapshot, Player, Position, TeamId};

/// Rotation size used everywhere in this module — top-8 by OVR.
const ROTATION_SIZE: usize = 8;

/// "High-OVR" threshold for skill-overlap penalty.
const HIGH_OVR_THRESHOLD: u8 = 80;

/// Hand-fit positional baseline used as the magnitude anchor for fit
/// bonuses/penalties. Mirrors the curve used by Worker A's player_value
/// at a coarser resolution so that asset_fit deltas are commensurate
/// with player_value deltas (i.e., a fit penalty on a star is bigger
/// than a fit penalty on a role player).
fn baseline_cents_for_ovr(ovr: u8) -> f64 {
    let ovr = ovr.min(99) as f64;
    if ovr <= 50.0 {
        return 0.0;
    }
    let x = ovr - 50.0;
    let millions = (x / 49.0).powf(2.6) * 210.0;
    millions * 1_000_000.0 * 100.0 // dollars → cents
}

/// True if `roster_player.primary_position` matches `target` OR their
/// secondary does. Captures "this team already has a center" even
/// when the listed primary is something else.
fn shares_position(p: &Player, target: Position) -> bool {
    p.primary_position == target || p.secondary_position == Some(target)
}

/// Pick the top-8 players on `team` by OVR. Sorted descending so the
/// first entry is the team's headliner.
fn top_rotation<'a>(team: TeamId, league: &'a LeagueSnapshot<'a>) -> Vec<&'a Player> {
    let mut roster = league.roster(team);
    roster.sort_by(|a, b| b.overall.cmp(&a.overall));
    roster.truncate(ROTATION_SIZE);
    roster
}

pub fn asset_fit(
    incoming: &Player,
    receiving_team: TeamId,
    league: &LeagueSnapshot,
    weights: &AssetFitWeights,
) -> Score {
    let rotation = top_rotation(receiving_team, league);
    let pos = incoming.primary_position;
    let baseline = baseline_cents_for_ovr(incoming.overall);

    // ---- 1. Positional need -----------------------------------------
    // Count rotation players who can cover `pos`. 0 → strong need
    // (positive bonus = positional_need_max × baseline). 4+ → strong
    // glut (penalty of similar magnitude).
    let same_pos_count = rotation.iter().filter(|p| shares_position(p, pos)).count();
    // Sweet spot is 2 (starter + backup). Linear deviation from there.
    let need_signal = 2.0 - same_pos_count as f64; // +2 → no one, -2 → glut of 4
    let need_delta = baseline * weights.positional_need_max as f64 * (need_signal / 2.0);

    // ---- 2. Skill overlap penalty ------------------------------------
    // Each high-OVR (≥80) rotation player at the same position adds a
    // penalty proportional to baseline × skill_overlap_penalty.
    let high_ovr_overlap = rotation
        .iter()
        .filter(|p| shares_position(p, pos) && p.overall >= HIGH_OVR_THRESHOLD)
        .count();
    let skill_overlap_delta =
        -(baseline * weights.skill_overlap_penalty as f64 * high_ovr_overlap as f64);

    // ---- 3. Rotation saturation --------------------------------------
    // If the rotation already has 8 players with OVR ≥ incoming, this
    // player can't crack the top-8 → saturation penalty.
    let saturating = rotation
        .iter()
        .filter(|p| p.overall >= incoming.overall)
        .count();
    let saturated = rotation.len() >= ROTATION_SIZE && saturating >= ROTATION_SIZE;
    let saturation_delta = if saturated {
        -(baseline * weights.rotation_saturation_penalty as f64)
    } else {
        0.0
    };

    let mut score = Score::new(0.0);
    score.add("positional_need", need_delta);
    score.add("skill_overlap", skill_overlap_delta);
    score.add("rotation_saturation", saturation_delta);
    score.sort_reasons();
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weights::AssetFitWeights;
    use nba3k_core::{
        Conference, Division, GMArchetype, GMPersonality, LeagueSnapshot, LeagueYear, Player,
        PlayerId, Ratings, SeasonId, SeasonPhase, Team, TeamId,
    };
    use std::collections::HashMap;

    fn mk_team(id: u8, abbrev: &str) -> Team {
        Team {
            id: TeamId(id),
            abbrev: abbrev.into(),
            city: abbrev.into(),
            name: abbrev.into(),
            conference: Conference::East,
            division: Division::Atlantic,
            gm: GMPersonality::from_archetype(format!("{abbrev} GM"), GMArchetype::WinNow),
            roster: Vec::new(),
            draft_picks: Vec::new(),
            coach: nba3k_core::Coach::default_for(abbrev),
        }
    }

    fn mk_player(id: u32, ovr: u8, pos: Position, team: Option<TeamId>) -> Player {
        Player {
            id: PlayerId(id),
            name: format!("P{id}"),
            primary_position: pos,
            secondary_position: None,
            age: 26,
            overall: ovr,
            potential: ovr,
            ratings: Ratings::default(),
            contract: None,
            team,
            injury: None,
            no_trade_clause: false,
            trade_kicker_pct: None,
            role: nba3k_core::PlayerRole::default(),
            morale: 0.5,
        }
    }

    fn snap<'a>(
        teams: &'a [Team],
        players: &'a HashMap<PlayerId, Player>,
        picks: &'a HashMap<nba3k_core::DraftPickId, nba3k_core::DraftPick>,
        standings: &'a HashMap<TeamId, nba3k_core::TeamRecordSummary>,
    ) -> LeagueSnapshot<'a> {
        LeagueSnapshot {
            current_season: SeasonId(2026),
            current_phase: SeasonPhase::Regular,
            current_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            league_year: LeagueYear::for_season(SeasonId(2026)).unwrap(),
            teams,
            players_by_id: players,
            picks_by_id: picks,
            standings,
        }
    }

    #[test]
    fn baseline_cents_curve_is_monotonic() {
        let mut prev = -1.0;
        for ovr in 50u8..=99 {
            let v = baseline_cents_for_ovr(ovr);
            assert!(v >= prev, "non-monotonic at {ovr}: prev={prev} cur={v}");
            prev = v;
        }
    }

    #[test]
    fn same_position_high_ovr_counts_toward_overlap() {
        let star_c = mk_player(1, 90, Position::C, Some(TeamId(1)));
        assert!(shares_position(&star_c, Position::C));
        let pf_w_c_secondary = Player {
            secondary_position: Some(Position::C),
            ..mk_player(2, 80, Position::PF, Some(TeamId(1)))
        };
        assert!(shares_position(&pf_w_c_secondary, Position::C));
        let pg = mk_player(3, 80, Position::PG, Some(TeamId(1)));
        assert!(!shares_position(&pg, Position::C));
    }
}
