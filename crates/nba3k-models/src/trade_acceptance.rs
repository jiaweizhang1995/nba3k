//! Worker D — trade acceptance composite.
//!
//! Composes player_value + contract_value + asset_fit + star_protection
//! + team_context into a single TradeAcceptance with probability,
//! verdict, net value, top-K reasons, and commentary.
//!
//! Star protection short-circuits BEFORE any value math: outgoing
//! protection ≥ `weights.star_protection.absolute_threshold` (0.85
//! default) → Reject("untouchable") regardless of net.
//!
//! Verdict is sampled from a logistic on `net_pct`:
//!   p = sigmoid(intercept + slope · net_pct + gaussian_noise)
//! with `gullibility`-scaled noise. Thresholds (default):
//!   p ≥ 0.55 → Accept
//!   p ≤ 0.20 → Reject(InsufficientValue)
//!   else      → Counter(offer.clone())
//!
//! See `phases/M4-realism.md` "Worker D" for the full spec.
//!
//! ## Mocking workers A/B during dev
//!
//! Workers A and B's function bodies are `todo!()` while M4 is in
//! flight, but their signatures are LOCKED. The public
//! [`trade_acceptance`] entry calls those real functions. For unit
//! tests that exercise the composition logic without depending on
//! A/B bodies, use [`trade_acceptance_with_providers`] which takes
//! closures for the per-player value lookups. Once A+B land, the
//! workspace integration suite runs the public entry end-to-end.

use crate::asset_fit::asset_fit;
use crate::star_protection::{star_protection, StarRoster};
use crate::team_context::{team_context, TeamContext, TeamMode};
use crate::weights::{
    AssetFitWeights, ContractValueWeights, PlayerValueWeights, StarProtectionWeights,
    TeamContextWeights, TradeAcceptanceWeights,
};
use crate::{Reason, Score};
use nba3k_core::{
    Cents, GMTraits, LeagueSnapshot, LeagueYear, Player, PlayerId, RejectReason, TeamId,
    TradeAssets, TradeOffer, Verdict,
};
use rand::RngCore;
use rand_distr::{Distribution, Normal};

#[derive(Debug, Clone)]
pub struct TradeAcceptance {
    pub probability: f64,
    pub verdict: Verdict,
    pub net_value: Cents,
    pub reasons: Vec<Reason>,
    pub commentary: String,
}

/// Convenience: collapse a Score's value into the canonical Cents.
#[allow(dead_code)]
pub(crate) fn score_cents(s: &Score) -> Cents {
    Cents(s.value as i64)
}

// ---------------------------------------------------------------------------
// Provider plumbing — closure-based so tests can mock workers A/B without
// requiring their bodies to be implemented. The public entry point wires
// up the real model functions.
// ---------------------------------------------------------------------------

/// All weights bundle that the composite needs from the wider model
/// registry. Keeps the public signature small.
#[derive(Debug, Clone)]
pub struct ComposeWeights<'w> {
    pub player_value: &'w PlayerValueWeights,
    pub contract_value: &'w ContractValueWeights,
    pub asset_fit: &'w AssetFitWeights,
    pub star_protection: &'w StarProtectionWeights,
    pub team_context: &'w TeamContextWeights,
    pub trade_acceptance: &'w TradeAcceptanceWeights,
}

/// Closures that wrap A/B/D model functions. Tests inject deterministic
/// stubs; production wiring inserts the real `crate::*::*` calls.
pub struct ValueProviders<'a> {
    pub player_value:
        Box<dyn Fn(&Player, &GMTraits, TeamId, &LeagueSnapshot) -> Score + 'a>,
    pub contract_value:
        Box<dyn Fn(&Player, &GMTraits, &LeagueYear) -> Score + 'a>,
    pub star_protection:
        Box<dyn Fn(PlayerId, TeamId, &LeagueSnapshot, &StarRoster) -> Score + 'a>,
    pub team_context:
        Box<dyn Fn(TeamId, &LeagueSnapshot) -> TeamContext + 'a>,
    pub asset_fit:
        Box<dyn Fn(&Player, TeamId, &LeagueSnapshot) -> Score + 'a>,
}

