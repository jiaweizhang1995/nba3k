//! Shared test scaffolding for the nba3k-models integration tests.
//!
//! `LeagueSnapshot` borrows everything, so each test owns its buffers via
//! `OwnedSnapshot` and lends a `LeagueSnapshot<'_>` per assertion.
//!
//! This module is `pub` and consumed across every `tests/<name>.rs` —
//! Rust's integration test runner compiles each file as its own crate, so
//! the unused-warning policy is per-file. The `#[allow(dead_code)]` shrugs
//! off helpers any single test happens not to call.

#![allow(dead_code)]

use chrono::NaiveDate;
use nba3k_core::{
    BirdRights, Cents, Conference, Contract, ContractYear, Division, DraftPick, DraftPickId,
    GMArchetype, GMPersonality, LeagueSnapshot, LeagueYear, Player, PlayerId, Position, Ratings,
    SeasonId, SeasonPhase, Team, TeamId, TeamRecordSummary,
};
use std::collections::HashMap;

pub struct OwnedSnapshot {
    pub teams: Vec<Team>,
    pub players: HashMap<PlayerId, Player>,
    pub picks: HashMap<DraftPickId, DraftPick>,
    pub standings: HashMap<TeamId, TeamRecordSummary>,
    pub season: SeasonId,
    pub phase: SeasonPhase,
    pub date: NaiveDate,
    pub league_year: LeagueYear,
}

impl OwnedSnapshot {
    pub fn snapshot(&self) -> LeagueSnapshot<'_> {
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

pub fn league_year() -> LeagueYear {
    LeagueYear::for_season(SeasonId(2026)).expect("2025-26 encoded")
}

pub fn build_team(team_id: TeamId, abbrev: &str, archetype: GMArchetype) -> Team {
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

pub fn build_player(id: PlayerId, name: &str, team: TeamId, ovr: u8, age: u8) -> Player {
    Player {
        id,
        name: name.to_string(),
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
            signed_in_season: SeasonId(2024),
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

pub struct RosterSpec {
    /// (overall, age) pairs, one per slot. Names are auto-generated.
    pub members: Vec<(u8, u8)>,
}

/// Build a single-team snapshot. Players get auto-generated names; for the
/// star_protection tests that need a specific name, use `build_named_snapshot`.
pub fn build_snapshot(
    team_id: TeamId,
    abbrev: &str,
    archetype: GMArchetype,
    roster: RosterSpec,
    record: TeamRecordSummary,
    phase: SeasonPhase,
    date: NaiveDate,
) -> OwnedSnapshot {
    let team = build_team(team_id, abbrev, archetype);
    let mut players: HashMap<PlayerId, Player> = HashMap::new();
    for (i, (ovr, age)) in roster.members.iter().enumerate() {
        let pid = PlayerId(1000 + i as u32);
        let name = format!("Player{}", pid.0);
        players.insert(pid, build_player(pid, &name, team_id, *ovr, *age));
    }
    let mut standings: HashMap<TeamId, TeamRecordSummary> = HashMap::new();
    standings.insert(team_id, record);

    OwnedSnapshot {
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

/// Spec for a named-player roster. The first entry is the "subject" — used
/// by the star_protection tests as the player under inspection.
pub struct NamedRosterSpec {
    /// (name, ovr, age, potential) tuples.
    pub members: Vec<(&'static str, u8, u8, u8)>,
}

pub fn build_named_snapshot(
    team_id: TeamId,
    abbrev: &str,
    archetype: GMArchetype,
    roster: NamedRosterSpec,
    record: TeamRecordSummary,
) -> OwnedSnapshot {
    let team = build_team(team_id, abbrev, archetype);
    let mut players: HashMap<PlayerId, Player> = HashMap::new();
    for (i, (name, ovr, age, potential)) in roster.members.iter().enumerate() {
        let pid = PlayerId(2000 + i as u32);
        let mut p = build_player(pid, name, team_id, *ovr, *age);
        p.potential = *potential;
        players.insert(pid, p);
    }
    let mut standings: HashMap<TeamId, TeamRecordSummary> = HashMap::new();
    standings.insert(team_id, record);

    OwnedSnapshot {
        teams: vec![team],
        players,
        picks: HashMap::new(),
        standings,
        season: SeasonId(2026),
        phase: SeasonPhase::Regular,
        date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        league_year: league_year(),
    }
}

/// Find the PlayerId of the named player in an OwnedSnapshot.
pub fn find_player_id(owned: &OwnedSnapshot, name: &str) -> PlayerId {
    owned
        .players
        .iter()
        .find(|(_, p)| p.name == name)
        .map(|(id, _)| *id)
        .unwrap_or_else(|| panic!("test bug: player '{}' not found in snapshot", name))
}
