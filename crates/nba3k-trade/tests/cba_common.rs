//! Shared fixture builders for the `cba_*` integration tests.
//!
//! Imported via `mod cba_common;` from each test file. This module is its
//! own test target on disk only because cargo treats every `tests/*.rs` as
//! an integration target — but the harness is tiny and `#[allow(dead_code)]`
//! keeps unused helpers from breaking individual tests.

#![allow(dead_code)]

use chrono::NaiveDate;
use indexmap::IndexMap;
use nba3k_core::{
    Cents, Conference, Contract, ContractYear, Division, DraftPick, DraftPickId, GMArchetype,
    GMPersonality, LeagueYear, Player, PlayerId, Position, Ratings, SeasonId, SeasonPhase, Team,
    TeamId, TradeAssets, TradeId, TradeOffer,
};
use nba3k_trade::snapshot::{LeagueSnapshot, TeamRecordSummary};
use std::collections::HashMap;

pub const SEASON: SeasonId = SeasonId(2026);
pub const TEAM_A: TeamId = TeamId(1);
pub const TEAM_B: TeamId = TeamId(2);

pub fn ly() -> LeagueYear {
    LeagueYear::for_season(SEASON).expect("2025-26 LeagueYear must exist")
}

/// One contract year, fully guaranteed, no options.
pub fn salary_year(season: SeasonId, dollars: i64) -> ContractYear {
    ContractYear {
        season,
        salary: Cents::from_dollars(dollars),
        guaranteed: true,
        team_option: false,
        player_option: false,
    }
}

pub fn flat_contract(years: u8, dollars_per_year: i64) -> Contract {
    let mut yrs = Vec::with_capacity(years as usize);
    for i in 0..years {
        yrs.push(salary_year(SeasonId(SEASON.0 + i as u16), dollars_per_year));
    }
    Contract {
        years: yrs,
        signed_in_season: SEASON,
        bird_rights: nba3k_core::BirdRights::Full,
    }
}

pub fn make_player(id: u32, team: TeamId, contract: Option<Contract>) -> Player {
    Player {
        id: PlayerId(id),
        name: format!("Player{id}"),
        primary_position: Position::SF,
        secondary_position: None,
        age: 27,
        overall: 78,
        potential: 80,
        ratings: Ratings::default(),
        contract,
        team: Some(team),
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: nba3k_core::PlayerRole::default(),
        morale: 0.5,
    }
}

/// Helper: a player on `team` at $`dollars`/yr for `years`, no kicker, no NTC.
pub fn player_on(id: u32, team: TeamId, years: u8, dollars_per_year: i64) -> Player {
    make_player(id, team, Some(flat_contract(years, dollars_per_year)))
}

pub fn make_team(id: TeamId, abbrev: &str) -> Team {
    Team {
        id,
        abbrev: abbrev.into(),
        city: format!("City{}", id.0),
        name: format!("Name{}", id.0),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
        roster: Vec::new(),
        draft_picks: Vec::new(),
        coach: nba3k_core::Coach::default_for(abbrev),
    }
}

/// Container that owns the data a `LeagueSnapshot` borrows from.
pub struct World {
    pub teams: Vec<Team>,
    pub players: HashMap<PlayerId, Player>,
    pub picks: HashMap<DraftPickId, DraftPick>,
    pub standings: HashMap<TeamId, TeamRecordSummary>,
}

impl World {
    pub fn new(teams: Vec<Team>, players: Vec<Player>) -> Self {
        let mut player_map = HashMap::new();
        for p in players {
            player_map.insert(p.id, p);
        }
        Self {
            teams,
            players: player_map,
            picks: HashMap::new(),
            standings: HashMap::new(),
        }
    }

    pub fn snapshot(&self) -> LeagueSnapshot<'_> {
        self.snapshot_with_phase(SeasonPhase::Regular)
    }

    pub fn snapshot_with_phase(&self, phase: SeasonPhase) -> LeagueSnapshot<'_> {
        LeagueSnapshot {
            current_season: SEASON,
            current_phase: phase,
            current_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            league_year: ly(),
            teams: &self.teams,
            players_by_id: &self.players,
            picks_by_id: &self.picks,
            standings: &self.standings,
        }
    }
}

/// Two-team trade builder.
pub fn two_team_offer(
    a: TeamId,
    a_assets: TradeAssets,
    b: TeamId,
    b_assets: TradeAssets,
) -> TradeOffer {
    let mut map = IndexMap::new();
    map.insert(a, a_assets);
    map.insert(b, b_assets);
    TradeOffer {
        id: TradeId(1),
        initiator: a,
        assets_by_team: map,
        round: 1,
        parent: None,
    }
}

/// Convenience: build assets where `team` is sending out exactly the listed
/// player IDs (and no picks, no cash).
pub fn assets_players(players: &[u32]) -> TradeAssets {
    TradeAssets {
        players_out: players.iter().copied().map(PlayerId).collect(),
        picks_out: Vec::new(),
        cash_out: Cents::ZERO,
    }
}

pub fn assets_with_cash(players: &[u32], cash: Cents) -> TradeAssets {
    TradeAssets {
        players_out: players.iter().copied().map(PlayerId).collect(),
        picks_out: Vec::new(),
        cash_out: cash,
    }
}

/// Pad a team's roster with cheap minimum-salary players up to `target_size`
/// total (existing roster + added). Used so post-trade roster sizes land in
/// the legal 13–15 window without other side-effects.
pub fn pad_roster(world: &mut World, team: TeamId, target_size: usize, start_id: u32) {
    let current: usize = world
        .players
        .values()
        .filter(|p| p.team == Some(team))
        .count();
    if current >= target_size {
        return;
    }
    let to_add = target_size - current;
    for i in 0..to_add {
        let id = start_id + i as u32;
        // Minimum-salary $1M flat for 1 yr — won't tip apron tiers in tests.
        let p = player_on(id, team, 1, 1_000_000);
        world.players.insert(p.id, p);
    }
}
