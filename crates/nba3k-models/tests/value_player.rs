//! Worker A — acceptance tests for `player_value` and `contract_value`.
//!
//! Covers the bullets in M4-realism.md "Acceptance — Worker A":
//!  - 95-OVR star age 27 → > $100M
//!  - 70-OVR role player → < $10M
//!  - star premium nonlinearity (87 vs 89 super-linear)
//!  - age cliff (33 vs 28 same OVR — at least 30% drop)
//!  - contract surplus signs and Cheapskate vs WinNow divergence
//!  - loyalty bonus only when evaluator owns the player
//!  - reasons count (≥ 4 baseline; top_k(3) trims to 3)

use chrono::NaiveDate;
use nba3k_core::{
    BirdRights, Cents, Coach, Conference, Contract, ContractYear, DraftPick, DraftPickId, Division,
    GMArchetype, GMPersonality, GMTraits, InjuryStatus, LeagueSnapshot, LeagueYear, Player,
    PlayerId, PlayerRole, Position, Ratings, SeasonId, SeasonPhase, Team, TeamId,
    TeamRecordSummary,
};
use nba3k_models::contract_value::contract_value;
use nba3k_models::player_value::player_value;
use nba3k_models::weights::{ContractValueWeights, PlayerValueWeights};
use std::collections::HashMap;

// --------------------------------------------------------------------------
// Test fixtures
// --------------------------------------------------------------------------

fn league_year_2026() -> LeagueYear {
    LeagueYear::for_season(SeasonId(2026)).expect("2025-26 season encoded")
}

fn mk_player(
    id: u32,
    ovr: u8,
    age: u8,
    pos: Position,
    team: Option<TeamId>,
    contract: Option<Contract>,
) -> Player {
    Player {
        id: PlayerId(id),
        name: format!("Player{id}"),
        primary_position: pos,
        secondary_position: None,
        age,
        overall: ovr,
        potential: ovr,
        ratings: Ratings::default(),
        contract,
        team,
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

fn mk_contract_one_year(salary_dollars: i64, season: SeasonId) -> Contract {
    Contract {
        years: vec![ContractYear {
            season,
            salary: Cents::from_dollars(salary_dollars),
            guaranteed: true,
            team_option: false,
            player_option: false,
        }],
        signed_in_season: SeasonId(season.0 - 1),
        bird_rights: BirdRights::Full,
    }
}

fn mk_contract_multi(
    yearly_dollars: &[i64],
    start: SeasonId,
    player_option_last: bool,
    team_option_last: bool,
) -> Contract {
    let years = yearly_dollars
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let last = i + 1 == yearly_dollars.len();
            ContractYear {
                season: SeasonId(start.0 + i as u16),
                salary: Cents::from_dollars(*d),
                guaranteed: true,
                team_option: last && team_option_last,
                player_option: last && player_option_last,
            }
        })
        .collect();
    Contract {
        years,
        signed_in_season: SeasonId(start.0 - 1),
        bird_rights: BirdRights::Full,
    }
}

/// Build a minimal `LeagueSnapshot` with the given players already inserted.
struct Fixture {
    teams: Vec<Team>,
    players: HashMap<PlayerId, Player>,
    picks: HashMap<DraftPickId, DraftPick>,
    standings: HashMap<TeamId, TeamRecordSummary>,
    league_year: LeagueYear,
}

impl Fixture {
    fn new(players: &[Player]) -> Self {
        let team_a = Team {
            id: TeamId(1),
            abbrev: "AAA".into(),
            city: "Aville".into(),
            name: "Aces".into(),
            conference: Conference::East,
            division: Division::Atlantic,
            gm: GMPersonality::from_archetype("A-GM", GMArchetype::WinNow),
            roster: vec![],
            draft_picks: vec![],
            coach: Coach::default(),
        };
        let team_b = Team {
            id: TeamId(2),
            abbrev: "BBB".into(),
            city: "Beeville".into(),
            name: "Bees".into(),
            conference: Conference::West,
            division: Division::Pacific,
            gm: GMPersonality::from_archetype("B-GM", GMArchetype::Rebuilder),
            roster: vec![],
            draft_picks: vec![],
            coach: Coach::default(),
        };
        let mut by_id = HashMap::new();
        for p in players {
            by_id.insert(p.id, p.clone());
        }
        Self {
            teams: vec![team_a, team_b],
            players: by_id,
            picks: HashMap::new(),
            standings: HashMap::new(),
            league_year: league_year_2026(),
        }
    }

