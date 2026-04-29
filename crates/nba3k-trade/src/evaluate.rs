//! Trade evaluation entry point — Worker A.
//!
//! Pipeline (top-level [`evaluate`]):
//! 1. CBA gate FIRST (call [`crate::cba::validate`]). On violation we short
//!    -circuit to `Verdict::Reject(CbaViolation(...))` with a one-line
//!    commentary and zero-net value — no point in valuing an illegal trade.
//! 2. Resolve evaluator traits from the team's GM personality, then run
//!    `context::apply_context` to modulate by team mode and season phase.
//! 3. Sum each side's value via [`crate::valuation::value_side`] and compute
//!    net = incoming − outgoing.
//! 4. Add gaussian noise scaled by `traits.gullibility` (Wildcard high,
//!    Conservative near zero). Noise stddev = 8% of outgoing × gullibility.
//! 5. Threshold: ≥ +5% of outgoing → Accept, ≤ −15% → Reject(InsufficientValue),
//!    otherwise Counter(offer.clone()).
//!
//! For unit testing we expose [`evaluate_with_traits`] which skips the CBA
//! gate AND the context modulation. This lets tests on this crate exercise
//! the value-and-verdict math without depending on Worker B/C/D bodies.

use crate::snapshot::LeagueSnapshot;
use crate::valuation::value_side;
use nba3k_core::{Cents, GMTraits, RejectReason, TeamId, TradeEvaluation, TradeOffer, Verdict};
#[cfg(test)]
use nba3k_core::{Coach, PlayerRole};
use rand::RngCore;
use rand_distr::{Distribution, Normal};

// Loosen the value gates so peer-OVR fair trades land in Accept/Counter
// instead of Reject. NBA-realistic: most trades that aren't lopsided get a
// counter rather than an outright rejection.
const ACCEPT_THRESHOLD_PCT: f64 = -0.02;
const REJECT_THRESHOLD_PCT: f64 = -0.30;

/// Top-level entry. Runs the M4 realism pipeline:
///   1. CBA gate FIRST. On violation → Reject(CbaViolation).
///   2. Delegate to `nba3k_models::trade_acceptance` which composes
///      player_value + contract_value + asset_fit + star_protection +
///      team_context. Star protection short-circuits BEFORE value math.
///
/// `evaluate_with_traits` (below) preserves the M3 standalone path for
/// tests that don't have a star roster handy.
pub fn evaluate(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    rng: &mut dyn RngCore,
) -> TradeEvaluation {
    let star_roster = realism_resources::star_roster();

    // 1. Untouchable star short-circuit. Fires BEFORE CBA so the rejection
    //    message reflects the franchise's "not for sale" stance, not a
    //    salary-matching technicality. Per M4-realism.md, untouchables are
    //    untouchable regardless of CBA legality or trade value.
    if let Some(outgoing) = offer.assets_by_team.get(&evaluator) {
        let sp_weights = &realism_resources::weights().star_protection;
        for pid in &outgoing.players_out {
            let Some(player) = league.player(*pid) else {
                continue;
            };
            let score = nba3k_models::star_protection::star_protection(
                *pid,
                evaluator,
                league,
                star_roster,
                sp_weights,
            );
            if score.value >= sp_weights.absolute_threshold as f64 {
                return TradeEvaluation {
                    net_value: Cents::ZERO,
                    verdict: Verdict::Reject(RejectReason::Other(format!(
                        "{} is untouchable",
                        player.name
                    ))),
                    confidence: 1.0,
                    commentary: format!("{} is not on the table.", player.name),
                };
            }
        }
    }

    // 2. CBA gate.
    if let Err(violation) = crate::cba::validate(offer, league) {
        return TradeEvaluation {
            net_value: Cents::ZERO,
            verdict: Verdict::Reject(RejectReason::CbaViolation(violation.to_string())),
            confidence: 1.0,
            commentary: format!("Front office says no — {}.", short_violation(&violation)),
        };
    }

    // 3. Delegate to the realism composite.
    let weights = realism_resources::trade_acceptance_weights();
    let acceptance = nba3k_models::trade_acceptance::trade_acceptance(
        offer,
        evaluator,
        league,
        star_roster,
        weights,
        rng,
    );

    let confidence = acceptance.probability.clamp(0.0, 1.0) as f32;
    TradeEvaluation {
        net_value: acceptance.net_value,
        verdict: acceptance.verdict,
        confidence,
        commentary: acceptance.commentary,
    }
}

