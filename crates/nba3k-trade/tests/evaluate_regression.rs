//! Worker A integration tests — regression for the three acceptance
//! requirements:
//!  1. star-for-filler returns Reject(InsufficientValue) with net% ≤ -15%.
//!  2. equal-value swap (same OVR ± 2, same age ± 2, comparable contract)
//!     returns Accept.
//!  3. same offer evaluated by Cheapskate vs WinNow produces different
//!     verdicts.
//!
//! These tests deliberately call `evaluate_with_traits` (not `evaluate`) so
//! they don't transitively invoke the still-`todo!()` bodies in
//! `cba::validate` / `context::apply_context` / `personality::*`. Once the
//! other workers land, the workspace tests in `cargo test --workspace` will
//! cover the full pipeline end-to-end.

use chrono::NaiveDate;
use indexmap::IndexMap;
use nba3k_core::{
    BirdRights, Cents, Conference, Contract, ContractYear, Division, DraftPick, DraftPickId,
    GMArchetype, GMPersonality, GMTraits, LeagueYear, Player, PlayerId, Position, Ratings,
    RejectReason, SeasonId, SeasonPhase, Team, TeamId, TradeAssets, TradeId, TradeOffer, Verdict,
};
use nba3k_trade::evaluate::evaluate_with_traits;
use nba3k_trade::snapshot::LeagueSnapshot;
use nba3k_trade::TeamRecordSummary;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

struct World {
    teams: Vec<Team>,
    players: HashMap<PlayerId, Player>,
    picks: HashMap<DraftPickId, DraftPick>,
    standings: HashMap<TeamId, TeamRecordSummary>,
}

impl World {
    fn new() -> Self {
        Self {
            teams: vec![
                mk_team(1, "BOS", GMArchetype::Conservative),
                mk_team(2, "LAL", GMArchetype::StarHunter),
                mk_team(3, "OKC", GMArchetype::Cheapskate),
            ],
            players: HashMap::new(),
            picks: HashMap::new(),
            standings: HashMap::new(),
        }
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

fn mk_team(id: u8, abbrev: &str, arch: GMArchetype) -> Team {
    Team {
        id: TeamId(id),
        abbrev: abbrev.into(),
        city: abbrev.into(),
        name: abbrev.into(),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype(format!("{abbrev} GM"), arch),
        roster: Vec::new(),
        draft_picks: Vec::new(),
        coach: nba3k_core::Coach::default_for(abbrev),
    }
}

fn mk_player(id: u32, ovr: u8, age: u8, salary_dollars: i64, team: TeamId) -> Player {
    Player {
        id: PlayerId(id),
        name: format!("P{id}"),
        primary_position: Position::SF,
        secondary_position: None,
        age,
        overall: ovr,
        potential: ovr,
        ratings: Ratings::default(),
        contract: Some(Contract {
            years: vec![ContractYear {
                season: SeasonId(2026),
                salary: Cents::from_dollars(salary_dollars),
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

fn two_team_offer(
    initiator: TeamId,
    counter: TeamId,
    initiator_send: Vec<PlayerId>,
    counter_send: Vec<PlayerId>,
) -> TradeOffer {
    let mut assets = IndexMap::new();
    assets.insert(
        initiator,
        TradeAssets {
            players_out: initiator_send,
            picks_out: vec![],
            cash_out: Cents::ZERO,
        },
    );
    assets.insert(
        counter,
        TradeAssets {
            players_out: counter_send,
            picks_out: vec![],
            cash_out: Cents::ZERO,
        },
    );
    TradeOffer {
        id: TradeId(1),
        initiator,
        assets_by_team: assets,
        round: 1,
        parent: None,
    }
}

#[test]
fn evaluate_star_for_filler_rejected() {
    // BOS sends a 70-OVR filler, LAL sends a 95-OVR star. Evaluate from
    // LAL's POV — they must reject.
    let mut world = World::new();
    let bos = TeamId(1);
    let lal = TeamId(2);
    let filler = mk_player(10, 70, 25, 5_000_000, bos);
    let star = mk_player(20, 95, 28, 50_000_000, lal);
    world.players.insert(filler.id, filler.clone());
    world.players.insert(star.id, star.clone());
    let snap = world.snap();

    let offer = two_team_offer(bos, lal, vec![filler.id], vec![star.id]);
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let traits = GMTraits::default();
    let eval = evaluate_with_traits(&offer, lal, &snap, &traits, &mut rng);
    assert!(
        matches!(
            eval.verdict,
            Verdict::Reject(RejectReason::InsufficientValue)
        ),
        "star-for-filler should be Reject(InsufficientValue), got {:?}",
        eval.verdict
    );
    assert!(eval.net_value.0 < 0);
}

#[test]
fn evaluate_equal_value_swap_accepted() {
    // OVR 80 vs 81, both age 26, both $22M. Slight uplift for receiver
    // should land in Accept.
    let mut world = World::new();
    let bos = TeamId(1);
    let lal = TeamId(2);
    let send = mk_player(10, 80, 26, 22_000_000, bos);
    let recv = mk_player(20, 81, 27, 22_000_000, lal);
    world.players.insert(send.id, send.clone());
    world.players.insert(recv.id, recv.clone());
    let snap = world.snap();

    let offer = two_team_offer(bos, lal, vec![send.id], vec![recv.id]);
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let traits = GMTraits::default();
    let eval = evaluate_with_traits(&offer, bos, &snap, &traits, &mut rng);
    assert!(
        matches!(eval.verdict, Verdict::Accept),
        "equal-value with slight uplift should Accept, got {:?}",
        eval.verdict
    );
}

#[test]
fn evaluate_winnow_vs_cheap_diverge_on_aging_star() {
    // Same offer evaluated by two different GM personalities should produce
    // different verdicts. Setup: outgoing = young cheap rotation guy;
    // incoming = aging star on max contract. WinNow loves the talent now;
    // Cheapskate hates the cap hit + age tail.
    let mut world = World::new();
    let okc = TeamId(3); // receives the star
    let lal = TeamId(2);
    let outgoing = mk_player(10, 78, 23, 4_000_000, okc);
    let incoming = mk_player(20, 90, 33, 48_000_000, lal);
    world.players.insert(outgoing.id, outgoing.clone());
    world.players.insert(incoming.id, incoming.clone());
    let snap = world.snap();

    let offer = two_team_offer(okc, lal, vec![outgoing.id], vec![incoming.id]);
    let win_now = GMPersonality::from_archetype("WN", GMArchetype::WinNow).traits;
    let cheap = GMPersonality::from_archetype("CS", GMArchetype::Cheapskate).traits;

    let mut rng_a = ChaCha8Rng::seed_from_u64(0);
    let mut rng_b = ChaCha8Rng::seed_from_u64(0);
    let win_eval = evaluate_with_traits(&offer, okc, &snap, &win_now, &mut rng_a);
    let cheap_eval = evaluate_with_traits(&offer, okc, &snap, &cheap, &mut rng_b);

    assert_ne!(
        win_eval.net_value.0, cheap_eval.net_value.0,
        "WinNow vs Cheapskate must produce different net values for same offer"
    );
    assert_ne!(
        std::mem::discriminant(&win_eval.verdict),
        std::mem::discriminant(&cheap_eval.verdict),
        "WinNow vs Cheapskate must diverge on verdict; win={:?} cheap={:?}",
        win_eval.verdict,
        cheap_eval.verdict
    );
}