    fn snapshot(&self) -> LeagueSnapshot<'_> {
        LeagueSnapshot {
            current_season: SeasonId(2026),
            current_phase: SeasonPhase::Regular,
            current_date: NaiveDate::from_ymd_opt(2025, 11, 15).unwrap(),
            league_year: self.league_year,
            teams: &self.teams,
            players_by_id: &self.players,
            picks_by_id: &self.picks,
            standings: &self.standings,
        }
    }
}

// --------------------------------------------------------------------------
// player_value
// --------------------------------------------------------------------------

#[test]
fn value_star_95_ovr_age_27_above_100m() {
    let star = mk_player(1, 95, 27, Position::SF, None, None);
    let fixture = Fixture::new(&[star.clone()]);
    let traits = GMTraits::default();
    let weights = PlayerValueWeights::default();

    let score = player_value(&star, &traits, TeamId(1), &fixture.snapshot(), &weights);

    // ≥ $100M = 100_000_000 dollars × 100 cents = 10_000_000_000 cents.
    assert!(
        score.value > 10_000_000_000.0,
        "95-OVR star should value above $100M, got cents={:.0} (~${:.1}M)",
        score.value,
        score.value / 1e8
    );
}

#[test]
fn value_role_70_ovr_below_10m() {
    let role = mk_player(2, 70, 26, Position::SG, None, None);
    let fixture = Fixture::new(&[role.clone()]);
    let traits = GMTraits::default();
    let weights = PlayerValueWeights::default();

    let score = player_value(&role, &traits, TeamId(1), &fixture.snapshot(), &weights);

    // < $10M = 1_000_000_000 cents.
    assert!(
        score.value < 1_000_000_000.0,
        "70-OVR role player should value below $10M, got cents={:.0} (~${:.1}M)",
        score.value,
        score.value / 1e8
    );
}

#[test]
fn value_star_premium_is_nonlinear_above_88() {
    // Compare 87 vs 89: the gap should exceed twice the gap between 86 and 87
    // (which sits below the threshold and is purely baseline-driven).
    let traits = GMTraits::default();
    let weights = PlayerValueWeights::default();
    let p86 = mk_player(86, 86, 27, Position::SF, None, None);
    let p87 = mk_player(87, 87, 27, Position::SF, None, None);
    let p89 = mk_player(89, 89, 27, Position::SF, None, None);
    let fixture = Fixture::new(&[p86.clone(), p87.clone(), p89.clone()]);
    let snap = fixture.snapshot();

    let v86 = player_value(&p86, &traits, TeamId(1), &snap, &weights).value;
    let v87 = player_value(&p87, &traits, TeamId(1), &snap, &weights).value;
    let v89 = player_value(&p89, &traits, TeamId(1), &snap, &weights).value;

    let baseline_step = v87 - v86;
    let star_step = v89 - v87;
    assert!(
        star_step > baseline_step * 2.0,
        "OVR 89 should be > OVR 87 by more than 2× the 86→87 baseline step. \
         86→87={baseline_step:.0}, 87→89={star_step:.0}"
    );
}

#[test]
fn value_age_cliff_drops_at_least_30_pct() {
    // Same OVR, age 33 vs age 28 — bigs decline slower than guards but even
    // a big should drop ≥ 30% in five post-peak years.
    let traits = GMTraits::default();
    let weights = PlayerValueWeights::default();
    let young = mk_player(28, 88, 28, Position::SF, None, None);
    let old = mk_player(33, 88, 33, Position::SF, None, None);
    let fixture = Fixture::new(&[young.clone(), old.clone()]);
    let snap = fixture.snapshot();

    let v_young = player_value(&young, &traits, TeamId(1), &snap, &weights).value;
    let v_old = player_value(&old, &traits, TeamId(1), &snap, &weights).value;

    let drop = (v_young - v_old) / v_young;
    assert!(
        drop >= 0.30,
        "33-yo SF should be ≥ 30% below 28-yo same OVR. \
         young={v_young:.0}, old={v_old:.0}, drop={drop:.3}"
    );
}

