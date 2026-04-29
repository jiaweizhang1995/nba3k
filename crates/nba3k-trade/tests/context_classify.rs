//! Tests for `context::classify_team` + `apply_context`.
//!
//! `LeagueSnapshot` is borrow-only, so the test module owns the underlying
//! buffers via `SnapshotOwned` and lends out a `LeagueSnapshot<'_>` per
//! assertion.

use chrono::NaiveDate;
use nba3k_core::{
    BirdRights, Cents, Conference, Contract, ContractYear, Division, DraftPick, DraftPickId,
    GMArchetype, GMPersonality, GMTraits, LeagueYear, Player, PlayerId, Position, Ratings,
    SeasonId, SeasonPhase, Team, TeamId,
};
use nba3k_trade::context::{apply_context, classify_team};
use nba3k_trade::snapshot::{LeagueSnapshot, TeamRecordSummary};
use nba3k_trade::TeamMode;
use std::collections::HashMap;

/// Owned-side of a synthetic league. Tests build one of these and call
/// `.snapshot()` to get a borrowed view.
struct SnapshotOwned {
    teams: Vec<Team>,
    players: HashMap<PlayerId, Player>,
    picks: HashMap<DraftPickId, DraftPick>,
    standings: HashMap<TeamId, TeamRecordSummary>,
    season: SeasonId,
    phase: SeasonPhase,
    date: NaiveDate,
    league_year: LeagueYear,
}

impl SnapshotOwned {
    fn snapshot(&self) -> LeagueSnapshot<'_> {
        LeagueSnapshot {
            current_season: self.season,
            current_phase: self.phase,
            current_date: self.date,
            league_year: self.league_year,
            teams: &self.teams,
            players_by_id: &self.players,
            picks_by_id: &self.picks,
            standings: &self.standings,
        }
    }
}

struct RosterSpec {
    /// (overall, age) pairs, one per slot.
    members: Vec<(u8, u8)>,
}

fn build_team(team_id: TeamId, abbrev: &str, archetype: GMArchetype) -> Team {
    Team {
        id: team_id,
        abbrev: abbrev.to_string(),
        city: format!("City{}", team_id.0),
        name: format!("Name{}", team_id.0),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype(abbrev, archetype),
        roster: vec![],
        draft_picks: vec![],
        coach: nba3k_core::Coach::default_for(abbrev),
    }
}

fn build_player(id: PlayerId, team: TeamId, ovr: u8, age: u8) -> Player {
    Player {
        id,
        name: format!("Player{}", id.0),
        primary_position: Position::SF,
        secondary_position: None,
        age,
        overall: ovr,
        potential: ovr.max(80),
        ratings: Ratings::default(),
        contract: Some(Contract {
            years: vec![ContractYear {
                season: SeasonId(2026),
                salary: Cents(2_000_000_00),
                guaranteed: true,
                team_option: false,
                player_option: false,
            }],
            signed_in_season: SeasonId(2025),
            bird_rights: BirdRights::Full,
        }),
        team: Some(team),
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: nba3k_core::PlayerRole::default(),
        morale: 0.5,
    }
}

fn league_year() -> LeagueYear {
    LeagueYear::for_season(SeasonId(2026)).expect("2025-26 encoded")
}

fn build_snapshot(
    target_team: TeamId,
    abbrev: &str,
    archetype: GMArchetype,
    roster: RosterSpec,
    record: TeamRecordSummary,
    phase: SeasonPhase,
    date: NaiveDate,
) -> SnapshotOwned {
    let team = build_team(target_team, abbrev, archetype);
    let mut players: HashMap<PlayerId, Player> = HashMap::new();
    for (i, (ovr, age)) in roster.members.iter().enumerate() {
        let pid = PlayerId(1000 + i as u32);
        players.insert(pid, build_player(pid, target_team, *ovr, *age));
    }
    let mut standings: HashMap<TeamId, TeamRecordSummary> = HashMap::new();
    standings.insert(target_team, record);

    SnapshotOwned {
        teams: vec![team],
        players,
        picks: HashMap::new(),
        standings,
        season: SeasonId(2026),
        phase,
        date,
        league_year: league_year(),
    }
}

// --- classify_team -----------------------------------------------------------

#[test]
fn contender_with_star_and_winning_record() {
    // Top-9 OVR averages mid-twenties, has a 90-OVR star, .700 record.
    let roster = RosterSpec {
        members: vec![
            (90, 27),
            (84, 26),
            (82, 28),
            (80, 25),
            (79, 24),
            (78, 26),
            (76, 23),
            (75, 27),
            (74, 22),
        ],
    };
    let record = TeamRecordSummary {
        wins: 30,
        losses: 12,
        conf_rank: 2,
        point_diff: 200,
    };
    let owned = build_snapshot(
        TeamId(1),
        "BOS",
        GMArchetype::WinNow,
        roster,
        record,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );
    assert_eq!(
        classify_team(TeamId(1), &owned.snapshot()),
        TeamMode::Contend
    );
}