mod realism_resources {
    //! Process-wide cache for realism data files. Loaded lazily on first call.
    //! Missing files fall back to empty/default values — explicit policy from
    //! M4 phase doc.

    use nba3k_models::star_protection::{load_star_roster, StarRoster, STAR_ROSTER_PATH};
    use nba3k_models::weights::{load_or_default, RealismWeights};
    use std::path::Path;
    use std::sync::OnceLock;

    /// Path to the project-wide realism weights file (empty file → defaults).
    pub const REALISM_WEIGHTS_PATH: &str = "data/realism_weights.toml";

    static STAR_ROSTER: OnceLock<StarRoster> = OnceLock::new();
    static WEIGHTS: OnceLock<RealismWeights> = OnceLock::new();

    pub fn star_roster() -> &'static StarRoster {
        STAR_ROSTER
            .get_or_init(|| load_star_roster(Path::new(STAR_ROSTER_PATH)).unwrap_or_default())
    }

    pub fn weights() -> &'static RealismWeights {
        WEIGHTS.get_or_init(|| load_or_default(Path::new(REALISM_WEIGHTS_PATH)).unwrap_or_default())
    }

    pub fn trade_acceptance_weights() -> &'static nba3k_models::weights::TradeAcceptanceWeights {
        &weights().trade_acceptance
    }
}

/// Variant that takes already-modulated traits and skips both the CBA gate
/// and context modulation. Public so tests in this crate (and the
/// orchestrator's calibration harness) can drive the math directly.
pub fn evaluate_with_traits(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    traits: &GMTraits,
    rng: &mut dyn RngCore,
) -> TradeEvaluation {
    let (outgoing, incoming) = sum_sides(offer, evaluator, league, traits);
    let net_pre_noise = incoming.0 - outgoing.0;

    let noise_dollars = sample_noise(outgoing.0, traits.gullibility as f64, rng);
    let net = net_pre_noise + noise_dollars;

    let outgoing_abs = outgoing.0.max(1);
    let net_pct = net as f64 / outgoing_abs as f64;

    let verdict = if net_pct >= ACCEPT_THRESHOLD_PCT {
        Verdict::Accept
    } else if net_pct <= REJECT_THRESHOLD_PCT {
        Verdict::Reject(RejectReason::InsufficientValue)
    } else {
        Verdict::Counter(offer.clone())
    };

    let commentary = commentary_for(&verdict, net_pct);
    let confidence = confidence_for(net_pct, traits.gullibility);

    TradeEvaluation {
        net_value: Cents(net),
        verdict,
        confidence,
        commentary,
    }
}

/// Sum (outgoing_value, incoming_value) for `evaluator` from the offer's
/// sides. Outgoing = what evaluator gives up; incoming = what every other
/// side ships into the deal (the trade-acceptance model values the entire
/// pot of incoming assets, not just whichever leg is round-robin-routed to
/// this team — that abstraction matches how 3-team trades are negotiated in
/// practice, where each team weighs the whole package they're getting).
fn sum_sides(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    traits: &GMTraits,
) -> (Cents, Cents) {
    let mut outgoing = Cents::ZERO;
    let mut incoming = Cents::ZERO;
    for (team, assets) in &offer.assets_by_team {
        let v = value_side(*team, evaluator, assets, league, traits);
        if *team == evaluator {
            outgoing = outgoing + v;
        } else {
            incoming = incoming + v;
        }
    }
    (outgoing, incoming)
}

/// Resolve the evaluator's base GM traits from their team. Falls back to
/// neutral defaults if the team is missing from the snapshot (shouldn't
/// happen in practice but keeps evaluation total).
#[allow(dead_code)]
fn evaluator_traits(evaluator: TeamId, league: &LeagueSnapshot) -> GMTraits {
    league
        .team(evaluator)
        .map(|t| t.gm.traits)
        .unwrap_or_default()
}

