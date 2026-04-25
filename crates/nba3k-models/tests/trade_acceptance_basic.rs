//! Worker D — trade_acceptance composite tests.
//!
//! These tests use [`trade_acceptance_with_providers`] with mocked
//! Worker A/B closures so they don't depend on those workers' bodies
//! being filled in. The workspace integration suite exercises the real
//! wiring after all 4 workers complete.
//!
//! Coverage:
//! - untouchable short-circuit: outgoing star_protection ≥ 0.85 →
//!   Reject("untouchable") regardless of incoming value.
//! - believable counter zone: balanced 1-for-1 → Accept with high
//!   probability.
//! - wildly imbalanced filler-for-star (without untouchable tag) →
//!   Reject(InsufficientValue).
//! - reason composition: |reasons| ≤ top_k_reasons, sorted by |delta|.
//! - determinism: same seed + same inputs → identical output.

use chrono::NaiveDate;
use indexmap::IndexMap;
use nba3k_core::{
    BirdRights, Cents, Coach, Conference, Contract, ContractYear, Division, GMArchetype,
    GMPersonality, LeagueSnapshot, LeagueYear, Player, PlayerId, PlayerRole, Position, Ratings,
    RejectReason, SeasonId, SeasonPhase, Team, TeamId, TeamRecordSummary, TradeAssets, TradeId,
    TradeOffer, Verdict,
};
use nba3k_models::star_protection::StarRoster;
use nba3k_models::team_context::{TeamContext, TeamMode};
use nba3k_models::trade_acceptance::{
    trade_acceptance_with_providers, ComposeWeights, ValueProviders,
};
use nba3k_models::weights::{
    AssetFitWeights, ContractValueWeights, PlayerValueWeights, StarProtectionWeights,
    TeamContextWeights, TradeAcceptanceWeights,
};
use nba3k_models::{Reason, Score};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test world
// ---------------------------------------------------------------------------

struct World {
    teams: Vec<Team>,
    players: HashMap<PlayerId, Player>,
    picks: HashMap<nba3k_core::DraftPickId, nba3k_core::DraftPick>,
    standings: HashMap<TeamId, TeamRecordSummary>,
}