#[test]
fn full_rebuild_young_and_no_keepers() {
    // Young, no star, no keepers, losing.
    let roster = RosterSpec {
        members: vec![
            (75, 22),
            (73, 21),
            (72, 23),
            (70, 22),
            (70, 21),
            (68, 24),
            (68, 22),
            (66, 21),
            (65, 22),
        ],
    };
    let record = TeamRecordSummary {
        wins: 8,
        losses: 30,
        conf_rank: 14,
        point_diff: -300,
    };
    let owned = build_snapshot(
        TeamId(2),
        "WAS",
        GMArchetype::Rebuilder,
        roster,
        record,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );
    assert_eq!(
        classify_team(TeamId(2), &owned.snapshot()),
        TeamMode::FullRebuild
    );
}

#[test]
fn tank_old_and_losing() {
    // Veterans + losing record but no star — punted year.
    let roster = RosterSpec {
        members: vec![
            (84, 32),
            (82, 31),
            (80, 33),
            (78, 30),
            (77, 29),
            (76, 31),
            (75, 30),
            (74, 29),
            (73, 32),
        ],
    };
    let record = TeamRecordSummary {
        wins: 10,
        losses: 30,
        conf_rank: 13,
        point_diff: -250,
    };
    let owned = build_snapshot(
        TeamId(3),
        "POR",
        GMArchetype::Conservative,
        roster,
        record,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );
    assert_eq!(classify_team(TeamId(3), &owned.snapshot()), TeamMode::Tank);
}

#[test]
fn soft_rebuild_young_with_a_couple_keepers() {
    // Young rotation, two 82+ keepers, middling record.
    let roster = RosterSpec {
        members: vec![
            (85, 23),
            (83, 22),
            (78, 24),
            (76, 22),
            (74, 23),
            (73, 21),
            (72, 22),
            (71, 23),
            (70, 21),
        ],
    };
    let record = TeamRecordSummary {
        wins: 18,
        losses: 22,
        conf_rank: 11,
        point_diff: -50,
    };
    let owned = build_snapshot(
        TeamId(4),
        "ORL",
        GMArchetype::Rebuilder,
        roster,
        record,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
    );
    assert_eq!(
        classify_team(TeamId(4), &owned.snapshot()),
        TeamMode::SoftRebuild
    );
}

// --- apply_context -----------------------------------------------------------

fn neutral_traits() -> GMTraits {
    GMTraits::default()
}

fn pre_deadline_date() -> NaiveDate {
    // 7 days before the 2025-26 deadline (Feb 5, 2026) — inside the 14-day
    // window.
    NaiveDate::from_ymd_opt(2026, 1, 29).unwrap()
}

fn off_season_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 15).unwrap()
}

#[test]
fn pre_deadline_contender_has_high_risk_tolerance() {
    let mut base = neutral_traits();
    base.risk_tolerance = 0.55;
    let adjusted = apply_context(
        &base,
        TeamMode::Contend,
        SeasonPhase::Regular,
        pre_deadline_date(),
    );
    assert!(
        adjusted.risk_tolerance > 0.7,
        "expected risk_tolerance > 0.7, got {}",
        adjusted.risk_tolerance
    );
    assert!(adjusted.current_overall_weight > base.current_overall_weight);
    assert!(adjusted.potential_weight < base.potential_weight);
}

#[test]
fn off_season_rebuilder_has_high_patience() {
    let mut base = neutral_traits();
    base.patience = 0.6;
    let adjusted = apply_context(
        &base,
        TeamMode::FullRebuild,
        SeasonPhase::OffSeason,
        off_season_date(),
    );
    assert!(
        adjusted.patience > 0.85,
        "expected patience > 0.85, got {}",
        adjusted.patience
    );
}

#[test]
fn pre_deadline_does_not_affect_after_deadline() {
    // Day after deadline — no risk_tolerance bump.
    let day_after = NaiveDate::from_ymd_opt(2026, 2, 6).unwrap();
    let mut base = neutral_traits();
    base.risk_tolerance = 0.55;
    let adjusted = apply_context(&base, TeamMode::Contend, SeasonPhase::Regular, day_after);
    // Contend mode still raises risk_tolerance? No — Contend mode only
    // touches current_overall/potential/pick/patience/star_premium, NOT
    // risk_tolerance directly. The pre-deadline branch is the only place
    // risk_tolerance moves for a Contender.
    assert!(
        (adjusted.risk_tolerance - base.risk_tolerance).abs() < 1e-5,
        "risk_tolerance should be untouched after deadline; was {} → {}",
        base.risk_tolerance,
        adjusted.risk_tolerance,
    );
}

#[test]
fn full_rebuild_inverts_contend_emphasis() {
    let base = neutral_traits();
    let contend = apply_context(
        &base,
        TeamMode::Contend,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
    );
    let rebuild = apply_context(
        &base,
        TeamMode::FullRebuild,
        SeasonPhase::Regular,
        NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
    );
    assert!(contend.current_overall_weight > rebuild.current_overall_weight);
    assert!(rebuild.potential_weight > contend.potential_weight);
    assert!(rebuild.pick_value_multiplier > contend.pick_value_multiplier);
    assert!(rebuild.patience > contend.patience);
}