#[test]
fn value_loyalty_bonus_only_for_owning_team() {
    // Same player evaluated by their own team vs another team — owner sees
    // a higher value because of the loyalty bonus. Only kicks in when the
    // GM has nonzero loyalty (Loyalist).
    let owner = TeamId(1);
    let other = TeamId(2);
    let player = mk_player(11, 85, 27, Position::SF, Some(owner), None);
    let fixture = Fixture::new(&[player.clone()]);
    let snap = fixture.snapshot();
    let mut traits = GMTraits::default();
    traits.loyalty = 0.6; // Loyalist
    let weights = PlayerValueWeights::default();

    let v_owner = player_value(&player, &traits, owner, &snap, &weights).value;
    let v_other = player_value(&player, &traits, other, &snap, &weights).value;

    assert!(
        v_owner > v_other,
        "owner-side value ({v_owner:.0}) should exceed outsider ({v_other:.0})"
    );
    // Confirm a loyalty_bonus reason was emitted on the owner side.
    let owner_score = player_value(&player, &traits, owner, &snap, &weights);
    assert!(
        owner_score.reasons.iter().any(|r| r.label == "loyalty_bonus"),
        "owner-side score should include 'loyalty_bonus' reason"
    );
}

#[test]
fn value_reasons_at_least_four_then_top_k_to_three() {
    // A player with a contract on the evaluator's roster — that triggers
    // baseline + age + contract_burden + loyalty (and likely gm_age_pref or
    // star_premium too), giving ≥ 4 reasons. After top_k(3), exactly 3.
    let owner = TeamId(1);
    let contract = mk_contract_one_year(40_000_000, SeasonId(2026));
    let player = mk_player(7, 92, 27, Position::SF, Some(owner), Some(contract));
    let fixture = Fixture::new(&[player.clone()]);
    let snap = fixture.snapshot();
    let mut traits = GMTraits::default();
    traits.loyalty = 0.6;
    let weights = PlayerValueWeights::default();

    let mut score = player_value(&player, &traits, owner, &snap, &weights);
    assert!(
        score.reasons.len() >= 4,
        "expected ≥ 4 reasons, got {}: {:?}",
        score.reasons.len(),
        score.reasons.iter().map(|r| r.label).collect::<Vec<_>>()
    );
    score.top_k(3);
    assert_eq!(
        score.reasons.len(),
        3,
        "top_k(3) should leave exactly 3 reasons"
    );
}

// --------------------------------------------------------------------------
// contract_value
// --------------------------------------------------------------------------

#[test]
fn value_contract_surplus_positive_for_underpaid() {
    // OVR 80 player on a 1-year, $10M deal. Should show positive surplus.
    let player = mk_player(20, 80, 27, Position::SF, None, None);
    let cv_w = ContractValueWeights::default();
    let traits = GMTraits::default();
    let ly = league_year_2026();
    let contract = mk_contract_one_year(10_000_000, SeasonId(2026));

    let score = contract_value(&player, Some(&contract), &traits, &ly, &cv_w);
    assert!(
        score.value > 0.0,
        "OVR-80 on $10M for 1yr should be net positive, got {:.0}",
        score.value
    );
}

#[test]
fn value_contract_surplus_negative_for_overpaid() {
    // Same OVR-80 player on a 1-year, $40M deal — overpay → negative surplus.
    let player = mk_player(21, 80, 27, Position::SF, None, None);
    let cv_w = ContractValueWeights::default();
    let traits = GMTraits::default();
    let ly = league_year_2026();
    let contract = mk_contract_one_year(40_000_000, SeasonId(2026));

    let score = contract_value(&player, Some(&contract), &traits, &ly, &cv_w);
    assert!(
        score.value < 0.0,
        "OVR-80 on $40M/1yr should be net negative, got {:.0}",
        score.value
    );
}

