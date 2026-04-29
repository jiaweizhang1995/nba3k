//! 82-game schedule generator.
//!
//! Three stages:
//! 1. Matchup solver — combinatorial, deterministic, no RNG. Produces the
//!    1230 unordered (home, away) pairs with NBA distribution: 4× per
//!    division opponent, 4× for 6 conf-non-div opponents and 3× for the
//!    other 4, and 2× per inter-conference opponent.
//! 2. Greedy date assigner — walks shuffled games, assigns earliest valid
//!    date subject to "max 1 game per team per day" and "max 4-in-5".
//! 3. Simulated-annealing fixup — swaps dates between game pairs to reduce
//!    a per-team back-to-back / bunched-week energy function.

use chrono::{Duration, NaiveDate};
use nba3k_core::{Conference, GameId, SeasonId, Team, TeamId};
use rand::seq::SliceRandom;
use rand::Rng;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// First and last day of the regular season window. ~174 days.
pub const SEASON_START: (i32, u32, u32) = (2025, 10, 21);
pub const SEASON_END: (i32, u32, u32) = (2026, 4, 12);

/// One scheduled game: matchup + date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledGame {
    pub id: GameId,
    pub season: SeasonId,
    pub date: NaiveDate,
    pub home: TeamId,
    pub away: TeamId,
}

/// Full season schedule, sorted by date then game id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub season: SeasonId,
    pub games: Vec<ScheduledGame>,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl Schedule {
    /// Generate a full 82-game schedule using the hardcoded 2025-26 window.
    /// Kept as a thin shim for old callers; new code should pass an explicit
    /// `start` / `end` via `generate_with_dates`.
    pub fn generate(season: SeasonId, seed: u64, teams: &[Team]) -> Self {
        let start = NaiveDate::from_ymd_opt(SEASON_START.0, SEASON_START.1, SEASON_START.2)
            .expect("valid season start");
        let end = NaiveDate::from_ymd_opt(SEASON_END.0, SEASON_END.1, SEASON_END.2)
            .expect("valid season end");
        Self::generate_with_dates(season, seed, teams, start, end)
    }

    /// Generate a full 82-game schedule between `start` and `end`.
    /// Deterministic for given (seed, teams, start, end).
    pub fn generate_with_dates(
        season: SeasonId,
        seed: u64,
        teams: &[Team],
        start: NaiveDate,
        end: NaiveDate,
    ) -> Self {
        assert_eq!(teams.len(), 30, "schedule generator expects 30 teams");
        assert!(end > start, "season end must come after start");

        let pairs = matchups(teams, seed);
        debug_assert_eq!(pairs.len(), 1230, "matchup solver produced wrong total");

        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut games = initial_dates(season, pairs, start, end, &mut rng);
        sa_fixup(&mut games, start, end, teams.len(), &mut rng, 80_000);

        games.sort_by_key(|g| (g.date, g.id.0));
        Self {
            season,
            games,
            start,
            end,
        }
    }
}