impl World {
    fn new() -> Self {
        Self {
            teams: vec![
                mk_team(1, "BOS", GMArchetype::WinNow),
                mk_team(2, "LAL", GMArchetype::StarHunter),
            ],
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

fn mk_team(id: u8, abbrev: &str, archetype: GMArchetype) -> Team {
    Team {
        id: TeamId(id),
        abbrev: abbrev.into(),
        city: abbrev.into(),
        name: abbrev.into(),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype(format!("{abbrev} GM"), archetype),
        roster: Vec::new(),
        draft_picks: Vec::new(),
        coach: Coach::default(),
    }
}

fn mk_player(id: u32, name: &str, ovr: u8, team: TeamId, salary_dollars: i64) -> Player {
    Player {
        id: PlayerId(id),
        name: name.into(),
        primary_position: Position::SF,
        secondary_position: None,
        age: 27,
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
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

fn two_team_offer(
    from: TeamId,
    to: TeamId,
    send: Vec<PlayerId>,
    receive: Vec<PlayerId>,
) -> TradeOffer {
    let mut assets = IndexMap::new();
    assets.insert(
        from,
        TradeAssets { players_out: send, picks_out: vec![], cash_out: Cents::ZERO },
    );
    assets.insert(
        to,
        TradeAssets { players_out: receive, picks_out: vec![], cash_out: Cents::ZERO },
    );
    TradeOffer { id: TradeId(1), initiator: from, assets_by_team: assets, round: 1, parent: None }
}

// ---------------------------------------------------------------------------
// Mock providers — deterministic Score values keyed off Player.overall
// so we don't depend on Worker A/B function bodies. Tunable per test
// via the `untouchable_pids` set, which makes star_protection return
// 1.0 for that player.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MockKnobs {
    untouchable_pids: std::collections::HashSet<PlayerId>,
    /// PlayerIds that should land in the 0.60-0.85 premium zone.
    premium_pids: std::collections::HashSet<PlayerId>,
    /// Force a specific TeamMode for the evaluator.
    team_mode: TeamMode,
}

impl Default for MockKnobs {
    fn default() -> Self {
        Self {
            untouchable_pids: std::collections::HashSet::new(),
            premium_pids: std::collections::HashSet::new(),
            team_mode: TeamMode::Retool,
        }
    }
}

fn mock_providers<'a>(knobs: &'a MockKnobs) -> ValueProviders<'a> {
    ValueProviders {
        // player_value: $1M per OVR-50 point above 50, with $5M floor.
        // Keeps the math simple and easy to reason about in tests.
        player_value: Box::new(|p, _t, _ev, _l| {
            let dollars = ((p.overall.saturating_sub(50)) as f64) * 1_000_000.0;
            Score::new(dollars * 100.0)
                .with_reason("mock player_value", dollars * 100.0)
        }),
        // contract_value: 0 — mock doesn't care about contracts.
        contract_value: Box::new(|_p, _t, _ly| Score::new(0.0)),
        // star_protection: 1.0 if listed as untouchable, 0.7 if premium,
        // else 0.0. Returns Score with value in [0,1].
        star_protection: Box::new({
            let untouchables = knobs.untouchable_pids.clone();
            let premium = knobs.premium_pids.clone();
            move |pid, _ev, _l, _roster| {
                let v = if untouchables.contains(&pid) {
                    1.0
                } else if premium.contains(&pid) {
                    0.7
                } else {
                    0.0
                };
                Score::new(v).with_reason("mock star_protection", v)
            }
        }),
        // team_context: forced mode.
        team_context: Box::new({
            let mode = knobs.team_mode;
            move |_t, _l| TeamContext {
                mode,
                contend_score: 0.0,
                rebuild_score: 0.0,
                win_now_pressure: 0.0,
                reasons: Vec::new(),
            }
        }),
        // asset_fit: neutral.
        asset_fit: Box::new(|_p, _t, _l| Score::new(0.0)),
    }
}

fn weights_bundle<'w>(
    pv: &'w PlayerValueWeights,
    cv: &'w ContractValueWeights,
    af: &'w AssetFitWeights,
    sp: &'w StarProtectionWeights,
    tc: &'w TeamContextWeights,
    ta: &'w TradeAcceptanceWeights,
) -> ComposeWeights<'w> {
    ComposeWeights {
        player_value: pv,
        contract_value: cv,
        asset_fit: af,
        star_protection: sp,
        team_context: tc,
        trade_acceptance: ta,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const BOS: TeamId = TeamId(1);
const LAL: TeamId = TeamId(2);

#[test]
fn untouchable_short_circuits_to_reject_regardless_of_incoming_value() {
    // LAL has Luka tagged as untouchable. BOS sends FOUR superstars +
    // even more. LAL should still reject with reason "untouchable" —
    // the user's headline behavior.
    let mut world = World::new();
    let luka = mk_player(100, "Luka", 96, LAL, 50_000_000);
    let star1 = mk_player(10, "Brown", 92, BOS, 40_000_000);
    let star2 = mk_player(11, "Tatum", 95, BOS, 45_000_000);
    let star3 = mk_player(12, "White", 88, BOS, 25_000_000);
    let star4 = mk_player(13, "Holiday", 90, BOS, 35_000_000);
    world.add(luka.clone());
    world.add(star1.clone());
    world.add(star2.clone());
    world.add(star3.clone());
    world.add(star4.clone());

    let snap = world.snap();
    let offer = two_team_offer(
        BOS,
        LAL,
        vec![star1.id, star2.id, star3.id, star4.id],
        vec![luka.id],
    );

    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let ta = TradeAcceptanceWeights::default();
    let bundle = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta);

    let mut knobs = MockKnobs::default();
    knobs.untouchable_pids.insert(luka.id);
    knobs.team_mode = TeamMode::Contend;
    let providers = mock_providers(&knobs);

    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let star_roster = StarRoster::default();
    let result = trade_acceptance_with_providers(
        &offer, LAL, &snap, &star_roster, &bundle, &providers, &mut rng,
    );

    match &result.verdict {
        Verdict::Reject(RejectReason::Other(s)) => {
            assert_eq!(s, "untouchable", "reject reason must be 'untouchable'");
        }
        other => panic!("expected Reject(Other(\"untouchable\")), got {other:?}"),
    }
    assert_eq!(result.probability, 0.0, "untouchable short-circuit must be p=0.0");
    assert_eq!(result.net_value, Cents::ZERO);
    assert!(
        result.reasons.iter().any(|r| r.label == "untouchable star"),
        "must surface 'untouchable star' reason; got {:?}",
        result.reasons
    );
    assert_eq!(result.commentary, "Luka is not on the table.");
}

#[test]
fn untouchable_short_circuits_even_with_absurd_incoming_value() {
    // Stress-test the short-circuit: incoming side is *empty* of value
    // assets but evaluator still has to receive something. We make
    // outgoing untouchable; the incoming volume is irrelevant.
    let mut world = World::new();
    let luka = mk_player(100, "Luka", 96, LAL, 50_000_000);
    // 5 incoming stars worth 500M+ combined.
    for i in 0..5 {
        world.add(mk_player(20 + i, &format!("MegaStar{i}"), 99, BOS, 50_000_000));
    }
    world.add(luka.clone());
    let snap = world.snap();

    let incoming_ids: Vec<PlayerId> = (20..25).map(PlayerId).collect();
    let offer = two_team_offer(BOS, LAL, incoming_ids, vec![luka.id]);

    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let ta = TradeAcceptanceWeights::default();
    let bundle = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta);

