//! End-of-regular-season awards engine.
//!
//! Pipeline: aggregate per-player season lines from `Vec<GameResult>`,
//! compute one composite score per award, simulate a 100-voter ballot using
//! the canonical 10-7-5-3-1 (MVP / All-NBA) or 5-3-1 (DPOY / ROY / Sixth Man /
//! MIP / COY) weights, and pick the winner.
//!
//! All RNG is `ChaCha8Rng` seeded from `season.0` so re-running the engine
//! on the same inputs yields the same ballots.
//!
//! Rookie eligibility (v1): a player counts as a rookie when their `age <= 22`
//! and they did not appear in `prior_games`. This is the documented stand-in
//! until Worker C's `dev.last_progressed_season` lands.
//!
//! Sixth Man eligibility (v1): we do not yet have `Player.role`'s SixthMan
//! variant wired through the store (Worker B's flow). Stand-in: top scorers
//! whose minutes-per-game falls between 18 and 28 (i.e. heavy-rotation
//! non-starters). Once `role` lands, swap in `role == PlayerRole::SixthMan`.

use crate::standings::{Standings, TeamRecord};
use nba3k_core::{GameResult, PlayerId, Position, SeasonId, TeamId};
use rand::Rng;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ----------------------------------------------------------------------
// Public types
// ----------------------------------------------------------------------

/// Award identifier — used as the `award` column in the persisted `awards`
/// table and as the JSON key in `season-summary --json`. Keep string forms
/// stable (consumers may depend on them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AwardKind {
    MVP,
    DPOY,
    ROY,
    SixthMan,
    MIP,
    COY,
    AllNBA1,
    AllNBA2,
    AllNBA3,
    AllDefensive1,
    AllDefensive2,
    AllStarEast,
    AllStarWest,
}

impl AwardKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MVP => "MVP",
            Self::DPOY => "DPOY",
            Self::ROY => "ROY",
            Self::SixthMan => "SixthMan",
            Self::MIP => "MIP",
            Self::COY => "COY",
            Self::AllNBA1 => "AllNBA1",
            Self::AllNBA2 => "AllNBA2",
            Self::AllNBA3 => "AllNBA3",
            Self::AllDefensive1 => "AllDefensive1",
            Self::AllDefensive2 => "AllDefensive2",
            Self::AllStarEast => "AllStarEast",
            Self::AllStarWest => "AllStarWest",
        }
    }
}

/// One award's outcome. Winner is `top_voted` (highest weighted ballot share);
/// the full ranked ballot is preserved for UI / debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwardResult {
    pub kind: AwardKind,
    pub winner: Option<PlayerId>,
    /// (player, weighted ballot share). Sorted descending by share.
    pub ballot: Vec<(PlayerId, f32)>,
}

/// Like `AwardResult` but for COY which votes on coaches (= teams in our
/// model — light Coach struct lives on Team).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamAwardResult {
    pub kind: AwardKind,
    pub winner: Option<TeamId>,
    pub ballot: Vec<(TeamId, f32)>,
}

/// Per-player counting-stats aggregate for one season. Built from
/// `Vec<GameResult>` via `aggregate_season`.
#[derive(Debug, Clone)]
pub struct PlayerSeason {
    pub player: PlayerId,
    pub team: Option<TeamId>,
    pub games: u16,
    pub minutes: u32,
    pub pts: u32,
    pub reb: u32,
    pub ast: u32,
    pub stl: u32,
    pub blk: u32,
    pub tov: u32,
}

impl PlayerSeason {
    fn empty(player: PlayerId) -> Self {
        Self {
            player,
            team: None,
            games: 0,
            minutes: 0,
            pts: 0,
            reb: 0,
            ast: 0,
            stl: 0,
            blk: 0,
            tov: 0,
        }
    }

    pub fn ppg(&self) -> f32 {
        if self.games == 0 { 0.0 } else { self.pts as f32 / self.games as f32 }
    }
    pub fn mpg(&self) -> f32 {
        if self.games == 0 { 0.0 } else { self.minutes as f32 / self.games as f32 }
    }
    pub fn rpg(&self) -> f32 {
        if self.games == 0 { 0.0 } else { self.reb as f32 / self.games as f32 }
    }
    pub fn apg(&self) -> f32 {
        if self.games == 0 { 0.0 } else { self.ast as f32 / self.games as f32 }
    }
    pub fn spg(&self) -> f32 {
        if self.games == 0 { 0.0 } else { self.stl as f32 / self.games as f32 }
    }
    pub fn bpg(&self) -> f32 {
        if self.games == 0 { 0.0 } else { self.blk as f32 / self.games as f32 }
    }
    pub fn tpg(&self) -> f32 {
        if self.games == 0 { 0.0 } else { self.tov as f32 / self.games as f32 }
    }
}