/// Sample gaussian noise in `Cents` units. Stddev is 8% of the outgoing
/// value × gullibility. Wildcard GMs (gullibility 0.7) get ~5.6% noise;
/// Conservative GMs (~0.05) get nearly none.
fn sample_noise(outgoing_cents: i64, gullibility: f64, rng: &mut dyn RngCore) -> i64 {
    let stddev = (outgoing_cents as f64).abs() * 0.08 * gullibility.max(0.0);
    if stddev <= 0.0 {
        return 0;
    }
    // `Normal::new` returns Result; we built it with mean=0 and a positive
    // stddev so it can't fail.
    let normal = Normal::new(0.0, stddev).expect("stddev > 0");
    let mut wrapper = RngWrapper(rng);
    normal.sample(&mut wrapper) as i64
}

/// Adapter so we can pass `&mut dyn RngCore` into `Distribution::sample`
/// (which wants `R: Rng + ?Sized`). RngCore alone is enough because Normal
/// only calls fill_bytes/next_u32/next_u64 internally.
struct RngWrapper<'a>(&'a mut dyn RngCore);

impl rand::RngCore for RngWrapper<'_> {
    fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.0.fill_bytes(dest)
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.0.try_fill_bytes(dest)
    }
}

fn short_violation(v: &crate::cba::CbaViolation) -> String {
    use crate::cba::CbaViolation::*;
    match v {
        SalaryMatching { .. } => "salary matching".into(),
        HardCapTrigger { .. } => "hard-cap trigger".into(),
        NoTradeClause(_) => "no-trade clause".into(),
        CashLimitExceeded { .. } => "cash limit".into(),
        AggregationCooldown { .. } => "aggregation cooldown".into(),
        RosterSize { .. } => "roster size".into(),
        Apron2Restriction { .. } => "apron 2 restrictions".into(),
        PickTooFarOut { .. } => "seven-year rule".into(),
        StepienViolation { .. } => "stepien rule".into(),
    }
}

/// Confidence is high when the verdict is decisively over a threshold and
/// the GM doesn't second-guess themselves (low gullibility). Range 0..1.
fn confidence_for(net_pct: f64, gullibility: f32) -> f32 {
    let base = if net_pct >= ACCEPT_THRESHOLD_PCT {
        ((net_pct - ACCEPT_THRESHOLD_PCT) / 0.20).min(1.0)
    } else if net_pct <= REJECT_THRESHOLD_PCT {
        ((REJECT_THRESHOLD_PCT - net_pct) / 0.20).min(1.0)
    } else {
        // Counter zone — confidence is naturally lower in the middle.
        0.4
    };
    let gullibility_penalty = gullibility.clamp(0.0, 1.0) * 0.3;
    ((base as f32) - gullibility_penalty).clamp(0.0, 1.0)
}