/// Stage 1 — produce 1230 unordered (home, away) pairs.
///
/// Per-team breakdown:
/// - 4 games × 4 division opponents = 16
/// - 4 games × 6 conf-non-div opponents = 24
/// - 3 games × 4 conf-non-div opponents = 12
/// - 2 games × 15 inter-conf opponents = 30
/// - total = 82
///
/// Home/away split:
/// - 4× series: 2H / 2A
/// - 3× series: 2H / 1A for one side, 1H / 2A for the other (alternates)
/// - 2× series: 1H / 1A
///
/// The selection of which 6 conf-non-div opponents are 4× vs 3× is driven
/// by `seed` so different seasons get different rotations.
pub fn matchups(teams: &[Team], seed: u64) -> Vec<(TeamId, TeamId)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut pairs: Vec<(TeamId, TeamId)> = Vec::with_capacity(1230);

    let by_id: HashMap<TeamId, &Team> = teams.iter().map(|t| (t.id, t)).collect();
    let team_ids: Vec<TeamId> = {
        let mut ids: Vec<TeamId> = teams.iter().map(|t| t.id).collect();
        ids.sort();
        ids
    };

    // Inter-conference: every East-West pair plays 2× (1H / 1A).
    let east: Vec<TeamId> = team_ids
        .iter()
        .copied()
        .filter(|id| by_id[id].conference == Conference::East)
        .collect();
    let west: Vec<TeamId> = team_ids
        .iter()
        .copied()
        .filter(|id| by_id[id].conference == Conference::West)
        .collect();
    assert_eq!(east.len(), 15, "expected 15 East teams");
    assert_eq!(west.len(), 15, "expected 15 West teams");

    for &e in &east {
        for &w in &west {
            pairs.push((e, w));
            pairs.push((w, e));
        }
    }

    // Per conference: division (4×) and non-div (3 or 4×).
    for conf_teams in [&east, &west] {
        // Division pairs: 4× each, 2H/2A.
        for i in 0..conf_teams.len() {
            for j in (i + 1)..conf_teams.len() {
                let a = conf_teams[i];
                let b = conf_teams[j];
                if by_id[&a].division == by_id[&b].division {
                    pairs.push((a, b));
                    pairs.push((a, b));
                    pairs.push((b, a));
                    pairs.push((b, a));
                }
            }
        }

        // Non-division conference pairs: each team has 10 of these. We need
        // exactly 6 to be 4× and 4 to be 3× per team. That requires choosing,
        // across the 6 divisions, two "host" divisions that this team plays
        // 4× (5 + 5 = 10 opponents... no — division has 5 teams; this team's
        // own division has 4 other teams, so non-div conference has
        // 5 + 5 = 10 opponents from the other two divisions in this conference).
        //
        // Pattern (mirrors NBA rotation): for each pair of teams in two
        // distinct conf divisions, the matchup is 4× iff a deterministic
        // hash of (seed, low_div, high_div, pair_index) selects them.
        //
        // We use a simpler, exact approach: build a 5x5 bipartite mapping
        // between each pair of divisions (A, B) where 12 of the 25 (team,
        // team) pairs are 4× and the remaining 13 are 3×, and balance
        // counts so each team ends up with 6×4 and 4×3.

        // Group conference into divisions. Use a Vec<(Division, Vec<TeamId>)>
        // because nba3k_core::Division does not implement Hash.
        let mut div_buckets: Vec<(nba3k_core::Division, Vec<TeamId>)> = Vec::new();
        for &id in conf_teams.iter() {
            let d = by_id[&id].division;
            if let Some(slot) = div_buckets.iter_mut().find(|(div, _)| *div == d) {
                slot.1.push(id);
            } else {
                div_buckets.push((d, vec![id]));
            }
        }
        // Sort each bucket and the bucket order so iteration is deterministic.
        for (_, v) in div_buckets.iter_mut() {
            v.sort();
        }
        div_buckets.sort_by_key(|(d, _)| format!("{:?}", d));
        assert_eq!(div_buckets.len(), 3);

        // For each pair of divisions, build a 5x5 binary matrix M where
        // M[i][j] == true means team i (in div X) plays team j (in div Y) 4×.
        // Constraint: exactly 3 trues per row AND 3 trues per column (per
        // team-pair-of-divisions). Across the team's two foreign divisions,
        // 3 + 3 = 6 4× opponents → matches NBA shape.
        for di in 0..div_buckets.len() {
            for dj in (di + 1)..div_buckets.len() {
                let a = &div_buckets[di].1;
                let b = &div_buckets[dj].1;
                let m = balanced_5x5(&mut rng);
                for (i, &ta) in a.iter().enumerate() {
                    for (j, &tb) in b.iter().enumerate() {
                        if m[i][j] {
                            // 4×: 2H/2A.
                            pairs.push((ta, tb));
                            pairs.push((ta, tb));
                            pairs.push((tb, ta));
                            pairs.push((tb, ta));
                        } else {
                            // 3×: 2H/1A for one side, 1H/2A for the other.
                            // Direction alternates by (i+j) parity for fairness.
                            if (i + j) % 2 == 0 {
                                pairs.push((ta, tb));
                                pairs.push((ta, tb));
                                pairs.push((tb, ta));
                            } else {
                                pairs.push((ta, tb));
                                pairs.push((tb, ta));
                                pairs.push((tb, ta));
                            }
                        }
                    }
                }
            }
        }
    }

    pairs
}

