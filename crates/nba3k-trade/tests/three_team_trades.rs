//! 3-team trade evaluation (M10 worker-a).
//!
//! Verifies that `cba::validate` and the value-and-verdict math
//! (`evaluate::evaluate_with_traits`) handle offers with three sides.
//! The CBA matcher routes incoming salary using the same round-robin
//! convention as `apply_accepted_trade` (team `i`'s outgoing flows to
//! team `(i+1) % n`); the value math sums every other side as incoming
//! since the GM weighs the entire pot they're getting.
//!
//! In M10 the CLI driver requires Unanimous Accept across all 3 teams to fire
//! a 3-team trade — there is no counter-offer flow. These tests therefore
//! call `evaluate_with_traits` once per team and assert each team's verdict
//! directly (the `_with_traits` variant skips the CBA gate + realism
//! resources, mirroring the existing pattern in `evaluate_regression.rs`).

mod cba_common;

use cba_common::*;
use indexmap::IndexMap;
use nba3k_core::{GMTraits, RejectReason, TeamId, TradeAssets, TradeId, TradeOffer, Verdict};
use nba3k_trade::{cba, evaluate::evaluate_with_traits};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

const TEAM_C: TeamId = TeamId(3);

fn three_team_offer(initiator: TeamId, legs: [(TeamId, TradeAssets); 3]) -> TradeOffer {
    let mut map = IndexMap::new();
    for (team, assets) in legs {
        map.insert(team, assets);
    }
    TradeOffer {
        id: TradeId(1),
        initiator,
        assets_by_team: map,
        round: 1,
        parent: None,
    }
}

/// Build a 3-team world where every team is in the **non-apron** tier
/// (~$160M anchor salary, well below apron_1) so salary matching uses the
/// 200%+$250K rule on small movers and easily clears for round-robin
/// pairings of equal-salary movers.
fn balanced_world() -> World {
    let teams = vec![
        make_team(TEAM_A, "AAA"),
        make_team(TEAM_B, "BBB"),
        make_team(TEAM_C, "CCC"),
    ];
    let mut players = Vec::new();

    // Anchor ~$160M each side (non-apron tier).
    players.push(player_on(101, TEAM_A, 1, 160_000_000));
    players.push(player_on(201, TEAM_B, 1, 160_000_000));
    players.push(player_on(301, TEAM_C, 1, 160_000_000));

    // Three peer-grade movers — same OVR (78), age, and salary.
    players.push(player_on(110, TEAM_A, 3, 12_000_000));
    players.push(player_on(210, TEAM_B, 3, 12_000_000));
    players.push(player_on(310, TEAM_C, 3, 12_000_000));

    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    pad_roster(&mut w, TEAM_C, 14, 3_000);
    w
}

/// World where TEAM_B is the side getting hosed: it ships out a 92-OVR star
/// while only receiving 78-OVR peers from TEAM_A and TEAM_C. Pairwise salary
/// matching still passes because all movers are $24M.
fn lopsided_world() -> World {
    let teams = vec![
        make_team(TEAM_A, "AAA"),
        make_team(TEAM_B, "BBB"),
        make_team(TEAM_C, "CCC"),
    ];
    let mut players = Vec::new();

    players.push(player_on(101, TEAM_A, 1, 160_000_000));
    players.push(player_on(201, TEAM_B, 1, 160_000_000));
    players.push(player_on(301, TEAM_C, 1, 160_000_000));

    players.push(player_on(110, TEAM_A, 3, 24_000_000));
    players.push(player_on(310, TEAM_C, 3, 24_000_000));

    let mut star = player_on(210, TEAM_B, 3, 24_000_000);
    star.overall = 92;
    star.potential = 92;
    star.age = 27;
    players.push(star);

    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    pad_roster(&mut w, TEAM_C, 14, 3_000);
    w
}

#[test]
fn three_team_balance_passes_cba() {
    // Each team's outgoing $12M matches its incoming $12M (round-robin).
    let w = balanced_world();
    let snap = w.snapshot();
    let offer = three_team_offer(
        TEAM_A,
        [
            (TEAM_A, assets_players(&[110])),
            (TEAM_B, assets_players(&[210])),
            (TEAM_C, assets_players(&[310])),
        ],
    );
    let res = cba::validate(&offer, &snap);
    assert!(
        res.is_ok(),
        "expected balanced 3-team trade to pass CBA, got {res:?}"
    );
}

