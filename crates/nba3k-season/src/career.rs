//! Dynasty career stats aggregation.
//!
//! Pure functions: take a `&[GameResult]` slice + a `PlayerId`, return one
//! `SeasonAvgRow` per season the player appeared in (including playoffs —
//! a player's box-score line is identical regardless of `is_playoffs`,
//! and dynasty career totals normally include both). Season ordering is
//! by `SeasonId` ascending. The career-totals row is computed by summing
//! the per-season totals (not by re-walking games), so it's exactly
//! consistent with the per-season rows.

use nba3k_core::{GameResult, PlayerId, PlayerLine, SeasonId, TeamId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One row in a player's career table — either a single-season slice or
/// the cumulative career row. `team` is `None` for the career row (a
/// player can switch teams across seasons), otherwise the team they
/// appeared for in that season. If a player switched teams mid-season,
/// `team` is the team of their *final* game that season.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeasonAvgRow {
    pub season: SeasonId,
    pub team: Option<TeamId>,
    pub gp: u32,
    pub pts: u32,
    pub reb: u32,
    pub ast: u32,
    pub stl: u32,
    pub blk: u32,
    pub fg_made: u32,
    pub fg_att: u32,
    pub three_made: u32,
    pub three_att: u32,
    pub ft_made: u32,
    pub ft_att: u32,
    pub minutes: u32,
}

impl SeasonAvgRow {
    pub fn ppg(&self) -> f32 { per_game(self.pts, self.gp) }
    pub fn rpg(&self) -> f32 { per_game(self.reb, self.gp) }
    pub fn apg(&self) -> f32 { per_game(self.ast, self.gp) }
    pub fn spg(&self) -> f32 { per_game(self.stl, self.gp) }
    pub fn bpg(&self) -> f32 { per_game(self.blk, self.gp) }
    pub fn mpg(&self) -> f32 { per_game(self.minutes, self.gp) }

    pub fn fg_pct(&self) -> f32 { ratio(self.fg_made, self.fg_att) }
    pub fn three_pct(&self) -> f32 { ratio(self.three_made, self.three_att) }
    pub fn ft_pct(&self) -> f32 { ratio(self.ft_made, self.ft_att) }
}

fn per_game(num: u32, gp: u32) -> f32 {
    if gp == 0 { 0.0 } else { num as f32 / gp as f32 }
}

fn ratio(made: u32, att: u32) -> f32 {
    if att == 0 { 0.0 } else { made as f32 / att as f32 }
}

/// Walk `games`, sum per-season totals for `player`. Returns one row per
/// season the player appeared in, in `SeasonId` ascending order. Empty
/// vec when the player has zero box-score lines across all games.
pub fn aggregate_career(games: &[GameResult], player: PlayerId) -> Vec<SeasonAvgRow> {
    let mut by_season: BTreeMap<SeasonId, SeasonAvgRow> = BTreeMap::new();

    for g in games {
        let walk = |lines: &[PlayerLine], team: TeamId, by_season: &mut BTreeMap<_, SeasonAvgRow>| {
            for line in lines.iter().filter(|l| l.player == player) {
                let entry = by_season.entry(g.season).or_insert_with(|| SeasonAvgRow {
                    season: g.season,
                    team: Some(team),
                    gp: 0,
                    pts: 0,
                    reb: 0,
                    ast: 0,
                    stl: 0,
                    blk: 0,
                    fg_made: 0,
                    fg_att: 0,
                    three_made: 0,
                    three_att: 0,
                    ft_made: 0,
                    ft_att: 0,
                    minutes: 0,
                });
                entry.team = Some(team);
                entry.gp += 1;
                entry.pts += line.pts as u32;
                entry.reb += line.reb as u32;
                entry.ast += line.ast as u32;
                entry.stl += line.stl as u32;
                entry.blk += line.blk as u32;
                entry.fg_made += line.fg_made as u32;
                entry.fg_att += line.fg_att as u32;
                entry.three_made += line.three_made as u32;
                entry.three_att += line.three_att as u32;
                entry.ft_made += line.ft_made as u32;
                entry.ft_att += line.ft_att as u32;
                entry.minutes += line.minutes as u32;
            }
        };
        walk(&g.box_score.home_lines, g.home, &mut by_season);
        walk(&g.box_score.away_lines, g.away, &mut by_season);
    }

    by_season.into_values().collect()
}

/// Sum a slice of per-season rows into a single career-total row. `season`
/// is set to the latest season in `seasons` (callers usually display this
/// as a "career" label, not as the season). `team` is `None` since careers
/// can span multiple teams.
pub fn career_totals(seasons: &[SeasonAvgRow]) -> SeasonAvgRow {
    let last_season = seasons.last().map(|r| r.season).unwrap_or(SeasonId(0));
    let mut total = SeasonAvgRow {
        season: last_season,
        team: None,
        gp: 0,
        pts: 0,
        reb: 0,
        ast: 0,
        stl: 0,
        blk: 0,
        fg_made: 0,
        fg_att: 0,
        three_made: 0,
        three_att: 0,
        ft_made: 0,
        ft_att: 0,
        minutes: 0,
    };
    for r in seasons {
        total.gp += r.gp;
        total.pts += r.pts;
        total.reb += r.reb;
        total.ast += r.ast;
        total.stl += r.stl;
        total.blk += r.blk;
        total.fg_made += r.fg_made;
        total.fg_att += r.fg_att;
        total.three_made += r.three_made;
        total.three_att += r.three_att;
        total.ft_made += r.ft_made;
        total.ft_att += r.ft_att;
        total.minutes += r.minutes;
    }
    total
}