/// Build a 5×5 0/1 matrix with exactly 3 ones per row and 3 per column
/// (equivalent to a 3-regular bipartite graph). Randomized via `rng` so
/// different seeds rotate the schedule. We pick uniformly from a set of
/// permutation-based decompositions: each 3-regular bipartite K5,5
/// subgraph is a sum of three permutation matrices.
fn balanced_5x5(rng: &mut ChaCha8Rng) -> [[bool; 5]; 5] {
    let mut m = [[false; 5]; 5];
    let mut indices: Vec<usize> = (0..5).collect();
    // Three permutations of {0..5}, but they must be pairwise disjoint as
    // permutation matrices — i.e., no two perms map any row to the same
    // column. Instead of brute-forcing, build via Latin-square row shifts.
    // A 5×5 Latin square row's positions for symbol k partition into 5
    // permutations. Pick any 3 of those 5 symbols.
    let mut symbols: Vec<usize> = (0..5).collect();
    symbols.shuffle(rng);
    let chosen = &symbols[..3];

    // Latin square: cell (i, j) holds symbol (i + j) mod 5.
    // For symbol s, the permutation row→col is col = (s - i) mod 5.
    // Apply rotation offset to randomize: shift cols by a random amount.
    let col_shift: usize = rng.gen_range(0..5);
    let row_shift: usize = rng.gen_range(0..5);

    indices.shuffle(rng);
    let row_perm = indices.clone();
    let mut col_indices: Vec<usize> = (0..5).collect();
    col_indices.shuffle(rng);
    let col_perm = col_indices;

    for &s in chosen {
        for i in 0..5 {
            let j = ((s + 5 - i) % 5 + col_shift) % 5;
            let ri = (row_perm[i] + row_shift) % 5;
            let cj = col_perm[j];
            m[ri][cj] = true;
        }
    }

    // Sanity: each row should have exactly 3 trues, each col exactly 3.
    debug_assert!(m.iter().all(|row| row.iter().filter(|&&x| x).count() == 3));
    debug_assert!((0..5).all(|c| (0..5).filter(|&r| m[r][c]).count() == 3));
    m
}