/// Bag of every season aggregate the awards engine needs. Built once per
/// season and reused across awards (avoids re-walking `games`).
#[derive(Debug, Clone, Default)]
pub struct SeasonAggregate {
    pub by_player: HashMap<PlayerId, PlayerSeason>,
    pub team_drtg: HashMap<TeamId, f32>,
}

// ----------------------------------------------------------------------
// Aggregation
// ----------------------------------------------------------------------

/// Walk regular-season games, build per-player season totals + a coarse
/// team defensive rating (points allowed per game, normalized to 100 poss.
/// using a flat 100-possession assumption — adequate for ranking, not
/// for absolute DRtg).
pub fn aggregate_season(games: &[GameResult]) -> SeasonAggregate {
    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    // (team_id, points_allowed_total, games_played).
    let mut def: HashMap<TeamId, (u32, u32)> = HashMap::new();

    for g in games.iter().filter(|g| !g.is_playoffs) {
        // Per-team defensive accumulator.
        let h_entry = def.entry(g.home).or_insert((0, 0));
        h_entry.0 += g.away_score as u32;
        h_entry.1 += 1;
        let a_entry = def.entry(g.away).or_insert((0, 0));
        a_entry.0 += g.home_score as u32;
        a_entry.1 += 1;

        for line in g.box_score.home_lines.iter() {
            accumulate_line(&mut by_player, line, g.home);
        }
        for line in g.box_score.away_lines.iter() {
            accumulate_line(&mut by_player, line, g.away);
        }
    }

    let team_drtg = def
        .into_iter()
        .map(|(t, (pts, gp))| {
            let drtg = if gp == 0 { 110.0 } else { pts as f32 / gp as f32 };
            (t, drtg)
        })
        .collect();

    SeasonAggregate { by_player, team_drtg }
}

fn accumulate_line(
    by_player: &mut HashMap<PlayerId, PlayerSeason>,
    line: &nba3k_core::PlayerLine,
    team: TeamId,
) {
    let entry = by_player
        .entry(line.player)
        .or_insert_with(|| PlayerSeason::empty(line.player));
    entry.team = Some(team);
    entry.games += 1;
    entry.minutes += line.minutes as u32;
    entry.pts += line.pts as u32;
    entry.reb += line.reb as u32;
    entry.ast += line.ast as u32;
    entry.stl += line.stl as u32;
    entry.blk += line.blk as u32;
    entry.tov += line.tov as u32;
}

// ----------------------------------------------------------------------
// Score formulas (composite per award)
// ----------------------------------------------------------------------

/// MVP composite — box-score production gated by team success.
///
/// Components per game: `PTS + 0.5×REB + 0.7×AST + 1.5×STL + 1.5×BLK − 1.0×TOV`.
/// Multiplied by `team_win_pct_gate`: 0 when team has fewer than 30 wins
/// (cuts noise from low-volume scorers on tanking teams), otherwise the
/// team's regular-season win pct.
///
/// Documented in `phases/M5-realism-v2.md` "Decision log".
pub fn mvp_composite(season: &PlayerSeason, team_win_pct: f32, team_wins: u16) -> f32 {
    if season.games < 40 {
        // Volume gate: same as 65-game rule but relaxed for v1 (no schedule
        // calendar awareness here). Players who barely played can't win MVP.
        return 0.0;
    }
    let per_game = season.pts as f32
        + 0.5 * season.reb as f32
        + 0.7 * season.ast as f32
        + 1.5 * season.stl as f32
        + 1.5 * season.blk as f32
        - 1.0 * season.tov as f32;
    let avg = per_game / season.games as f32;
    let gate = if team_wins < 30 { 0.0 } else { team_win_pct };
    avg * gate
}