impl<'a> ValueProviders<'a> {
    /// Production wiring — calls real model functions with full weights.
    /// `weights` is captured by reference into each closure so the wider
    /// caller still owns it.
    pub fn real(weights: &'a ComposeWeights<'a>) -> Self {
        let pv_w = weights.player_value;
        let cv_w = weights.contract_value;
        let sp_w = weights.star_protection;
        let tc_w = weights.team_context;
        let af_w = weights.asset_fit;
        Self {
            player_value: Box::new(move |p, t, ev, l| {
                crate::player_value::player_value(p, t, ev, l, pv_w)
            }),
            contract_value: Box::new(move |p, t, ly| {
                crate::contract_value::contract_value(p, p.contract.as_ref(), t, ly, cv_w)
            }),
            star_protection: Box::new(move |pid, owner, l, roster| {
                star_protection(pid, owner, l, roster, sp_w)
            }),
            team_context: Box::new(move |t, l| team_context(t, l, tc_w)),
            asset_fit: Box::new(move |p, t, l| asset_fit(p, t, l, af_w)),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry — wires real model functions, then delegates to the
// provider-based composer.
// ---------------------------------------------------------------------------

pub fn trade_acceptance(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    star_roster: &StarRoster,
    weights: &TradeAcceptanceWeights,
    rng: &mut dyn RngCore,
) -> TradeAcceptance {
    // Use defaults for the sub-model weights when callers only have the
    // top-level TradeAcceptanceWeights. The orchestrator's production
    // wiring should call `trade_acceptance_with_providers` directly with
    // the full bundle from `RealismWeights`.
    let pv = PlayerValueWeights::default();
    let cv = ContractValueWeights::default();
    let af = AssetFitWeights::default();
    let sp = StarProtectionWeights::default();
    let tc = TeamContextWeights::default();
    let bundle = ComposeWeights {
        player_value: &pv,
        contract_value: &cv,
        asset_fit: &af,
        star_protection: &sp,
        team_context: &tc,
        trade_acceptance: weights,
    };
    let providers = ValueProviders::real(&bundle);
    trade_acceptance_with_providers(offer, evaluator, league, star_roster, &bundle, &providers, rng)
}

/// Composition entry that takes injected providers. Callers (production
/// wiring) typically use [`trade_acceptance`]; tests use this with stub
/// closures so they don't depend on Worker A/B function bodies.
pub fn trade_acceptance_with_providers(
    offer: &TradeOffer,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    star_roster: &StarRoster,
    weights: &ComposeWeights<'_>,
    providers: &ValueProviders<'_>,
    rng: &mut dyn RngCore,
) -> TradeAcceptance {
    let traits = evaluator_traits(evaluator, league);

    // ---- 1. Star-protection short-circuit ---------------------------
    // Iterate evaluator's outgoing players and bail before any value math
    // if any one of them is flagged untouchable. This is the user's
    // headline behavior: Luka cannot be acquired, period.
    if let Some(outgoing_assets) = offer.assets_by_team.get(&evaluator) {
        for pid in &outgoing_assets.players_out {
            let Some(player) = league.player(*pid) else { continue };
            let sp_score = (providers.star_protection)(*pid, evaluator, league, star_roster);
            if sp_score.value >= weights.star_protection.absolute_threshold as f64 {
                return short_circuit_untouchable(player, &sp_score);
            }
        }
    }

    // ---- 2. Sum sides ----------------------------------------------
    let league_year = league.league_year;
    let mut all_reasons: Vec<Reason> = Vec::new();
    let mut sum_out_cents: f64 = 0.0;
    let mut sum_in_cents: f64 = 0.0;

    for (team, assets) in &offer.assets_by_team {
        if *team == evaluator {
            sum_out_cents += sum_outgoing_side(
                assets,
                evaluator,
                league,
                &league_year,
                &traits,
                star_roster,
                weights,
                providers,
                &mut all_reasons,
            );
        } else {
            sum_in_cents += sum_incoming_side(
                assets,
                evaluator,
                league,
                &league_year,
                &traits,
                providers,
                &mut all_reasons,
            );
        }
    }

    let net_pre_noise = sum_in_cents - sum_out_cents;
    let outgoing_abs = sum_out_cents.abs().max(1.0);

    // ---- 3. Team-context modifier ----------------------------------
    let context = (providers.team_context)(evaluator, league);
    let context_delta = team_context_value_delta(&context, sum_in_cents, sum_out_cents);
    if context_delta.abs() > 1.0 {
        all_reasons.push(Reason {
            label: context_label(&context.mode),
            delta: context_delta,
        });
    }
    let net_pre_noise = net_pre_noise + context_delta;

    // ---- 4. Logistic + noise ---------------------------------------
    let net_pct = net_pre_noise / outgoing_abs;
    let noise = sample_noise(traits.gullibility as f64, weights, rng);
    let logit = weights.trade_acceptance.accept_probability_intercept
        + weights.trade_acceptance.accept_probability_slope * net_pct
        + noise;
    let probability = sigmoid(logit);

    // ---- 5. Verdict -------------------------------------------------
    let verdict = if probability >= ACCEPT_PROBABILITY {
        Verdict::Accept
    } else if probability <= REJECT_PROBABILITY {
        Verdict::Reject(RejectReason::InsufficientValue)
    } else {
        Verdict::Counter(offer.clone())
    };

    // ---- 6. Top-K reasons + commentary -----------------------------
    let top_k = weights.trade_acceptance.top_k_reasons.max(1);
    let mut reasons = all_reasons;
    reasons.sort_by(|a, b| {
        b.delta
            .abs()
            .partial_cmp(&a.delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if reasons.len() > top_k {
        reasons.truncate(top_k);
    }
    let commentary = commentary_for(&verdict, reasons.first());

    TradeAcceptance {
        probability,
        verdict,
        net_value: Cents(net_pre_noise as i64),
        reasons,
        commentary,
    }
}

// ---------------------------------------------------------------------------
// Verdict thresholds — sampled from the logistic.
// ---------------------------------------------------------------------------

/// p ≥ this → Accept. Tuned in 2026-04 calibration.
pub const ACCEPT_PROBABILITY: f64 = 0.55;
/// p ≤ this → Reject. Tuned in 2026-04 calibration.
pub const REJECT_PROBABILITY: f64 = 0.20;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn evaluator_traits(evaluator: TeamId, league: &LeagueSnapshot) -> GMTraits {
    league
        .team(evaluator)
        .map(|t| t.gm.traits)
        .unwrap_or_default()
}

fn short_circuit_untouchable(player: &Player, sp_score: &Score) -> TradeAcceptance {
    let mut reasons = vec![Reason { label: "untouchable star", delta: -1.0 }];
    // Forward star_protection's component reasons so callers can render
    // *why* the player is untouchable (franchise tag, top-OVR, etc).
    reasons.extend(sp_score.reasons.iter().copied());
    TradeAcceptance {
        probability: 0.0,
        verdict: Verdict::Reject(RejectReason::Other("untouchable".into())),
        net_value: Cents::ZERO,
        reasons,
        commentary: format!("{} is not on the table.", player.name),
    }
}

#[allow(clippy::too_many_arguments)]
fn sum_outgoing_side(
    assets: &TradeAssets,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    league_year: &LeagueYear,
    traits: &GMTraits,
    star_roster: &StarRoster,
    weights: &ComposeWeights<'_>,
    providers: &ValueProviders<'_>,
    out_reasons: &mut Vec<Reason>,
) -> f64 {
    let mut total: f64 = 0.0;
    for pid in &assets.players_out {
        let Some(player) = league.player(*pid) else { continue };
        let pv = (providers.player_value)(player, traits, evaluator, league);
        let cv = (providers.contract_value)(player, traits, league_year);
        // contract_value > 0 = team-friendly: losing it costs us. So the
        // outgoing side adds (player_value − contract_value): a friendly
        // contract makes the loss bigger, an overpaid one shrinks it.
        let line = pv.value - cv.value;
        total += line;
        // Premium-zone star protection (between premium and absolute
        // thresholds) imposes a soft penalty so the model still rejects
        // close-to-untouchable trades.
        let sp = (providers.star_protection)(*pid, evaluator, league, star_roster);
        if sp.value >= weights.star_protection.premium_threshold as f64 {
            // Linear ramp from 0× at premium_threshold to ~+25% of pv at
            // absolute_threshold. We add this to the outgoing total so the
            // side feels heavier.
            let span =
                (weights.star_protection.absolute_threshold
                    - weights.star_protection.premium_threshold)
                    .max(0.001) as f64;
            let frac = ((sp.value - weights.star_protection.premium_threshold as f64) / span)
                .clamp(0.0, 1.0);
            let penalty = pv.value.abs() * 0.25 * frac;
            total += penalty;
            out_reasons.push(Reason {
                label: "outgoing star premium",
                delta: -penalty,
            });
        }

        out_reasons.push(Reason { label: "outgoing player_value", delta: -pv.value });
        out_reasons.extend(pv.reasons.into_iter());
        if cv.value.abs() > 1.0 {
            out_reasons.push(Reason {
                label: "outgoing contract_value",
                delta: cv.value, // positive contract = positive loss → reason is positive number meaning "we gave up surplus"
            });
        }
    }
    for pickid in &assets.picks_out {
        if let Some(pk) = league.pick(*pickid) {
            let pv = pick_value_cents(pk, league.current_season, traits, league);
            total += pv;
            out_reasons.push(Reason { label: "outgoing pick", delta: -pv });
        }
    }
    total += assets.cash_out.0 as f64;
    if assets.cash_out.0 != 0 {
        out_reasons.push(Reason { label: "outgoing cash", delta: -(assets.cash_out.0 as f64) });
    }
    total
}

fn sum_incoming_side(
    assets: &TradeAssets,
    evaluator: TeamId,
    league: &LeagueSnapshot,
    league_year: &LeagueYear,
    traits: &GMTraits,
    providers: &ValueProviders<'_>,
    out_reasons: &mut Vec<Reason>,
) -> f64 {
    let mut total: f64 = 0.0;
    for pid in &assets.players_out {
        let Some(player) = league.player(*pid) else { continue };
        let pv = (providers.player_value)(player, traits, evaluator, league);
        let af = (providers.asset_fit)(player, evaluator, league);
        let cv = (providers.contract_value)(player, traits, league_year);
        let line = pv.value + af.value + cv.value;
        total += line;

        out_reasons.push(Reason { label: "incoming player_value", delta: pv.value });
        if af.value.abs() > 1.0 {
            out_reasons.push(Reason { label: "asset fit", delta: af.value });
            // Forward the most descriptive sub-reason from asset_fit.
            if let Some(top) = af.reasons.first() {
                out_reasons.push(*top);
            }
        }
        if cv.value.abs() > 1.0 {
            out_reasons.push(Reason { label: "incoming contract_value", delta: cv.value });
        }
    }
    for pickid in &assets.picks_out {
        if let Some(pk) = league.pick(*pickid) {
            let pv = pick_value_cents(pk, league.current_season, traits, league);
            total += pv;
            out_reasons.push(Reason { label: "incoming pick", delta: pv });
        }
    }
    total += assets.cash_out.0 as f64;
    if assets.cash_out.0 != 0 {
        out_reasons.push(Reason { label: "incoming cash", delta: assets.cash_out.0 as f64 });
    }
    total
}

/// Pick valuation — local copy of the curve from `nba3k_trade::valuation`
/// so this crate doesn't need a back-edge into `nba3k-trade`. The shape
/// must mirror that file (tuned by the calibration harness).
fn pick_value_cents(
    pick: &nba3k_core::DraftPick,
    current_season: nba3k_core::SeasonId,
    traits: &GMTraits,
    league: &LeagueSnapshot,
) -> f64 {
    let slot = if pick.season == current_season {
        project_pick_slot(pick, league)
    } else {
        15
    };
    let raw = pick_slot_dollars(pick.round, slot) as f64;

    let years_out = (pick.season.0 as i32 - current_season.0 as i32).max(0);
    let discount = if years_out <= 1 {
        1.0
    } else {
        0.90_f64.powi(years_out - 1)
    };
    let mult = traits.pick_value_multiplier.max(0.0) as f64;
    raw * discount * mult * 100.0 // dollars → cents
}

fn pick_slot_dollars(round: u8, projected_slot: u8) -> i64 {
    if round >= 2 {
        return 500_000;
    }
    let s = projected_slot.clamp(1, 30) as f64;
    let v = 90.0 * (1.0 - (s - 1.0) / 29.0).powf(1.4) + 4.0;
    (v * 1_000_000.0) as i64
}

fn project_pick_slot(pick: &nba3k_core::DraftPick, league: &LeagueSnapshot) -> u8 {
    let owner = pick.original_team;
    let record = league.record(owner);
    if record.games_played() == 0 {
        return 15;
    }
    let pct = record.win_pct().clamp(0.0, 1.0);
    let slot = 1.0 + 29.0 * pct as f64;
    slot.round().clamp(1.0, 30.0) as u8
}

/// Translate team context into a $-valued tilt of the offer. Contend
/// teams need a higher delivery (negative tilt = effectively raises the
/// bar). FullRebuild accepts more upside-for-stars so positive tilt.
fn team_context_value_delta(ctx: &TeamContext, sum_in: f64, sum_out: f64) -> f64 {
    let scale = sum_in.abs().max(sum_out.abs()).max(1.0);
    match ctx.mode {
        TeamMode::Contend => -0.05 * scale,
        TeamMode::Retool => -0.02 * scale,
        TeamMode::SoftRebuild => 0.02 * scale,
        TeamMode::FullRebuild => 0.05 * scale,
        TeamMode::Tank => 0.03 * scale,
    }
}

fn context_label(mode: &TeamMode) -> &'static str {
    match mode {
        TeamMode::Contend => "context: contend (raises bar)",
        TeamMode::Retool => "context: retool",
        TeamMode::SoftRebuild => "context: soft rebuild",
        TeamMode::FullRebuild => "context: full rebuild (lowers bar)",
        TeamMode::Tank => "context: tank",
    }
}

fn sample_noise(
    gullibility: f64,
    weights: &ComposeWeights<'_>,
    rng: &mut dyn RngCore,
) -> f64 {
    let stddev = weights.trade_acceptance.gullibility_noise_pct * gullibility.max(0.0)
        * weights.trade_acceptance.accept_probability_slope.abs();
    if stddev <= 0.0 {
        return 0.0;
    }
    let normal = Normal::new(0.0, stddev).expect("stddev > 0");
    let mut wrapper = RngWrapper(rng);
    normal.sample(&mut wrapper)
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Adapter so `Distribution::sample` can use a `&mut dyn RngCore`.
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

fn commentary_for(verdict: &Verdict, top_reason: Option<&Reason>) -> String {
    let label = top_reason.map(|r| r.label).unwrap_or("the math");
    match verdict {
        Verdict::Accept => format!("Works for us — {label} tips it our way."),
        Verdict::Counter(_) => format!("Close, but {label} is the sticking point — counter coming."),
        Verdict::Reject(reason) => match reason {
            RejectReason::InsufficientValue => {
                format!("Doesn't move the needle — {label} kills it.")
            }
            RejectReason::Other(s) if s == "untouchable" => {
                "Not on the table.".to_string()
            }
            other => format!("Pass — {other:?}."),
        },
    }
}