    let mut knobs = MockKnobs::default();
    knobs.untouchable_pids.insert(luka.id);
    let providers = mock_providers(&knobs);

    let mut rng = ChaCha8Rng::seed_from_u64(0);
    let result = trade_acceptance_with_providers(
        &offer, LAL, &snap, &StarRoster::default(), &bundle, &providers, &mut rng,
    );

    assert!(
        matches!(result.verdict, Verdict::Reject(RejectReason::Other(ref s)) if s == "untouchable"),
        "absurd offer for untouchable star must still reject; got {:?}",
        result.verdict
    );
}

#[test]
fn balanced_one_for_one_lands_in_accept_with_high_probability() {
    // BOS sends an 85-OVR; LAL sends an 89-OVR. From BOS's POV they
    // gain a clear bump — should accept with p ≥ 0.6.
    let mut world = World::new();
    let send = mk_player(10, "Out", 85, BOS, 20_000_000);
    let recv = mk_player(20, "In", 89, LAL, 22_000_000);
    world.add(send.clone());
    world.add(recv.clone());
    let snap = world.snap();
    let offer = two_team_offer(BOS, LAL, vec![send.id], vec![recv.id]);

    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let ta = TradeAcceptanceWeights::default();
    let bundle = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta);

    let knobs = MockKnobs::default(); // no protection, no team-mode tilt
    let providers = mock_providers(&knobs);

    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let result = trade_acceptance_with_providers(
        &offer, BOS, &snap, &StarRoster::default(), &bundle, &providers, &mut rng,
    );

    assert!(
        matches!(result.verdict, Verdict::Accept),
        "small-uplift balanced offer should accept; got {:?} (p={})",
        result.verdict,
        result.probability
    );
    assert!(
        result.probability >= 0.6,
        "probability {} should be ≥ 0.6 on a clearly-positive uplift",
        result.probability
    );
}

