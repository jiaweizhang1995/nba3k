//! Worker D — asset_fit integration tests.
//!
//! Verifies the user's stated intuition: a center incoming on a team
//! with an existing center glut produces a redundancy penalty; the same
//! center on a team without one produces a positional bonus.

use chrono::NaiveDate;
use nba3k_core::{
    BirdRights, Cents, Coach, Conference, Contract, ContractYear, Division, GMArchetype, GMPersonality,
    LeagueSnapshot, LeagueYear, Player, PlayerId, PlayerRole, Position, Ratings, SeasonId, SeasonPhase, Team,
    TeamId, TeamRecordSummary,
};
use nba3k_models::asset_fit::asset_fit;
use nba3k_models::weights::AssetFitWeights;
use std::collections::HashMap;

const TEAM_ID: TeamId = TeamId(1);

fn mk_team(abbrev: &str) -> Team {
    Team {
        id: TEAM_ID,
        abbrev: abbrev.into(),
        city: abbrev.into(),
        name: abbrev.into(),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype(format!("{abbrev} GM"), GMArchetype::WinNow),
        roster: Vec::new(),
        draft_picks: Vec::new(),
        coach: Coach::default(),
    }
}

fn mk_player(id: u32, ovr: u8, pos: Position, team: TeamId) -> Player {
    Player {
        id: PlayerId(id),
        name: format!("P{id}"),
        primary_position: pos,
        secondary_position: None,
        age: 26,
        overall: ovr,
        potential: ovr,
        ratings: Ratings::default(),
        contract: Some(Contract {
            years: vec![ContractYear {
                season: SeasonId(2026),
                salary: Cents::from_dollars(5_000_000),
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
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

struct World {
    teams: Vec<Team>,
    players: HashMap<PlayerId, Player>,
    picks: HashMap<nba3k_core::DraftPickId, nba3k_core::DraftPick>,
    standings: HashMap<TeamId, TeamRecordSummary>,
}

impl World {
    fn new() -> Self {
        Self {
            teams: vec![mk_team("XYZ")],
            players: HashMap::new(),
            picks: HashMap::new(),
            standings: HashMap::new(),
        }
    }

    fn add(&mut self, p: Player) {
        self.players.insert(p.id, p);
    }

    fn snap(&self) -> LeagueSnapshot<'_> {
        LeagueSnapshot {
            current_season: SeasonId(2026),
            current_phase: SeasonPhase::Regular,
            current_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            league_year: LeagueYear::for_season(SeasonId(2026)).unwrap(),
            teams: &self.teams,
            players_by_id: &self.players,
            picks_by_id: &self.picks,
            standings: &self.standings,
        }
    }
}

#[test]
fn center_into_center_glut_is_negative_fit() {
    let mut world = World::new();
    // Top-8 with 4 centers in rotation.
    world.add(mk_player(1, 88, Position::C, TEAM_ID));
    world.add(mk_player(2, 84, Position::C, TEAM_ID));
    world.add(mk_player(3, 82, Position::C, TEAM_ID));
    world.add(mk_player(4, 80, Position::C, TEAM_ID));
    world.add(mk_player(5, 79, Position::PF, TEAM_ID));
    world.add(mk_player(6, 78, Position::SF, TEAM_ID));
    world.add(mk_player(7, 76, Position::SG, TEAM_ID));
    world.add(mk_player(8, 74, Position::PG, TEAM_ID));

    let incoming_c = mk_player(99, 85, Position::C, TeamId(99));
    let weights = AssetFitWeights::default();
    let snap = world.snap();
    let score = asset_fit(&incoming_c, TEAM_ID, &snap, &weights);

    assert!(
        score.value < 0.0,
        "incoming C into center-glut team should be negative fit, got {}",
        score.value
    );

    // The dominant reason should be one of the negative components.
    let top = score.reasons().first().expect("at least one reason");
    assert!(
        top.delta < 0.0,
        "top reason on a glut should be negative, got {top:?}"
    );
}

#[test]
fn center_into_center_void_is_positive_fit() {
    let mut world = World::new();
    // Top-8 with NO centers — incoming C fills a gaping hole.
    world.add(mk_player(1, 88, Position::PG, TEAM_ID));
    world.add(mk_player(2, 84, Position::PG, TEAM_ID));
    world.add(mk_player(3, 82, Position::SG, TEAM_ID));
    world.add(mk_player(4, 80, Position::SG, TEAM_ID));
    world.add(mk_player(5, 79, Position::SF, TEAM_ID));
    world.add(mk_player(6, 78, Position::SF, TEAM_ID));
    world.add(mk_player(7, 76, Position::PF, TEAM_ID));
    world.add(mk_player(8, 74, Position::PF, TEAM_ID));

    let incoming_c = mk_player(99, 85, Position::C, TeamId(99));
    let weights = AssetFitWeights::default();
    let snap = world.snap();
    let score = asset_fit(&incoming_c, TEAM_ID, &snap, &weights);

    assert!(
        score.value > 0.0,
        "incoming C into center-void team should be positive fit, got {}",
        score.value
    );
}

#[test]
fn fit_score_scales_with_player_quality() {
    // A star at the right position fills a need worth more than a role
    // player at the same position.
    let mut world = World::new();
    world.add(mk_player(1, 88, Position::PG, TEAM_ID));
    world.add(mk_player(2, 84, Position::PG, TEAM_ID));
    world.add(mk_player(3, 82, Position::SG, TEAM_ID));
    world.add(mk_player(4, 80, Position::SG, TEAM_ID));
    world.add(mk_player(5, 79, Position::SF, TEAM_ID));
    world.add(mk_player(6, 78, Position::SF, TEAM_ID));
    world.add(mk_player(7, 76, Position::PF, TEAM_ID));
    world.add(mk_player(8, 74, Position::PF, TEAM_ID));

    let weights = AssetFitWeights::default();
    let snap = world.snap();

    let star_c = mk_player(101, 95, Position::C, TeamId(99));
    let role_c = mk_player(102, 72, Position::C, TeamId(99));

    let star_fit = asset_fit(&star_c, TEAM_ID, &snap, &weights);
    let role_fit = asset_fit(&role_c, TEAM_ID, &snap, &weights);

    assert!(
        star_fit.value > role_fit.value,
        "star fit {} should exceed role fit {}",
        star_fit.value,
        role_fit.value
    );
}

#[test]
fn deterministic_same_inputs_same_output() {
    let mut world = World::new();
    world.add(mk_player(1, 88, Position::PG, TEAM_ID));
    world.add(mk_player(2, 84, Position::C, TEAM_ID));
    world.add(mk_player(3, 82, Position::SG, TEAM_ID));

    let incoming = mk_player(99, 85, Position::SF, TeamId(99));
    let weights = AssetFitWeights::default();
    let snap = world.snap();

    let a = asset_fit(&incoming, TEAM_ID, &snap, &weights);
    let b = asset_fit(&incoming, TEAM_ID, &snap, &weights);
    assert_eq!(a.value, b.value);
    assert_eq!(a.reasons.len(), b.reasons.len());
}
