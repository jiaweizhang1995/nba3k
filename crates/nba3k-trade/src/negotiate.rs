//! Counter-offer state machine. Worker D.
//!
//! Public surface:
//! - [`step`] advances a [`NegotiationState`] one round.
//! - [`generate_counter`] produces the receiving GM's counter offer.
//!
//! `step` calls [`crate::evaluate::evaluate`] for the verdict and
//! [`crate::cba::validate`] before accepting any generated counter.
//!
//! For unit testing in isolation from Workers A and C, the internals are
//! parameterised via [`step_with`] / [`generate_counter_with`] over
//! [`EvalFn`] / [`ValidateFn`] function pointers; the public entry points
//! wire those to the real workers, while tests pass mock implementations.

use crate::cba::{self, CbaViolation};
use crate::snapshot::LeagueSnapshot;
use nba3k_core::{
    Cents, GMArchetype, NegotiationState, Player, PlayerId, RejectReason, TeamId, TradeAssets,
    TradeId, TradeOffer, Verdict,
};
use rand::{Rng, RngCore};
use rand_distr::{Distribution, Normal};

/// Maximum chain length before we declare the negotiation `Stalled`.
pub const MAX_CHAIN_LEN: usize = 5;

// ---------------------------------------------------------------------------
// Function-pointer surfaces — abstraction over Worker A/C so tests don't
// depend on them. Function pointers (not closures) sidestep the higher-ranked
// lifetime inference pain that blanket-impl `FnMut` traits run into here.
// ---------------------------------------------------------------------------

pub type EvalFn = for<'a> fn(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot<'a>,
    rng: &mut dyn RngCore,
) -> nba3k_core::TradeEvaluation;

pub type ValidateFn =
    for<'a> fn(offer: &TradeOffer, league: &LeagueSnapshot<'a>) -> Result<(), CbaViolation>;

// ---------------------------------------------------------------------------
// Public API — real wiring.
// ---------------------------------------------------------------------------

pub fn step(
    state: NegotiationState,
    league: &LeagueSnapshot,
    rng: &mut dyn RngCore,
) -> NegotiationState {
    step_with(state, league, rng, crate::evaluate::evaluate, cba::validate)
}

pub fn generate_counter(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    rng: &mut dyn RngCore,
) -> Option<TradeOffer> {
    generate_counter_with(
        offer,
        evaluator,
        league,
        rng,
        crate::evaluate::evaluate,
        cba::validate,
    )
}

// ---------------------------------------------------------------------------
// Internal step / generator parameterised on EvalFn + ValidateFn.
// ---------------------------------------------------------------------------

pub fn step_with(
    state: NegotiationState,
    league: &LeagueSnapshot,
    rng: &mut dyn RngCore,
    evaluator_fn: EvalFn,
    validator_fn: ValidateFn,
) -> NegotiationState {
    let mut chain = match state {
        NegotiationState::Open { chain } => chain,
        // Terminal states never re-advance.
        s @ (NegotiationState::Accepted(_)
        | NegotiationState::Rejected { .. }
        | NegotiationState::Stalled) => return s,
    };

    if chain.is_empty() {
        return NegotiationState::Stalled;
    }
    if chain.len() >= MAX_CHAIN_LEN {
        return NegotiationState::Stalled;
    }

    let latest = chain.last().expect("non-empty chain").clone();
    let receiver = match receiving_team(&latest) {
        Some(t) => t,
        None => {
            return NegotiationState::Rejected {
                final_offer: latest,
                reason: RejectReason::Other("malformed offer: no receiver".into()),
            };
        }
    };

    let evaluation = evaluator_fn(&latest, receiver, league, rng);

    match evaluation.verdict {
        Verdict::Accept => NegotiationState::Accepted(latest),
        Verdict::Reject(reason) => NegotiationState::Rejected {
            final_offer: latest,
            reason,
        },
        Verdict::Counter(_) => {
            match generate_counter_with(&latest, receiver, league, rng, evaluator_fn, validator_fn)
            {
                Some(counter) => {
                    chain.push(counter);
                    if chain.len() >= MAX_CHAIN_LEN {
                        NegotiationState::Stalled
                    } else {
                        NegotiationState::Open { chain }
                    }
                }
                None => NegotiationState::Rejected {
                    final_offer: latest,
                    reason: RejectReason::BadFaith,
                },
            }
        }
    }
}