/// DPOY composite — defensive event rate × team defensive quality.
/// Higher is better. Team DRtg (points allowed per game) is inverted and
/// normalized so an elite (low) DRtg multiplies up.
pub fn dpoy_composite(season: &PlayerSeason, team_drtg: f32) -> f32 {
    if season.games < 40 {
        return 0.0;
    }
    let stocks = (season.stl + season.blk) as f32 / season.games as f32;
    let reb_rate = season.reb as f32 / season.games.max(1) as f32;
    let raw = stocks * 2.0 + 0.25 * reb_rate;
    // Convert "points allowed per game" to a positive multiplier centered at
    // 1.0 (110 pts allowed = 1.0; 100 = 1.20; 120 = 0.80).
    let drtg_mult = (220.0 - team_drtg) / 110.0;
    raw * drtg_mult.max(0.1)
}

/// ROY composite — counting stats only, gated on rookie eligibility.
pub fn roy_composite(season: &PlayerSeason) -> f32 {
    if season.games < 30 {
        return 0.0;
    }
    season.ppg() + 0.5 * season.rpg() + 0.7 * season.apg()
}

/// Sixth Man composite — scoring + assists for non-starter rotation players.
pub fn sixth_man_composite(season: &PlayerSeason) -> f32 {
    if season.games < 40 {
        return 0.0;
    }
    let mpg = season.mpg();
    if !(18.0..=28.0).contains(&mpg) {
        return 0.0;
    }
    season.ppg() + 0.4 * season.apg() + 0.3 * season.rpg()
}

/// MIP composite — current composite minus prior-season composite.
/// Players without a meaningful prior season (under 30 games) are excluded;
/// they're more likely ROY candidates than MIP.
pub fn mip_delta(curr: &PlayerSeason, prev: Option<&PlayerSeason>) -> f32 {
    if curr.games < 40 {
        return 0.0;
    }
    let p = match prev {
        Some(p) if p.games >= 30 => p,
        _ => return 0.0,
    };
    let curr_score = curr.ppg() + 0.5 * curr.rpg() + 0.7 * curr.apg();
    let prev_score = p.ppg() + 0.5 * p.rpg() + 0.7 * p.apg();
    curr_score - prev_score
}

// ----------------------------------------------------------------------
// Ballot simulation
// ----------------------------------------------------------------------

/// 100-voter ballot using `weights` (e.g. [10, 7, 5, 3, 1] for MVP).
/// Each voter picks a top-N from `scores` with controlled noise; we tally
/// weighted points and return the result sorted descending.
///
/// Determinism: `rng` is the only entropy source. Pass a seeded ChaCha8Rng.
pub fn run_ballot<K: Copy + Eq + std::hash::Hash + Ord>(
    scores: &[(K, f32)],
    weights: &[u32],
    voters: u32,
    rng: &mut ChaCha8Rng,
) -> Vec<(K, f32)> {
    if scores.is_empty() {
        return Vec::new();
    }
    let mut tally: HashMap<K, f32> = HashMap::new();
    let n_select = weights.len();

    // Pre-sort by score desc for sane noise application.
    let mut base: Vec<(K, f32)> = scores.to_vec();
    base.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    // Bound the noise to ~10% of the top score so the strongest candidate
    // wins most ballots while still allowing the long tail to leak through.
    let top = base[0].1.max(1.0);
    let sigma = top * 0.10;

    for _ in 0..voters {
        let mut perturbed: Vec<(K, f32)> = base
            .iter()
            .map(|(k, s)| {
                let noise: f32 = rng.gen_range(-sigma..=sigma);
                (*k, s + noise)
            })
            .collect();
        perturbed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (i, w) in weights.iter().enumerate().take(n_select) {
            if let Some(&(k, _)) = perturbed.get(i) {
                *tally.entry(k).or_insert(0.0) += *w as f32;
            }
        }
    }

    // Normalize tally by total possible (so values sit in 0..=1 range).
    let total_possible = voters as f32 * weights.iter().sum::<u32>() as f32;
    let mut out: Vec<(K, f32)> =
        tally.into_iter().map(|(k, v)| (k, v / total_possible)).collect();
    // Tiebreak deterministically on the natural order of K (e.g. PlayerId asc).
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}

// ----------------------------------------------------------------------
// Public award computations
// ----------------------------------------------------------------------

const MVP_WEIGHTS: &[u32] = &[10, 7, 5, 3, 1];
const ALLNBA_WEIGHTS: &[u32] = &[10, 7, 5, 3, 1];
const SHORT_BALLOT_WEIGHTS: &[u32] = &[5, 3, 1];
const VOTERS: u32 = 100;