/// Stage 2 — assign each (home, away) pair a valid date, spreading games
/// across the full season window rather than packing them at the start.
fn initial_dates(
    season: SeasonId,
    mut pairs: Vec<(TeamId, TeamId)>,
    start: NaiveDate,
    end: NaiveDate,
    rng: &mut ChaCha8Rng,
) -> Vec<ScheduledGame> {
    pairs.shuffle(rng);

    let total_days = (end - start).num_days() as usize + 1;
    let mut team_day_used: HashMap<TeamId, Vec<bool>> = HashMap::new();
    // Per-team running game count, used to spread games across the window.
    let mut team_games_placed: HashMap<TeamId, u32> = HashMap::new();

    let mut placed: Vec<ScheduledGame> = Vec::with_capacity(pairs.len());
    let mut next_id: u64 = 1;

    // Each team plays 82 games over `total_days`. Target spacing is
    // total_days / 82 ≈ 2.12 days per game. For each pair, target day for
    // each team is `(games_already_placed + 1) * total_days / 82`. We pick
    // a day near the larger of the two target days that is free for both
    // teams and respects the 4-in-5 constraint.
    for (home, away) in pairs {
        team_day_used
            .entry(home)
            .or_insert_with(|| vec![false; total_days]);
        team_day_used
            .entry(away)
            .or_insert_with(|| vec![false; total_days]);
        let h_count = *team_games_placed.entry(home).or_insert(0);
        let a_count = *team_games_placed.entry(away).or_insert(0);
        // Target day per team: keeps each team on a roughly even cadence.
        let target_h = ((h_count as usize + 1) * total_days) / 83;
        let target_a = ((a_count as usize + 1) * total_days) / 83;
        let target = target_h.max(target_a).min(total_days - 1);

        let mut chosen_day: Option<usize> = None;

        // Coin-flip: ~half the time, prefer days that don't create a
        // back-to-back; the other half, accept any day that satisfies the
        // 4-in-5 constraint. This produces an initial state where each
        // team already has ~10-14 b2bs, near the NBA mean — gives the SA
        // a useful starting point instead of having to introduce b2bs.
        let prefer_no_b2b = rng.gen_bool(0.62);
        if prefer_no_b2b {
            'outer: for offset in 0..total_days {
                for &dir in &[1i64, -1] {
                    let candidate = target as i64 + dir * offset as i64;
                    if candidate < 0 || candidate as usize >= total_days {
                        continue;
                    }
                    let d = candidate as usize;
                    if day_ok_strict(&team_day_used, home, away, d)
                        && !creates_back_to_back(&team_day_used, home, away, d)
                    {
                        chosen_day = Some(d);
                        break 'outer;
                    }
                }
            }
        }
        if chosen_day.is_none() {
            'middle: for offset in 0..total_days {
                for &dir in &[1i64, -1] {
                    let candidate = target as i64 + dir * offset as i64;
                    if candidate < 0 || candidate as usize >= total_days {
                        continue;
                    }
                    let d = candidate as usize;
                    if day_ok_strict(&team_day_used, home, away, d) {
                        chosen_day = Some(d);
                        break 'middle;
                    }
                }
            }
        }
        // Fallback: relax 4-in-5, keep "1 game per team per day".
        if chosen_day.is_none() {
            'fb: for offset in 0..total_days {
                for &dir in &[1i64, -1] {
                    let candidate = target as i64 + dir * offset as i64;
                    if candidate < 0 || candidate as usize >= total_days {
                        continue;
                    }
                    let d = candidate as usize;
                    if !team_day_used[&home][d] && !team_day_used[&away][d] {
                        chosen_day = Some(d);
                        break 'fb;
                    }
                }
            }
        }

        let d = chosen_day.expect("greedy date assigner could not place game");
        team_day_used.get_mut(&home).unwrap()[d] = true;
        team_day_used.get_mut(&away).unwrap()[d] = true;
        *team_games_placed.get_mut(&home).unwrap() += 1;
        *team_games_placed.get_mut(&away).unwrap() += 1;

        placed.push(ScheduledGame {
            id: GameId(next_id),
            season,
            date: start + Duration::days(d as i64),
            home,
            away,
        });
        next_id += 1;
    }

    placed
}

/// True if scheduling (home, away) on day `d` would create a back-to-back
/// for either team (i.e., either team plays on `d-1` or `d+1`).
fn creates_back_to_back(
    team_day_used: &HashMap<TeamId, Vec<bool>>,
    home: TeamId,
    away: TeamId,
    d: usize,
) -> bool {
    for team in [home, away] {
        let used = &team_day_used[&team];
        if d > 0 && used[d - 1] {
            return true;
        }
        if d + 1 < used.len() && used[d + 1] {
            return true;
        }
    }
    false
}