#[test]
fn wildly_imbalanced_filler_for_star_is_rejected() {
    // LAL sends a 95 superstar; BOS sends a 65 filler. From LAL's POV
    // the net is deeply negative and should reject.
    let mut world = World::new();
    let star = mk_player(20, "Star", 95, LAL, 45_000_000);
    let filler = mk_player(10, "Filler", 65, BOS, 2_000_000);
    world.add(star.clone());
    world.add(filler.clone());
    let snap = world.snap();
    let offer = two_team_offer(BOS, LAL, vec![filler.id], vec![star.id]);

    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let ta = TradeAcceptanceWeights::default();
    let bundle = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta);

    // Crucially: NOT untouchable. Star is just a star, but tradeable.
    let knobs = MockKnobs::default();
    let providers = mock_providers(&knobs);

    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let result = trade_acceptance_with_providers(
        &offer, LAL, &snap, &StarRoster::default(), &bundle, &providers, &mut rng,
    );

    assert!(
        matches!(result.verdict, Verdict::Reject(RejectReason::InsufficientValue)),
        "filler-for-star must reject as InsufficientValue; got {:?} (p={}, net={})",
        result.verdict,
        result.probability,
        result.net_value.0
    );
    assert!(
        result.net_value.0 < 0,
        "net value should be deeply negative, got {}",
        result.net_value.0
    );
}

#[test]
fn reasons_capped_to_top_k_and_sorted_by_abs_delta() {
    // Build a multi-asset trade so plenty of reasons get generated.
    let mut world = World::new();
    let mut bos_ids = Vec::new();
    let mut lal_ids = Vec::new();
    for i in 0..3 {
        let p = mk_player(10 + i, &format!("BOS{i}"), 80 + i as u8, BOS, 15_000_000);
        bos_ids.push(p.id);
        world.add(p);
    }
    for i in 0..3 {
        let p = mk_player(20 + i, &format!("LAL{i}"), 80 + i as u8, LAL, 15_000_000);
        lal_ids.push(p.id);
        world.add(p);
    }
    let snap = world.snap();
    let offer = two_team_offer(BOS, LAL, bos_ids, lal_ids);

    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let ta = TradeAcceptanceWeights::default();
    let bundle = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta);

    let knobs = MockKnobs::default();
    let providers = mock_providers(&knobs);

    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let result = trade_acceptance_with_providers(
        &offer, BOS, &snap, &StarRoster::default(), &bundle, &providers, &mut rng,
    );

    let top_k = ta.top_k_reasons;
    assert!(
        result.reasons.len() <= top_k,
        "reasons must be ≤ top_k_reasons ({}); got {}",
        top_k,
        result.reasons.len()
    );
    // Sorted by |delta| descending.
    for pair in result.reasons.windows(2) {
        let a = pair[0].delta.abs();
        let b = pair[1].delta.abs();
        assert!(a >= b, "reasons not sorted by |delta| desc: {pair:?}");
    }
}

#[test]
fn deterministic_same_seed_same_inputs_same_output() {
    let mut world = World::new();
    let send = mk_player(10, "Out", 85, BOS, 20_000_000);
    let recv = mk_player(20, "In", 86, LAL, 21_000_000);
    world.add(send.clone());
    world.add(recv.clone());
    let snap = world.snap();
    let offer = two_team_offer(BOS, LAL, vec![send.id], vec![recv.id]);

    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let ta = TradeAcceptanceWeights::default();
    let bundle = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta);

    let knobs = MockKnobs::default();

    let providers_a = mock_providers(&knobs);
    let mut rng_a = ChaCha8Rng::seed_from_u64(1234);
    let a = trade_acceptance_with_providers(
        &offer, BOS, &snap, &StarRoster::default(), &bundle, &providers_a, &mut rng_a,
    );

    let providers_b = mock_providers(&knobs);
    let mut rng_b = ChaCha8Rng::seed_from_u64(1234);
    let b = trade_acceptance_with_providers(
        &offer, BOS, &snap, &StarRoster::default(), &bundle, &providers_b, &mut rng_b,
    );

    assert_eq!(a.probability, b.probability, "p must be deterministic for same seed");
    assert_eq!(a.net_value, b.net_value);
    assert_eq!(a.reasons.len(), b.reasons.len());
    assert_eq!(
        std::mem::discriminant(&a.verdict),
        std::mem::discriminant(&b.verdict)
    );
    // And the reason labels must match in order.
    let labels_a: Vec<_> = a.reasons.iter().map(|r: &Reason| r.label).collect();
    let labels_b: Vec<_> = b.reasons.iter().map(|r: &Reason| r.label).collect();
    assert_eq!(labels_a, labels_b);
}