fn rng_for(season: SeasonId, salt: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(season.0 as u64 ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

pub fn compute_mvp(
    aggregate: &SeasonAggregate,
    standings: &Standings,
    season: SeasonId,
) -> AwardResult {
    let scores = mvp_scores(aggregate, standings);
    let mut rng = rng_for(season, 1);
    let ballot = run_ballot(&scores, MVP_WEIGHTS, VOTERS, &mut rng);
    AwardResult { kind: AwardKind::MVP, winner: ballot.first().map(|(p, _)| *p), ballot }
}

pub fn compute_dpoy(
    aggregate: &SeasonAggregate,
    season: SeasonId,
) -> AwardResult {
    let scores: Vec<(PlayerId, f32)> = aggregate
        .by_player
        .values()
        .filter_map(|p| {
            let drtg = p
                .team
                .and_then(|t| aggregate.team_drtg.get(&t))
                .copied()
                .unwrap_or(110.0);
            let s = dpoy_composite(p, drtg);
            if s > 0.0 { Some((p.player, s)) } else { None }
        })
        .collect();
    let mut rng = rng_for(season, 2);
    let ballot = run_ballot(&scores, SHORT_BALLOT_WEIGHTS, VOTERS, &mut rng);
    AwardResult { kind: AwardKind::DPOY, winner: ballot.first().map(|(p, _)| *p), ballot }
}

/// `is_rookie(player_id) -> bool` is supplied by the caller — the awards
/// engine has no opinion on what makes a rookie. v1 callers pass a closure
/// that returns true when player.age <= 22 AND not in `prior_games`.
pub fn compute_roy(
    aggregate: &SeasonAggregate,
    season: SeasonId,
    is_rookie: impl Fn(PlayerId) -> bool,
) -> AwardResult {
    let scores: Vec<(PlayerId, f32)> = aggregate
        .by_player
        .values()
        .filter(|p| is_rookie(p.player))
        .filter_map(|p| {
            let s = roy_composite(p);
            if s > 0.0 { Some((p.player, s)) } else { None }
        })
        .collect();
    let mut rng = rng_for(season, 3);
    let ballot = run_ballot(&scores, SHORT_BALLOT_WEIGHTS, VOTERS, &mut rng);
    AwardResult { kind: AwardKind::ROY, winner: ballot.first().map(|(p, _)| *p), ballot }
}

pub fn compute_sixth_man(
    aggregate: &SeasonAggregate,
    season: SeasonId,
) -> AwardResult {
    let scores: Vec<(PlayerId, f32)> = aggregate
        .by_player
        .values()
        .filter_map(|p| {
            let s = sixth_man_composite(p);
            if s > 0.0 { Some((p.player, s)) } else { None }
        })
        .collect();
    let mut rng = rng_for(season, 4);
    let ballot = run_ballot(&scores, SHORT_BALLOT_WEIGHTS, VOTERS, &mut rng);
    AwardResult { kind: AwardKind::SixthMan, winner: ballot.first().map(|(p, _)| *p), ballot }
}

pub fn compute_mip(
    curr: &SeasonAggregate,
    prev: &SeasonAggregate,
    season: SeasonId,
) -> AwardResult {
    let scores: Vec<(PlayerId, f32)> = curr
        .by_player
        .values()
        .filter_map(|p| {
            let prev_p = prev.by_player.get(&p.player);
            let d = mip_delta(p, prev_p);
            if d > 0.0 { Some((p.player, d)) } else { None }
        })
        .collect();
    let mut rng = rng_for(season, 5);
    let ballot = run_ballot(&scores, SHORT_BALLOT_WEIGHTS, VOTERS, &mut rng);
    AwardResult { kind: AwardKind::MIP, winner: ballot.first().map(|(p, _)| *p), ballot }
}

/// COY composite — biggest year-over-year team improvement. The "coach" in
/// our model is `Team.coach`, but we vote on TeamId here and let the caller
/// dereference. Wins delta is the headline metric; Pythagorean (point-diff)
/// improvement breaks ties.
pub fn compute_coy(
    curr: &Standings,
    prev: &Standings,
    season: SeasonId,
) -> TeamAwardResult {
    let scores: Vec<(TeamId, f32)> = curr
        .records
        .iter()
        .filter_map(|(team, r)| {
            let prev_r = prev.records.get(team)?;
            let win_delta = r.wins as f32 - prev_r.wins as f32;
            let pd_delta = (r.point_diff - prev_r.point_diff) as f32;
            // Wins drive the bulk; point diff breaks ties (~0.05 weight per
            // total-season point delta = ~4 pts ≈ 1 win equivalent).
            let s = win_delta + pd_delta * 0.05;
            if s > 0.0 { Some((*team, s)) } else { None }
        })
        .collect();
    let mut rng = rng_for(season, 6);
    let ballot = run_ballot(&scores, SHORT_BALLOT_WEIGHTS, VOTERS, &mut rng);
    TeamAwardResult { kind: AwardKind::COY, winner: ballot.first().map(|(t, _)| *t), ballot }
}

fn mvp_scores(
    aggregate: &SeasonAggregate,
    standings: &Standings,
) -> Vec<(PlayerId, f32)> {
    aggregate
        .by_player
        .values()
        .filter_map(|p| {
            let team = p.team?;
            let rec = standings.records.get(&team)?;
            let s = mvp_composite(p, rec.win_pct(), rec.wins);
            if s > 0.0 { Some((p.player, s)) } else { None }
        })
        .collect()
}

// ----------------------------------------------------------------------
// All-NBA / All-Defensive / All-Star
// ----------------------------------------------------------------------

/// Slot in an All-NBA team. Two guards, two forwards, one center, mirrored
/// across the three teams (1st/2nd/3rd). Caller supplies position resolver.
#[derive(Debug, Clone, Copy)]
pub enum LineupSlot {
    Guard,
    Forward,
    Center,
}

fn slot_for(pos: Position) -> LineupSlot {
    match pos {
        Position::PG | Position::SG => LineupSlot::Guard,
        Position::SF | Position::PF => LineupSlot::Forward,
        Position::C => LineupSlot::Center,
    }
}

/// Build All-NBA 1st / 2nd / 3rd teams using MVP composite as the score axis.
/// `position_of` resolves a `PlayerId` → `Position` (caller wires through
/// `Player.primary_position`).
///
/// Output: 15 players total (3 teams × 5 slots) with positional balance
/// (2G + 2F + 1C per team). If the candidate pool runs out within a position
/// bucket the empty slots are skipped.
pub fn compute_all_nba(
    aggregate: &SeasonAggregate,
    standings: &Standings,
    season: SeasonId,
    position_of: impl Fn(PlayerId) -> Option<Position>,
) -> [AwardResult; 3] {
    let scores = mvp_scores(aggregate, standings);
    let mut rng = rng_for(season, 7);
    let ballot = run_ballot(&scores, ALLNBA_WEIGHTS, VOTERS, &mut rng);
    let teams = build_lineup_teams(&ballot, &position_of);
    [
        team_to_result(AwardKind::AllNBA1, &teams[0]),
        team_to_result(AwardKind::AllNBA2, &teams[1]),
        team_to_result(AwardKind::AllNBA3, &teams[2]),
    ]
}

/// Build All-Defensive 1st and 2nd teams using DPOY composite as the score
/// axis. 10 players total with the same 2G + 2F + 1C balance per team.
pub fn compute_all_defensive(
    aggregate: &SeasonAggregate,
    season: SeasonId,
    position_of: impl Fn(PlayerId) -> Option<Position>,
) -> [AwardResult; 2] {
    let scores: Vec<(PlayerId, f32)> = aggregate
        .by_player
        .values()
        .filter_map(|p| {
            let drtg = p
                .team
                .and_then(|t| aggregate.team_drtg.get(&t))
                .copied()
                .unwrap_or(110.0);
            let s = dpoy_composite(p, drtg);
            if s > 0.0 { Some((p.player, s)) } else { None }
        })
        .collect();
    let mut rng = rng_for(season, 8);
    let ballot = run_ballot(&scores, ALLNBA_WEIGHTS, VOTERS, &mut rng);
    let teams = build_lineup_teams(&ballot, &position_of);
    [
        team_to_result(AwardKind::AllDefensive1, &teams[0]),
        team_to_result(AwardKind::AllDefensive2, &teams[1]),
    ]
}

/// Allocate a sorted ballot into lineup teams (5 per team, 2G+2F+1C).
/// We greedily fill team 1's open slots, then team 2's, etc. Any candidate
/// whose position can't slot into the current team gets pushed to a
/// hold list and reconsidered for later teams.
fn build_lineup_teams(
    ballot: &[(PlayerId, f32)],
    position_of: &impl Fn(PlayerId) -> Option<Position>,
) -> Vec<Vec<(PlayerId, f32)>> {
    const TEAMS: usize = 3;
    const G_PER_TEAM: usize = 2;
    const F_PER_TEAM: usize = 2;
    const C_PER_TEAM: usize = 1;

    let mut teams: Vec<Vec<(PlayerId, f32)>> = (0..TEAMS).map(|_| Vec::with_capacity(5)).collect();
    let mut g_used = [0usize; TEAMS];
    let mut f_used = [0usize; TEAMS];
    let mut c_used = [0usize; TEAMS];

    for (pid, share) in ballot {
        let Some(pos) = position_of(*pid) else { continue };
        let slot = slot_for(pos);
        // Place into the lowest-index team that still has room for this slot.
        for t in 0..TEAMS {
            let placed = match slot {
                LineupSlot::Guard => {
                    if g_used[t] < G_PER_TEAM {
                        g_used[t] += 1;
                        teams[t].push((*pid, *share));
                        true
                    } else { false }
                }
                LineupSlot::Forward => {
                    if f_used[t] < F_PER_TEAM {
                        f_used[t] += 1;
                        teams[t].push((*pid, *share));
                        true
                    } else { false }
                }
                LineupSlot::Center => {
                    if c_used[t] < C_PER_TEAM {
                        c_used[t] += 1;
                        teams[t].push((*pid, *share));
                        true
                    } else { false }
                }
            };
            if placed { break; }
        }
        // Stop once every slot in every team is filled.
        if g_used.iter().all(|&n| n == G_PER_TEAM)
            && f_used.iter().all(|&n| n == F_PER_TEAM)
            && c_used.iter().all(|&n| n == C_PER_TEAM)
        {
            break;
        }
    }
    teams
}

fn team_to_result(kind: AwardKind, team: &[(PlayerId, f32)]) -> AwardResult {
    AwardResult {
        kind,
        winner: team.first().map(|(p, _)| *p),
        ballot: team.to_vec(),
    }
}

// ----------------------------------------------------------------------
// All-Star
// ----------------------------------------------------------------------

/// All-Star roster per conference: 12 players (5 starters + 7 reserves).
/// Starters: 2 guards + 2 forwards + 1 center, picked in conference order
/// from the MVP-composite ballot. Reserves: 7 next-best players regardless
/// of position. Triggered at game-41 marker by the orchestrator; this
/// function is pure data and does not consult game count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllStarRoster {
    pub conference: nba3k_core::Conference,
    pub starters: Vec<PlayerId>,
    pub reserves: Vec<PlayerId>,
}

/// Compute East + West All-Star rosters. Uses the in-progress (mid-season)
/// `aggregate` + `standings` — ranking still applies the win-pct gate so
/// scrubs on tanking teams don't make the team.
pub fn compute_all_star(
    aggregate: &SeasonAggregate,
    standings: &Standings,
    season: SeasonId,
    position_of: impl Fn(PlayerId) -> Option<Position>,
    team_of: impl Fn(PlayerId) -> Option<TeamId>,
) -> [AllStarRoster; 2] {
    let mut east_pool: Vec<(PlayerId, f32, Position)> = Vec::new();
    let mut west_pool: Vec<(PlayerId, f32, Position)> = Vec::new();
    for p in aggregate.by_player.values() {
        let Some(team) = team_of(p.player) else { continue };
        let Some(pos) = position_of(p.player) else { continue };
        let Some(rec) = standings.records.get(&team) else { continue };
        // All-Star uses a relaxed gate: any rotation player counts (no 30-win
        // floor), but mid-season volume must be enough that a mean exists.
        if p.games < 20 { continue }
        let composite = (p.pts as f32 + 0.5 * p.reb as f32 + 0.7 * p.ast as f32
            + 1.5 * p.stl as f32 + 1.5 * p.blk as f32 - p.tov as f32)
            / p.games as f32;
        // Win-pct multiplier capped at 1.4 so a 0.700 team gets ~+12% bump,
        // a 0.300 team still gets meaningful representation.
        let bump = 0.6 + rec.win_pct().clamp(0.0, 1.0);
        let s = composite * bump;
        match rec.conference {
            nba3k_core::Conference::East => east_pool.push((p.player, s, pos)),
            nba3k_core::Conference::West => west_pool.push((p.player, s, pos)),
        }
    }

    let east = build_all_star(nba3k_core::Conference::East, east_pool, season, 9);
    let west = build_all_star(nba3k_core::Conference::West, west_pool, season, 10);
    [east, west]
}

fn build_all_star(
    conf: nba3k_core::Conference,
    mut pool: Vec<(PlayerId, f32, Position)>,
    season: SeasonId,
    salt: u64,
) -> AllStarRoster {
    // Run a ballot on the score axis ignoring position; we then pick starters
    // by scanning the ballot order and respecting positional caps.
    let scores: Vec<(PlayerId, f32)> = pool.iter().map(|(p, s, _)| (*p, *s)).collect();
    let mut rng = rng_for(season, salt);
    let ballot = run_ballot(&scores, ALLNBA_WEIGHTS, VOTERS, &mut rng);
    // Map players → position from `pool`.
    let pos_lookup: HashMap<PlayerId, Position> =
        pool.drain(..).map(|(p, _, pos)| (p, pos)).collect();

    let mut starters: Vec<PlayerId> = Vec::with_capacity(5);
    let mut g_used = 0usize;
    let mut f_used = 0usize;
    let mut c_used = 0usize;
    let mut consumed: Vec<bool> = vec![false; ballot.len()];

    for (i, (pid, _)) in ballot.iter().enumerate() {
        if starters.len() == 5 { break; }
        let Some(pos) = pos_lookup.get(pid).copied() else { continue };
        let placed = match slot_for(pos) {
            LineupSlot::Guard if g_used < 2 => { g_used += 1; true }
            LineupSlot::Forward if f_used < 2 => { f_used += 1; true }
            LineupSlot::Center if c_used < 1 => { c_used += 1; true }
            _ => false,
        };
        if placed {
            starters.push(*pid);
            consumed[i] = true;
        }
    }

    // Reserves: next 7 unplaced players in ballot order.
    let mut reserves: Vec<PlayerId> = Vec::with_capacity(7);
    for (i, (pid, _)) in ballot.iter().enumerate() {
        if reserves.len() == 7 { break; }
        if consumed[i] { continue; }
        if starters.contains(pid) { continue; }
        reserves.push(*pid);
    }

    AllStarRoster { conference: conf, starters, reserves }
}

// ----------------------------------------------------------------------
// Convenience: bundled engine output for `awards --json`
// ----------------------------------------------------------------------

/// One-shot result bag — all awards in a single call. Used by the CLI's
/// `awards` and `season-summary` commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwardsBundle {
    pub mvp: AwardResult,
    pub dpoy: AwardResult,
    pub roy: AwardResult,
    pub sixth_man: AwardResult,
    pub mip: AwardResult,
    pub coy: TeamAwardResult,
    pub all_nba: [AwardResult; 3],
    pub all_defensive: [AwardResult; 2],
}