pub fn generate_counter_with(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    rng: &mut dyn RngCore,
    evaluator_fn: EvalFn,
    validator_fn: ValidateFn,
) -> Option<TradeOffer> {
    if !offer.is_two_team() {
        return None;
    }
    let initiator = offer.initiator;
    if initiator == evaluator {
        return None;
    }

    let archetype = league
        .team(evaluator)
        .map(|t| t.gm.archetype)
        .unwrap_or(GMArchetype::Conservative);

    let move_kind = pick_move(archetype, rng);

    let attempt = match move_kind {
        Move::Add => apply_add(offer, evaluator, initiator, league, rng),
        Move::Swap => apply_swap(offer, evaluator, initiator, league, rng),
        Move::Subtract => apply_subtract(offer, evaluator, league, rng),
    };

    let counter = attempt.or_else(|| apply_add(offer, evaluator, initiator, league, rng))?;

    // Validate against CBA. On failure, retry once with a cash-add fallback.
    if validator_fn(&counter, league).is_ok() {
        let _ = evaluator_fn; // evaluator-side sanity is rerun on next `step`.
        return Some(counter);
    }

    let fallback = cash_add_fallback(&counter, evaluator, league)?;
    if validator_fn(&fallback, league).is_ok() {
        Some(fallback)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Move selection — personality-weighted Add / Swap / Subtract.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Move {
    Add,
    Swap,
    Subtract,
}

/// Personality weighting:
/// - **Conservative**: Add only (never Swap aggressively, never Subtract).
/// - **Aggressive**: +0.4 Subtract, +0.3 Swap on top of a baseline.
/// - **Wildcard**: baseline weights + gaussian noise.
/// - **Default**: 0.6 Add / 0.35 Swap / 0.05 Subtract.
fn pick_move(archetype: GMArchetype, rng: &mut dyn RngCore) -> Move {
    let (mut add, mut swap, mut subtract) = (0.60_f32, 0.35_f32, 0.05_f32);

    match archetype {
        GMArchetype::Conservative => {
            // Conservative: Add only. Never Subtract under any RNG.
            return Move::Add;
        }
        GMArchetype::Aggressive => {
            add = 0.20;
            swap = 0.35; // baseline 0.35 (already includes the +0.30 over a
                         // pure-Add baseline of ~0.05).
            subtract = 0.45; // baseline 0.05 + 0.40.
        }
        GMArchetype::Wildcard => {
            // Wildcard: noisy weights — gaussian jitter then renormalize.
            let n = Normal::new(0.0_f32, 0.15_f32).unwrap();
            add = (add + n.sample(rng)).max(0.05);
            swap = (swap + n.sample(rng)).max(0.05);
            subtract = (subtract + n.sample(rng)).max(0.0);
        }
        GMArchetype::OldSchool | GMArchetype::Loyalist | GMArchetype::Cheapskate => {
            // Risk-averse archetypes — never Subtract.
            subtract = 0.0;
            add = 0.65;
            swap = 0.35;
        }
        _ => {}
    }

    let total = add + swap + subtract;
    let roll: f32 = rng.gen_range(0.0..total);
    if roll < add {
        Move::Add
    } else if roll < add + swap {
        Move::Swap
    } else {
        Move::Subtract
    }
}

// ---------------------------------------------------------------------------
// Move implementations.
// ---------------------------------------------------------------------------

/// Add: request the next-most-valuable asset from the initiator's roster
/// they haven't already included in `players_out`.
fn apply_add(
    offer: &TradeOffer,
    evaluator: TeamId,
    initiator: TeamId,
    league: &LeagueSnapshot,
    _rng: &mut dyn RngCore,
) -> Option<TradeOffer> {
    let already_out: Vec<PlayerId> = offer
        .assets_by_team
        .get(&initiator)
        .map(|a| a.players_out.clone())
        .unwrap_or_default();

    let candidate = best_initiator_candidate(initiator, &already_out, league)?;
    let mut next = bump(offer);
    next.assets_by_team
        .entry(initiator)
        .or_insert_with(TradeAssets::default)
        .players_out
        .push(candidate);
    let _ = evaluator; // evaluator-targeted heuristics live in Worker A.
    Some(next)
}

/// Swap: replace one of evaluator's outgoing players with a *higher-value*
/// initiator player and drop a low-value asset. Implemented as: drop the
/// lowest-OVR outgoing player on the evaluator's side AND request a higher-OVR
/// player from the initiator we don't already include.
fn apply_swap(
    offer: &TradeOffer,
    evaluator: TeamId,
    initiator: TeamId,
    league: &LeagueSnapshot,
    rng: &mut dyn RngCore,
) -> Option<TradeOffer> {
    let outgoing: Vec<PlayerId> = offer
        .assets_by_team
        .get(&evaluator)
        .map(|a| a.players_out.clone())
        .unwrap_or_default();
    if outgoing.is_empty() {
        return apply_add(offer, evaluator, initiator, league, rng);
    }

    // Drop the lowest-OVR outgoing player on the evaluator's side.
    let drop_idx = outgoing
        .iter()
        .enumerate()
        .filter_map(|(i, pid)| league.player(*pid).map(|p| (i, p.overall)))
        .min_by_key(|(_, ovr)| *ovr)
        .map(|(i, _)| i);

    let already_out: Vec<PlayerId> = offer
        .assets_by_team
        .get(&initiator)
        .map(|a| a.players_out.clone())
        .unwrap_or_default();
    let candidate = best_initiator_candidate(initiator, &already_out, league)?;

    let mut next = bump(offer);
    if let Some(idx) = drop_idx {
        if let Some(eval_assets) = next.assets_by_team.get_mut(&evaluator) {
            if idx < eval_assets.players_out.len() {
                eval_assets.players_out.remove(idx);
            }
        }
    }
    next.assets_by_team
        .entry(initiator)
        .or_insert_with(TradeAssets::default)
        .players_out
        .push(candidate);
    Some(next)
}

/// Subtract: remove a low-value asset evaluator was giving. Bad-faith move.
/// Returns `None` if the evaluator isn't sending more than one asset (you
/// can't subtract from a one-asset side).
fn apply_subtract(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    _rng: &mut dyn RngCore,
) -> Option<TradeOffer> {
    let outgoing: Vec<PlayerId> = offer
        .assets_by_team
        .get(&evaluator)
        .map(|a| a.players_out.clone())
        .unwrap_or_default();
    if outgoing.len() <= 1 {
        // Subtracting your only outgoing asset isn't a counter, it's a refusal.
        return None;
    }

    let drop_idx = outgoing
        .iter()
        .enumerate()
        .filter_map(|(i, pid)| league.player(*pid).map(|p| (i, p.overall)))
        .min_by_key(|(_, ovr)| *ovr)
        .map(|(i, _)| i)?;

    let mut next = bump(offer);
    if let Some(eval_assets) = next.assets_by_team.get_mut(&evaluator) {
        if drop_idx < eval_assets.players_out.len() {
            eval_assets.players_out.remove(drop_idx);
        }
    }
    Some(next)
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// Pick the highest-OVR player on `team`'s roster who isn't already in
/// `excluded`. Used by Add and Swap to find a target asset.
fn best_initiator_candidate(
    team: TeamId,
    excluded: &[PlayerId],
    league: &LeagueSnapshot,
) -> Option<PlayerId> {
    let roster = league.roster(team);
    roster
        .into_iter()
        .filter(|p| !excluded.contains(&p.id))
        .max_by_key(|p: &&Player| p.overall)
        .map(|p| p.id)
}

/// `step` looks at the latest offer; the receiver is whichever team in the
/// offer is *not* the initiator. v1 is two-team only.
fn receiving_team(offer: &TradeOffer) -> Option<TeamId> {
    offer
        .assets_by_team
        .keys()
        .copied()
        .find(|t| *t != offer.initiator)
}

/// Build a fresh offer that increments `round` and points `parent` at the
/// current offer's id. Asset maps are cloned and then mutated by the caller.
fn bump(offer: &TradeOffer) -> TradeOffer {
    TradeOffer {
        id: TradeId(offer.id.0.wrapping_add(1)),
        initiator: offer.initiator,
        assets_by_team: offer.assets_by_team.clone(),
        round: offer.round.saturating_add(1),
        parent: Some(offer.id),
    }
}

/// CBA-fallback: if the generated counter fails validation, try once more by
/// adding cash from the *initiator* (up to the season cash limit).
///
/// We add cash to the initiator side because the receiving GM is the one
/// asking for more value — pulling cash from the initiator is the cleanest
/// way to satisfy a salary-matching shortfall without re-shuffling assets.
fn cash_add_fallback(
    counter: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
) -> Option<TradeOffer> {
    let initiator = counter.initiator;
    if initiator == evaluator {
        return None;
    }
    let max = league.league_year.max_trade_cash;
    let current = counter
        .assets_by_team
        .get(&initiator)
        .map(|a| a.cash_out)
        .unwrap_or(Cents::ZERO);
    if current >= max {
        return None;
    }

    let mut next = counter.clone();
    let entry = next
        .assets_by_team
        .entry(initiator)
        .or_insert_with(TradeAssets::default);
    entry.cash_out = max;
    Some(next)
}

/// Public for tests: pick a move kind under a given archetype + RNG.
/// Used by the personality-distribution tests so they don't need to drive a
/// full `step` to assert "Conservative never picks Subtract".
#[doc(hidden)]
pub fn _pick_move_for_test(archetype: GMArchetype, rng: &mut dyn RngCore) -> &'static str {
    match pick_move(archetype, rng) {
        Move::Add => "add",
        Move::Swap => "swap",
        Move::Subtract => "subtract",
    }
}