#[test]
fn premium_zone_protection_makes_offer_harder_to_accept() {
    // Same balanced offer evaluated twice: once with premium-zone
    // protection on the outgoing star, once without. The premium-zone
    // version should produce a *lower* probability because the outgoing
    // side is weighted heavier.
    let mut world = World::new();
    let send = mk_player(10, "Star", 90, BOS, 30_000_000);
    let recv = mk_player(20, "In", 92, LAL, 35_000_000);
    world.add(send.clone());
    world.add(recv.clone());
    let snap = world.snap();
    let offer = two_team_offer(BOS, LAL, vec![send.id], vec![recv.id]);

    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let ta = TradeAcceptanceWeights::default();
    let bundle = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta);

    // Disable noise so the comparison is signal-only.
    let mut ta_noise_off = ta.clone();
    ta_noise_off.gullibility_noise_pct = 0.0;
    let bundle_no_noise = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta_noise_off);

    let knobs_plain = MockKnobs::default();
    let providers_plain = mock_providers(&knobs_plain);
    let mut rng_plain = ChaCha8Rng::seed_from_u64(7);
    let plain = trade_acceptance_with_providers(
        &offer, BOS, &snap, &StarRoster::default(), &bundle_no_noise, &providers_plain,
        &mut rng_plain,
    );

    let mut knobs_premium = MockKnobs::default();
    knobs_premium.premium_pids.insert(send.id);
    let providers_premium = mock_providers(&knobs_premium);
    let mut rng_premium = ChaCha8Rng::seed_from_u64(7);
    let premium = trade_acceptance_with_providers(
        &offer, BOS, &snap, &StarRoster::default(), &bundle_no_noise, &providers_premium,
        &mut rng_premium,
    );

    assert!(
        premium.probability < plain.probability,
        "premium-zone should lower acceptance probability: plain={}, premium={}",
        plain.probability,
        premium.probability
    );
}

#[test]
fn contend_mode_raises_bar_vs_full_rebuild() {
    // Same balanced trade, evaluator in Contend vs FullRebuild — Contend
    // should yield lower probability (need more delivery), FullRebuild
    // higher.
    let mut world = World::new();
    let send = mk_player(10, "Out", 85, BOS, 20_000_000);
    let recv = mk_player(20, "In", 86, LAL, 21_000_000);
    world.add(send.clone());
    world.add(recv.clone());
    let snap = world.snap();
    let offer = two_team_offer(BOS, LAL, vec![send.id], vec![recv.id]);

    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let mut ta = TradeAcceptanceWeights::default();
    ta.gullibility_noise_pct = 0.0; // signal-only comparison
    let bundle = weights_bundle(&pv, &cv, &af, &sp, &tc, &ta);

    let mut knobs_contend = MockKnobs::default();
    knobs_contend.team_mode = TeamMode::Contend;
    let providers_contend = mock_providers(&knobs_contend);
    let mut rng_c = ChaCha8Rng::seed_from_u64(9);
    let contend = trade_acceptance_with_providers(
        &offer, BOS, &snap, &StarRoster::default(), &bundle, &providers_contend, &mut rng_c,
    );

    let mut knobs_rebuild = MockKnobs::default();
    knobs_rebuild.team_mode = TeamMode::FullRebuild;
    let providers_rebuild = mock_providers(&knobs_rebuild);
    let mut rng_r = ChaCha8Rng::seed_from_u64(9);
    let rebuild = trade_acceptance_with_providers(
        &offer, BOS, &snap, &StarRoster::default(), &bundle, &providers_rebuild, &mut rng_r,
    );

    assert!(
        rebuild.probability > contend.probability,
        "FullRebuild ({}) should have higher acceptance prob than Contend ({})",
        rebuild.probability,
        contend.probability
    );
}
