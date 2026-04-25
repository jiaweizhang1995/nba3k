//! Standings tracker. Records game results, ranks by tiebreakers.

use nba3k_core::{Conference, Division, GameResult, Team, TeamId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecord {
    pub team: TeamId,
    pub conference: Conference,
    pub division: Division,
    pub wins: u16,
    pub losses: u16,
    pub division_wins: u16,
    pub division_losses: u16,
    pub conference_wins: u16,
    pub conference_losses: u16,
    pub point_diff: i32,
    pub conf_rank: u8,
    pub division_rank: u8,
}

impl TeamRecord {
    pub fn games_played(&self) -> u16 {
        self.wins + self.losses
    }

    pub fn win_pct(&self) -> f32 {
        let gp = self.games_played();
        if gp == 0 {
            0.5
        } else {
            self.wins as f32 / gp as f32
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Standings {
    pub records: HashMap<TeamId, TeamRecord>,
    /// (winner, loser) -> count. Used for head-to-head tiebreaker.
    pub head_to_head: HashMap<(TeamId, TeamId), u16>,
}

impl Standings {
    pub fn new(teams: &[Team]) -> Self {
        let mut records = HashMap::new();
        for t in teams {
            records.insert(
                t.id,
                TeamRecord {
                    team: t.id,
                    conference: t.conference,
                    division: t.division,
                    wins: 0,
                    losses: 0,
                    division_wins: 0,
                    division_losses: 0,
                    conference_wins: 0,
                    conference_losses: 0,
                    point_diff: 0,
                    conf_rank: 0,
                    division_rank: 0,
                },
            );
        }
        Self {
            records,
            head_to_head: HashMap::new(),
        }
    }

    /// Record a completed game: bumps wins/losses, point diff, head-to-head,
    /// and (if applicable) division/conference splits.
    pub fn record_game_result(&mut self, game: &GameResult) {
        let winner = game.winner();
        let loser = game.loser();
        let win_pts = game.home_score.max(game.away_score);
        let lose_pts = game.home_score.min(game.away_score);
        let margin = win_pts as i32 - lose_pts as i32;

        let (winner_div, winner_conf) = {
            let r = self.records.get(&winner).expect("unknown team");
            (r.division, r.conference)
        };
        let (loser_div, loser_conf) = {
            let r = self.records.get(&loser).expect("unknown team");
            (r.division, r.conference)
        };

        let same_div = winner_div == loser_div;
        let same_conf = winner_conf == loser_conf;

        if let Some(r) = self.records.get_mut(&winner) {
            r.wins += 1;
            r.point_diff += margin;
            if same_div {
                r.division_wins += 1;
            }
            if same_conf {
                r.conference_wins += 1;
            }
        }
        if let Some(r) = self.records.get_mut(&loser) {
            r.losses += 1;
            r.point_diff -= margin;
            if same_div {
                r.division_losses += 1;
            }
            if same_conf {
                r.conference_losses += 1;
            }
        }
        *self.head_to_head.entry((winner, loser)).or_insert(0) += 1;
    }

    /// Re-rank teams within each conference and division using the
    /// tiebreaker chain: win% → head-to-head → division record →
    /// conference record → point differential (SRS proxy).
    pub fn recompute_ranks(&mut self) {
        let teams: Vec<TeamId> = self.records.keys().copied().collect();

        for conf in [Conference::East, Conference::West] {
            let mut ids: Vec<TeamId> = teams
                .iter()
                .copied()
                .filter(|id| self.records[id].conference == conf)
                .collect();
            self.sort_by_tiebreakers(&mut ids);
            for (i, id) in ids.iter().enumerate() {
                self.records.get_mut(id).unwrap().conf_rank = (i + 1) as u8;
            }
        }

        // Division ranks: same tiebreaker chain restricted to division.
        // Vec keyed on Division because Division does not implement Hash.
        let mut by_div: Vec<(Division, Vec<TeamId>)> = Vec::new();
        for id in &teams {
            let d = self.records[id].division;
            if let Some(slot) = by_div.iter_mut().find(|(div, _)| *div == d) {
                slot.1.push(*id);
            } else {
                by_div.push((d, vec![*id]));
            }
        }
        for (_div, mut ids) in by_div {
            self.sort_by_tiebreakers(&mut ids);
            for (i, id) in ids.iter().enumerate() {
                self.records.get_mut(&id).unwrap().division_rank = (i + 1) as u8;
            }
        }
    }

    fn sort_by_tiebreakers(&self, ids: &mut [TeamId]) {
        ids.sort_by(|a, b| compare_tiebreakers(self, *a, *b));
    }
}

/// Tiebreaker comparator. Returns Ordering::Less when `a` ranks above `b`.
pub fn compare_tiebreakers(s: &Standings, a: TeamId, b: TeamId) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ra = &s.records[&a];
    let rb = &s.records[&b];
    // Higher win% ranks higher → reverse order.
    let pa = ra.win_pct();
    let pb = rb.win_pct();
    if (pa - pb).abs() > f32::EPSILON {
        return pb.partial_cmp(&pa).unwrap_or(Ordering::Equal);
    }
    // Head-to-head: more wins vs the other team ranks higher.
    let h2h_a = *s.head_to_head.get(&(a, b)).unwrap_or(&0);
    let h2h_b = *s.head_to_head.get(&(b, a)).unwrap_or(&0);
    if h2h_a != h2h_b {
        return h2h_b.cmp(&h2h_a);
    }
    // Division record (only meaningful if same division — fall through
    // otherwise; comparing across divisions is fine, just less informative).
    let da = win_pct(ra.division_wins, ra.division_losses);
    let db = win_pct(rb.division_wins, rb.division_losses);
    if (da - db).abs() > f32::EPSILON {
        return db.partial_cmp(&da).unwrap_or(Ordering::Equal);
    }
    // Conference record.
    let ca = win_pct(ra.conference_wins, ra.conference_losses);
    let cb = win_pct(rb.conference_wins, rb.conference_losses);
    if (ca - cb).abs() > f32::EPSILON {
        return cb.partial_cmp(&ca).unwrap_or(Ordering::Equal);
    }
    // SRS proxy: point differential.
    if ra.point_diff != rb.point_diff {
        return rb.point_diff.cmp(&ra.point_diff);
    }
    // Stable fallback: team id ascending.
    a.0.cmp(&b.0)
}

fn win_pct(wins: u16, losses: u16) -> f32 {
    let gp = wins + losses;
    if gp == 0 {
        0.5
    } else {
        wins as f32 / gp as f32
    }
}