fn day_ok_strict(
    team_day_used: &HashMap<TeamId, Vec<bool>>,
    home: TeamId,
    away: TeamId,
    d: usize,
) -> bool {
    if team_day_used[&home][d] || team_day_used[&away][d] {
        return false;
    }
    // 4-in-5: at most 4 games in any rolling 5-day window including `d`.
    for team in [home, away] {
        let used = &team_day_used[&team];
        for window_start in d.saturating_sub(4)..=d {
            let window_end = (window_start + 4).min(used.len() - 1);
            let mut count = 0;
            for i in window_start..=window_end {
                if used[i] {
                    count += 1;
                }
            }
            if count + 1 > 4 {
                return false;
            }
        }
    }
    true
}

/// Stage 3 — simulated-annealing fixup. Swap dates between random game
/// pairs; accept downhill moves always, uphill with `exp(-ΔE/T)`.
///
/// We use a per-team incremental energy model so each candidate swap costs
/// O(games-per-affected-team) instead of O(total games). With 4 affected
/// teams and ~82 games each, that's ~330 ops per move vs. 2460 for the
/// naive recomputation.
fn sa_fixup(
    games: &mut [ScheduledGame],
    start: NaiveDate,
    _end: NaiveDate,
    _n_teams: usize,
    rng: &mut ChaCha8Rng,
    iters: usize,
) {
    if games.is_empty() {
        return;
    }
    // Build per-team sorted day vectors and per-team energy components.
    let mut by_team: HashMap<TeamId, Vec<i64>> = HashMap::new();
    for g in games.iter() {
        let d = (g.date - start).num_days();
        by_team.entry(g.home).or_default().push(d);
        by_team.entry(g.away).or_default().push(d);
    }
    for v in by_team.values_mut() {
        v.sort();
    }
    let mut team_e: HashMap<TeamId, f64> = HashMap::new();
    let mut total_e = 0.0;
    for (tid, days) in by_team.iter() {
        let e = team_energy(days);
        total_e += e;
        team_e.insert(*tid, e);
    }

    let t0 = 8.0_f64;
    let t_end = 0.02_f64;
    let alpha = (t_end / t0).powf(1.0 / iters.max(1) as f64);
    let mut t = t0;

    for _ in 0..iters {
        let i = rng.gen_range(0..games.len());
        let j = rng.gen_range(0..games.len());
        if i == j {
            t *= alpha;
            continue;
        }
        if games[i].date == games[j].date {
            t *= alpha;
            continue;
        }
        if !swap_legal(games, i, j) {
            t *= alpha;
            continue;
        }
        let date_i = games[i].date;
        let date_j = games[j].date;
        let di = (date_i - start).num_days();
        let dj = (date_j - start).num_days();

        // Affected teams: {home_i, away_i, home_j, away_j}, deduplicated.
        let mut affected: Vec<TeamId> = Vec::with_capacity(4);
        for t in [games[i].home, games[i].away, games[j].home, games[j].away] {
            if !affected.contains(&t) {
                affected.push(t);
            }
        }

        // Compute pre-swap and post-swap energy for affected teams.
        let mut pre: f64 = 0.0;
        let mut post: f64 = 0.0;
        let mut new_day_lists: Vec<(TeamId, Vec<i64>)> = Vec::with_capacity(affected.len());
        for &tid in &affected {
            pre += team_e[&tid];
            // Build new day list: remove old date(s), insert new date(s).
            let touches_i = games[i].home == tid || games[i].away == tid;
            let touches_j = games[j].home == tid || games[j].away == tid;
            let old = by_team.get(&tid).unwrap();
            let mut new_days = old.clone();
            if touches_i {
                // remove di, add dj
                let pos = new_days.iter().position(|&x| x == di).unwrap();
                new_days.remove(pos);
                let ins = new_days.partition_point(|&x| x < dj);
                new_days.insert(ins, dj);
            }
            if touches_j {
                // remove dj, add di
                let pos = new_days.iter().position(|&x| x == dj).unwrap();
                new_days.remove(pos);
                let ins = new_days.partition_point(|&x| x < di);
                new_days.insert(ins, di);
            }
            let e_new = team_energy(&new_days);
            post += e_new;
            new_day_lists.push((tid, new_days));
        }

        let delta = post - pre;
        let accept = delta < 0.0 || rng.gen::<f64>() < (-delta / t).exp();
        if accept {
            games[i].date = date_j;
            games[j].date = date_i;
            for (tid, new_days) in new_day_lists {
                by_team.insert(tid, new_days);
            }
            for &tid in &affected {
                let new_e = team_energy(by_team.get(&tid).unwrap());
                total_e += new_e - team_e[&tid];
                team_e.insert(tid, new_e);
            }
            // total_e is updated incrementally; sanity-check unused but kept.
            let _ = total_e;
        }
        t *= alpha;
    }
}