#[allow(clippy::too_many_arguments)]
pub fn compute_all_awards(
    aggregate: &SeasonAggregate,
    standings: &Standings,
    season: SeasonId,
    prev_aggregate: Option<&SeasonAggregate>,
    prev_standings: Option<&Standings>,
    is_rookie: impl Fn(PlayerId) -> bool,
    position_of: impl Fn(PlayerId) -> Option<Position>,
) -> AwardsBundle {
    let mvp = compute_mvp(aggregate, standings, season);
    let dpoy = compute_dpoy(aggregate, season);
    let roy = compute_roy(aggregate, season, is_rookie);
    let sixth_man = compute_sixth_man(aggregate, season);
    let mip = match prev_aggregate {
        Some(prev) => compute_mip(aggregate, prev, season),
        None => AwardResult { kind: AwardKind::MIP, winner: None, ballot: vec![] },
    };
    let coy = match prev_standings {
        Some(prev) => compute_coy(standings, prev, season),
        None => TeamAwardResult { kind: AwardKind::COY, winner: None, ballot: vec![] },
    };
    let all_nba = compute_all_nba(aggregate, standings, season, &position_of);
    let all_defensive = compute_all_defensive(aggregate, season, &position_of);
    AwardsBundle { mvp, dpoy, roy, sixth_man, mip, coy, all_nba, all_defensive }
}

// Re-export helpers used by other modules that touch standings.
pub fn team_record_win_pct(rec: &TeamRecord) -> f32 {
    rec.win_pct()
}