#[test]
fn three_team_balance_unanimous_accept_via_traits() {
    // Three peer-grade legs → each side's value math sees +incoming roughly
    // matching outgoing (the realism asset_fit/team_context modifiers are
    // skipped here; this is a value-and-verdict math regression test, the
    // same shape as `evaluate_equal_value_swap_accepted` in the unit tests
    // but extended to 3 teams). Unanimous Accept fires with deterministic
    // (gullibility=0) traits.
    let w = balanced_world();
    let snap = w.snapshot();
    let offer = three_team_offer(
        TEAM_A,
        [
            (TEAM_A, assets_players(&[110])),
            (TEAM_B, assets_players(&[210])),
            (TEAM_C, assets_players(&[310])),
        ],
    );

    let mut traits = GMTraits::default();
    traits.gullibility = 0.0;

    for team in [TEAM_A, TEAM_B, TEAM_C] {
        let mut rng = ChaCha8Rng::seed_from_u64(team.0 as u64);
        let eval = evaluate_with_traits(&offer, team, &snap, &traits, &mut rng);
        match eval.verdict {
            Verdict::Accept => {}
            other => panic!(
                "team {:?} did not Accept a peer-grade 3-team trade: {:?} (net={})",
                team, other, eval.net_value.0
            ),
        }
    }
}

#[test]
fn three_team_one_dumper_rejects() {
    // TEAM_B ships a 92-OVR star for two 78-OVR peers. Outgoing value far
    // exceeds incoming; TEAM_B should Reject with InsufficientValue. The
    // other two teams are fine with the trade — they're getting peer value
    // in addition to a star piece — but unanimous Accept won't fire.
    let w = lopsided_world();
    let snap = w.snapshot();
    let offer = three_team_offer(
        TEAM_A,
        [
            (TEAM_A, assets_players(&[110])),
            (TEAM_B, assets_players(&[210])),
            (TEAM_C, assets_players(&[310])),
        ],
    );

    // Pairwise salary matching ($24M ↔ $24M each leg) clears CBA.
    let res = cba::validate(&offer, &snap);
    assert!(res.is_ok(), "lopsided 3-team should pass CBA, got {res:?}");

    let mut traits = GMTraits::default();
    traits.gullibility = 0.0;
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let eval_b = evaluate_with_traits(&offer, TEAM_B, &snap, &traits, &mut rng);
    match eval_b.verdict {
        Verdict::Reject(RejectReason::InsufficientValue) => {}
        other => panic!(
            "TEAM_B should Reject(InsufficientValue) shipping a star for two peers; \
             got verdict={:?} net={}",
            other, eval_b.net_value.0
        ),
    }
}

#[test]
fn three_team_offer_shape() {
    let offer = three_team_offer(
        TEAM_A,
        [
            (TEAM_A, assets_players(&[1])),
            (TEAM_B, assets_players(&[2])),
            (TEAM_C, assets_players(&[3])),
        ],
    );
    assert_eq!(offer.assets_by_team.len(), 3);
    assert_eq!(offer.round, 1);
    assert_eq!(offer.initiator, TEAM_A);
    let keys: Vec<TeamId> = offer.assets_by_team.keys().copied().collect();
    assert_eq!(keys, vec![TEAM_A, TEAM_B, TEAM_C]);
}

#[test]
fn cba_incoming_origin_round_robin() {
    // Direct check on the helper: in round-robin order A→B→C→A, A's origin
    // is C, B's origin is A, C's origin is B.
    let offer = three_team_offer(
        TEAM_A,
        [
            (TEAM_A, assets_players(&[1])),
            (TEAM_B, assets_players(&[2])),
            (TEAM_C, assets_players(&[3])),
        ],
    );
    assert_eq!(cba::incoming_origin(TEAM_A, &offer), Some(TEAM_C));
    assert_eq!(cba::incoming_origin(TEAM_B, &offer), Some(TEAM_A));
    assert_eq!(cba::incoming_origin(TEAM_C, &offer), Some(TEAM_B));
}