#[test]
fn value_cheapskate_more_negative_than_winnow_on_overpay() {
    // Same overpaid player. Cheapskate (salary_aversion=1.8) should see a
    // more negative surplus than WinNow (salary_aversion=0.6).
    let player = mk_player(22, 80, 27, Position::SF, None, None);
    let cv_w = ContractValueWeights::default();
    let ly = league_year_2026();
    let contract = mk_contract_one_year(40_000_000, SeasonId(2026));

    let mut cheap = GMTraits::default();
    cheap.salary_aversion = 1.8;
    let mut winnow = GMTraits::default();
    winnow.salary_aversion = 0.6;

    let v_cheap = contract_value(&player, Some(&contract), &cheap, &ly, &cv_w).value;
    let v_winnow = contract_value(&player, Some(&contract), &winnow, &ly, &cv_w).value;

    assert!(
        v_cheap < v_winnow,
        "Cheapskate ({v_cheap:.0}) should see worse surplus than WinNow ({v_winnow:.0}) on the same overpay"
    );
}

#[test]
fn value_future_year_discount_emits_both_components() {
    // A 4-year deal — confirm both market and salary components are emitted
    // and that the discount math is applied (PV salary < nominal AAV × years).
    let player = mk_player(30, 85, 26, Position::SF, None, None);
    let cv_w = ContractValueWeights::default();
    let traits = GMTraits::default();
    let ly = league_year_2026();
    // 4yr × $25M flat — reasonable for OVR-85.
    let contract = mk_contract_multi(&[25_000_000, 25_000_000, 25_000_000, 25_000_000], SeasonId(2026), false, false);

    let score = contract_value(&player, Some(&contract), &traits, &ly, &cv_w);

    assert!(
        score.reasons.iter().any(|r| r.label == "actual_salary"),
        "should emit actual_salary reason"
    );
    assert!(
        score.reasons.iter().any(|r| r.label == "expected_market"),
        "should emit expected_market reason"
    );
    let actual_salary = score
        .reasons
        .iter()
        .find(|r| r.label == "actual_salary")
        .unwrap()
        .delta;
    // Nominal 4 × $25M = $100M = 10_000_000_000 cents. PV must be smaller.
    assert!(
        actual_salary > -10_000_000_000.0,
        "actual_salary PV ({actual_salary}) should be less negative than -$100M nominal"
    );
}

#[test]
fn value_expiring_premium_only_on_one_year_left() {
    // 1-year contract should emit expiring_premium; 3-year should not.
    let player = mk_player(40, 78, 28, Position::SG, None, None);
    let cv_w = ContractValueWeights::default();
    let traits = GMTraits::default();
    let ly = league_year_2026();

    let one = mk_contract_one_year(20_000_000, SeasonId(2026));
    let three = mk_contract_multi(&[20_000_000, 20_000_000, 20_000_000], SeasonId(2026), false, false);

    let s1 = contract_value(&player, Some(&one), &traits, &ly, &cv_w);
    let s3 = contract_value(&player, Some(&three), &traits, &ly, &cv_w);

    assert!(
        s1.reasons.iter().any(|r| r.label == "expiring_premium"),
        "1-year deal should emit expiring_premium"
    );
    assert!(
        !s3.reasons.iter().any(|r| r.label == "expiring_premium"),
        "3-year deal should not emit expiring_premium"
    );
}

#[test]
fn value_player_option_costs_team() {
    // Same salary structure; one with player option on last year, one without.
    // Player option should be a negative delta to team value.
    let player = mk_player(50, 84, 28, Position::PF, None, None);
    let cv_w = ContractValueWeights::default();
    let traits = GMTraits::default();
    let ly = league_year_2026();

    let no_opt = mk_contract_multi(&[30_000_000, 30_000_000], SeasonId(2026), false, false);
    let with_opt = mk_contract_multi(&[30_000_000, 30_000_000], SeasonId(2026), true, false);

    let v_none = contract_value(&player, Some(&no_opt), &traits, &ly, &cv_w).value;
    let v_popt = contract_value(&player, Some(&with_opt), &traits, &ly, &cv_w).value;

    assert!(
        v_popt < v_none,
        "player option last year should reduce team-side value: \
         no_opt={v_none:.0}, popt={v_popt:.0}"
    );
}