/// Per-team energy: penalize back-to-backs outside [12, 16] range
/// (centered on the realistic NBA mean of ~14), and 5-day windows with
/// more than 4 games.
fn team_energy(days: &[i64]) -> f64 {
    if days.len() < 2 {
        return 0.0;
    }
    let mut b2b = 0i64;
    for w in days.windows(2) {
        if w[1] - w[0] == 1 {
            b2b += 1;
        }
    }
    // V-shaped penalty centered on b2b == 14, equal weight either side so
    // the SA pulls schedules with too few b2bs up and too many b2bs down
    // toward the NBA range. Strong penalty above 16 to push outliers down.
    let excess_high = (b2b - 16).max(0) as f64;
    let excess_low = (12 - b2b).max(0) as f64;
    let mut e = 4.0 * excess_high * excess_high + excess_low * excess_low;

    // 5-day window count using two-pointer over sorted days.
    let mut left = 0usize;
    for right in 0..days.len() {
        while days[right] - days[left] > 4 {
            left += 1;
        }
        let count = right - left + 1;
        if count > 4 {
            e += 4.0 * ((count - 4) as f64).powi(2);
        }
    }
    e
}

/// A swap of date(i) <-> date(j) is illegal if it would put either team
/// of game i (or j) on a day they already play another game.
fn swap_legal(games: &[ScheduledGame], i: usize, j: usize) -> bool {
    let gi = &games[i];
    let gj = &games[j];
    if gi.date == gj.date {
        return true;
    }
    // After swap: game i moves to gj.date, game j moves to gi.date.
    // Any *other* game k that shares a team with i and falls on gj.date is
    // a conflict. Symmetric for j on gi.date.
    for (k, g) in games.iter().enumerate() {
        if k == i || k == j {
            continue;
        }
        if g.date == gj.date
            && (g.home == gi.home || g.away == gi.home || g.home == gi.away || g.away == gi.away)
        {
            return false;
        }
        if g.date == gi.date
            && (g.home == gj.home || g.away == gj.home || g.home == gj.away || g.away == gj.away)
        {
            return false;
        }
    }
    true
}

/// Per-team back-to-back counts (utility for tests + standings UI).
pub fn back_to_back_counts(schedule: &Schedule) -> HashMap<TeamId, u32> {
    let mut by_team: HashMap<TeamId, Vec<NaiveDate>> = HashMap::new();
    for g in &schedule.games {
        by_team.entry(g.home).or_default().push(g.date);
        by_team.entry(g.away).or_default().push(g.date);
    }
    let mut out = HashMap::new();
    for (team, mut dates) in by_team {
        dates.sort();
        let mut b2b = 0;
        for w in dates.windows(2) {
            if (w[1] - w[0]).num_days() == 1 {
                b2b += 1;
            }
        }
        out.insert(team, b2b);
    }
    out
}

/// Per-team total games (utility).
pub fn games_per_team(schedule: &Schedule) -> HashMap<TeamId, u32> {
    let mut out = HashMap::new();
    for g in &schedule.games {
        *out.entry(g.home).or_insert(0) += 1;
        *out.entry(g.away).or_insert(0) += 1;
    }
    out
}