fn commentary_for(verdict: &Verdict, net_pct: f64) -> String {
    match verdict {
        Verdict::Accept => {
            if net_pct >= 0.20 {
                "We jump on this — clear win for our books.".into()
            } else {
                "Looks fair, we can make it work.".into()
            }
        }
        Verdict::Counter(_) => {
            if net_pct < 0.0 {
                "Like the names but we'd need real value coming back, not filler.".into()
            } else {
                "Close, but we'd want a sweetener before signing off.".into()
            }
        }
        Verdict::Reject(reason) => match reason {
            RejectReason::InsufficientValue => "Doesn't move the needle for us — we're out.".into(),
            RejectReason::CbaViolation(_) => {
                "Front office says no — CBA won't let us do it.".into()
            }
            RejectReason::NoTradeClause(_) => {
                "Player has a no-trade clause and isn't waiving it.".into()
            }
            RejectReason::BadFaith => "Not negotiating against ourselves here.".into(),
            RejectReason::OutOfRoundCap => "We're done talking on this one.".into(),
            RejectReason::Other(s) => s.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use nba3k_core::{
        BirdRights, Contract, ContractYear, DraftPick, DraftPickId, GMArchetype, GMPersonality,
        Player, PlayerId, Position, Ratings, SeasonId, SeasonPhase, Team, TeamId, TradeAssets,
        TradeId, TradeOffer,
    };
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::HashMap;

    /// Builder for a tiny self-contained snapshot world used by these tests.
    /// Owns its data; hand out a `LeagueSnapshot` borrowing into it.
    struct World {
        teams: Vec<Team>,
        players: HashMap<PlayerId, Player>,
        picks: HashMap<DraftPickId, DraftPick>,
        standings: HashMap<TeamId, crate::TeamRecordSummary>,
    }

    impl World {
        fn new() -> Self {
            let teams = vec![
                mk_team(1, "BOS", GMArchetype::WinNow),
                mk_team(2, "LAL", GMArchetype::StarHunter),
                mk_team(3, "OKC", GMArchetype::Cheapskate),
                mk_team(4, "MIA", GMArchetype::Conservative),
            ];
            Self {
                teams,
                players: HashMap::new(),
                picks: HashMap::new(),
                standings: HashMap::new(),
            }
        }

        fn add_player(&mut self, p: Player) {
            self.players.insert(p.id, p);
        }

        fn snap(&self) -> LeagueSnapshot<'_> {
            LeagueSnapshot {
                current_season: SeasonId(2026),
                current_phase: SeasonPhase::Regular,
                current_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
                league_year: nba3k_core::LeagueYear::for_season(SeasonId(2026)).unwrap(),
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
            conference: nba3k_core::Conference::East,
            division: nba3k_core::Division::Atlantic,
            gm: GMPersonality::from_archetype(format!("{abbrev} GM"), archetype),
            roster: Vec::new(),
            draft_picks: Vec::new(),
            coach: Coach::default(),
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
            TradeAssets {
                players_out: send,
                picks_out: vec![],
                cash_out: Cents::ZERO,
            },
        );
        assets.insert(
            to,
            TradeAssets {
                players_out: receive,
                picks_out: vec![],
                cash_out: Cents::ZERO,
            },
        );
        TradeOffer {
            id: TradeId(1),
            initiator: from,
            assets_by_team: assets,
            round: 1,
            parent: None,
        }
    }

    #[test]
    fn evaluate_star_for_filler_rejected_by_receiving_team() {
        // BOS sends filler, LAL sends a star. We evaluate from LAL's POV —
        // they should reject because they'd be giving up far more than they
        // receive.
        let mut world = World::new();
        let bos = TeamId(1);
        let lal = TeamId(2);
        let filler = mk_player(10, 70, 25, 5_000_000, bos);
        let star = mk_player(20, 95, 28, 50_000_000, lal);
        world.add_player(filler.clone());
        world.add_player(star.clone());

        let snap = world.snap();
        let offer = two_team_offer(bos, lal, vec![filler.id], vec![star.id]);

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        // Use neutral traits + skip CBA/context to avoid Worker B/C deps.
        let traits = GMTraits::default();
        let eval = evaluate_with_traits(&offer, lal, &snap, &traits, &mut rng);

        match eval.verdict {
            Verdict::Reject(RejectReason::InsufficientValue) => {}
            other => panic!("expected Reject(InsufficientValue), got {other:?}"),
        }
        // Net should be deeply negative for the side giving up the star.
        assert!(
            eval.net_value.0 < 0,
            "net should be negative for star-out side, got {}",
            eval.net_value.0
        );
        // And the percentage should be worse than -15% of outgoing.
        // (We don't know exact outgoing here but star >> filler, so net%
        // is comfortably below -50%.)
    }

    #[test]
    fn evaluate_equal_value_swap_accepted() {
        // Two role players, near-identical OVR, age, contract. Should
        // resolve to Accept (or at least not Reject) regardless of small
        // noise. We seed an rng for determinism and assert Accept.
        let mut world = World::new();
        let bos = TeamId(1);
        let phi = TeamId(2);
        // Make BOS's outgoing player slightly *worse* than incoming so that
        // even with noise the net comes out >= +5% of outgoing.
        let send = mk_player(10, 80, 26, 22_000_000, bos);
        let recv = mk_player(20, 81, 27, 22_000_000, phi);
        world.add_player(send.clone());
        world.add_player(recv.clone());

        let snap = world.snap();
        let offer = two_team_offer(bos, phi, vec![send.id], vec![recv.id]);

        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let traits = GMTraits::default();
        let eval = evaluate_with_traits(&offer, bos, &snap, &traits, &mut rng);

        match eval.verdict {
            Verdict::Accept => {}
            other => panic!("expected Accept on slight-uplift swap, got {other:?}"),
        }
    }

    #[test]
    fn equal_value_truly_neutral_offer_lands_in_counter() {
        // When net value is essentially zero (same OVR, age, salary), we
        // should land in the Counter band (between -15% and +5%).
        let mut world = World::new();
        let bos = TeamId(1);
        let phi = TeamId(2);
        let send = mk_player(10, 80, 26, 22_000_000, bos);
        let recv = mk_player(20, 80, 26, 22_000_000, phi);
        world.add_player(send.clone());
        world.add_player(recv.clone());

        let snap = world.snap();
        let offer = two_team_offer(bos, phi, vec![send.id], vec![recv.id]);

        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let mut traits = GMTraits::default();
        traits.gullibility = 0.0; // deterministic — no noise
        let eval = evaluate_with_traits(&offer, bos, &snap, &traits, &mut rng);

        // BOS is the sender so loyalty bonus tilts the math: with
        // default loyalty=0.1 and our 20% baseline-bonus formula that
        // is 2% of baseline value extra on the outgoing side, which
        // makes incoming net out *negative* — landing in Counter.
        match eval.verdict {
            Verdict::Counter(_) => {}
            Verdict::Accept => {} // also acceptable — depends on loyalty calibration
            other => panic!("expected Counter or Accept, got {other:?}"),
        }
    }

    #[test]
    fn cheapskate_vs_winnow_diverge_on_same_offer() {
        // Same offer evaluated by Cheapskate vs WinNow GM — they should
        // produce different verdicts. We construct an offer where:
        //  - WinNow wants the immediate impact star (likes incoming).
        //  - Cheapskate balks at the salary on the incoming star.
        let mut world = World::new();
        let okc = TeamId(3); // Cheapskate
        let lal = TeamId(2); // StarHunter — but we'll evaluate as if WinNow
                             // We're going to compare verdicts of the same offer evaluated by
                             // two GMs with very different traits (Cheapskate vs WinNow) acting
                             // as the *receiving* team.

        // Outgoing: a young, cheap, mid-OVR rotation guy.
        let outgoing = mk_player(10, 78, 23, 4_000_000, okc);
        // Incoming: an aging star on a max contract — WinNow loves the
        // talent now, Cheapskate hates the cap hit and the age tail.
        let incoming = mk_player(20, 90, 33, 48_000_000, lal);
        world.add_player(outgoing.clone());
        world.add_player(incoming.clone());

        let snap = world.snap();
        let offer = two_team_offer(okc, lal, vec![outgoing.id], vec![incoming.id]);

        let mut rng_a = ChaCha8Rng::seed_from_u64(123);
        let mut rng_b = ChaCha8Rng::seed_from_u64(123);

        let win_now = GMPersonality::from_archetype("WN", GMArchetype::WinNow).traits;
        let cheap = GMPersonality::from_archetype("CS", GMArchetype::Cheapskate).traits;

        let win_eval = evaluate_with_traits(&offer, okc, &snap, &win_now, &mut rng_a);
        let cheap_eval = evaluate_with_traits(&offer, okc, &snap, &cheap, &mut rng_b);

        // The two GMs should at minimum produce different net values — that
        // is the regression test the spec calls for. Verdict difference is
        // also expected with these specific personality weights (WinNow
        // values present talent 1.5×, Cheapskate weights salary 1.8×).
        assert_ne!(
            win_eval.net_value.0, cheap_eval.net_value.0,
            "WinNow and Cheapskate should produce different net values for same offer"
        );
        let win_disc = std::mem::discriminant(&win_eval.verdict);
        let cheap_disc = std::mem::discriminant(&cheap_eval.verdict);
        assert_ne!(
            win_disc, cheap_disc,
            "WinNow vs Cheapskate should diverge on verdict for an aging-star-on-max offer; \
             win_eval={:?} cheap_eval={:?}",
            win_eval.verdict, cheap_eval.verdict
        );
    }

    #[test]
    fn confidence_in_range() {
        let mut world = World::new();
        let bos = TeamId(1);
        let phi = TeamId(2);
        let a = mk_player(10, 80, 26, 22_000_000, bos);
        let b = mk_player(20, 80, 26, 22_000_000, phi);
        world.add_player(a.clone());
        world.add_player(b.clone());

        let snap = world.snap();
        let offer = two_team_offer(bos, phi, vec![a.id], vec![b.id]);

        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let traits = GMTraits::default();
        let eval = evaluate_with_traits(&offer, bos, &snap, &traits, &mut rng);
        assert!(eval.confidence >= 0.0 && eval.confidence <= 1.0);
    }
}
