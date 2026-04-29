//! Tests for the dynasty career-stats aggregator.

use chrono::NaiveDate;
use nba3k_core::{BoxScore, GameId, GameResult, PlayerId, PlayerLine, SeasonId, TeamId};
use nba3k_season::career::{aggregate_career, career_totals};

// ----------------------------------------------------------------------
// Builders
// ----------------------------------------------------------------------

fn line(player: u32, pts: u8, reb: u8, ast: u8) -> PlayerLine {
    line_full(player, pts, reb, ast, 0, 0, 28, 4, 8, 1, 3, 2, 2)
}

#[allow(clippy::too_many_arguments)]
fn line_full(
    player: u32,
    pts: u8,
    reb: u8,
    ast: u8,
    stl: u8,
    blk: u8,
    minutes: u8,
    fg_made: u8,
    fg_att: u8,
    three_made: u8,
    three_att: u8,
    ft_made: u8,
    ft_att: u8,
) -> PlayerLine {
    PlayerLine {
        player: PlayerId(player),
        minutes,
        pts,
        reb,
        ast,
        stl,
        blk,
        tov: 1,
        fg_made,
        fg_att,
        three_made,
        three_att,
        ft_made,
        ft_att,
        plus_minus: 0,
    }
}

fn game(
    id: u64,
    season: u16,
    home: u8,
    away: u8,
    home_lines: Vec<PlayerLine>,
    away_lines: Vec<PlayerLine>,
) -> GameResult {
    GameResult {
        id: GameId(id),
        season: SeasonId(season),
        date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        home: TeamId(home),
        away: TeamId(away),
        home_score: 110,
        away_score: 100,
        box_score: BoxScore {
            home_lines,
            away_lines,
        },
        overtime_periods: 0,
        is_playoffs: false,
    }
}

// ----------------------------------------------------------------------
// 1. Two-season aggregate — assert PPG = total_pts / total_gp.
// ----------------------------------------------------------------------

#[test]
fn two_season_ppg_matches_total_pts_over_total_gp() {
    let player = PlayerId(7);
    // Season 2026: 3 games on team 1, 30/25/20 pts.
    let s1 = vec![
        game(1, 2026, 1, 2, vec![line(7, 30, 5, 6)], vec![]),
        game(2, 2026, 1, 3, vec![line(7, 25, 4, 7)], vec![]),
        game(3, 2026, 2, 1, vec![], vec![line(7, 20, 6, 5)]),
    ];
    // Season 2027: 2 games on team 1, 28/22 pts.
    let s2 = vec![
        game(4, 2027, 1, 4, vec![line(7, 28, 5, 8)], vec![]),
        game(5, 2027, 1, 2, vec![line(7, 22, 6, 4)], vec![]),
    ];
    let mut games = s1;
    games.extend(s2);

    let rows = aggregate_career(&games, player);
    assert_eq!(rows.len(), 2);

    let career = career_totals(&rows);
    assert_eq!(career.gp, 5);
    assert_eq!(career.pts, 30 + 25 + 20 + 28 + 22);

    let manual_ppg = career.pts as f32 / career.gp as f32;
    assert!((career.ppg() - manual_ppg).abs() < 1e-6);

    // Sanity: per-season PPG also matches.
    let r0 = &rows[0];
    assert_eq!(r0.gp, 3);
    assert_eq!(r0.pts, 75);
    assert!((r0.ppg() - 25.0).abs() < 1e-6);
}

// ----------------------------------------------------------------------
// 2. Season ordering — rows must come out by SeasonId asc.
// ----------------------------------------------------------------------

#[test]
fn seasons_returned_ascending_even_when_input_unsorted() {
    let player = PlayerId(11);
    // Feed games out of order — 2028 first, then 2026, then 2027.
    let games = vec![
        game(10, 2028, 1, 2, vec![line(11, 10, 1, 1)], vec![]),
        game(11, 2026, 1, 2, vec![line(11, 30, 1, 1)], vec![]),
        game(12, 2027, 1, 2, vec![line(11, 20, 1, 1)], vec![]),
    ];
    let rows = aggregate_career(&games, player);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].season, SeasonId(2026));
    assert_eq!(rows[1].season, SeasonId(2027));
    assert_eq!(rows[2].season, SeasonId(2028));
}

// ----------------------------------------------------------------------
// 3. Empty case — player never appears.
// ----------------------------------------------------------------------

#[test]
fn no_games_for_player_returns_empty_with_zero_career() {
    let player = PlayerId(99);
    let games = vec![
        game(
            20,
            2026,
            1,
            2,
            vec![line(7, 30, 5, 5)],
            vec![line(8, 20, 3, 3)],
        ),
        game(21, 2026, 1, 3, vec![line(7, 28, 4, 6)], vec![]),
    ];
    let rows = aggregate_career(&games, player);
    assert!(rows.is_empty());

    let career = career_totals(&rows);
    assert_eq!(career.gp, 0);
    assert_eq!(career.pts, 0);
    assert_eq!(career.fg_made, 0);
    assert_eq!(career.fg_att, 0);
    // Per-game and pct accessors safe-divide.
    assert_eq!(career.ppg(), 0.0);
    assert_eq!(career.fg_pct(), 0.0);
    assert_eq!(career.three_pct(), 0.0);
    assert_eq!(career.ft_pct(), 0.0);
}

// ----------------------------------------------------------------------
// 4. Bonus: TM column reflects the team the player appeared for.
// Lead's spec calls out "player who switched teams shows the correct TM
// column per season"; this verifies the cross-season case.
// ----------------------------------------------------------------------

#[test]
fn team_per_season_reflects_appearance_team() {
    let player = PlayerId(42);
    // 2026 → played on TeamId(5). 2027 → played on TeamId(9).
    let games = vec![
        game(30, 2026, 5, 6, vec![line(42, 22, 4, 5)], vec![]),
        game(31, 2026, 5, 7, vec![line(42, 18, 3, 6)], vec![]),
        game(32, 2027, 9, 6, vec![line(42, 25, 5, 5)], vec![]),
    ];
    let rows = aggregate_career(&games, player);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].team, Some(TeamId(5)));
    assert_eq!(rows[1].team, Some(TeamId(9)));
    assert_eq!(rows[0].gp, 2);
    assert_eq!(rows[1].gp, 1);
}

// ----------------------------------------------------------------------
// 5. Bonus: shooting splits aggregate correctly.
// ----------------------------------------------------------------------

#[test]
fn shooting_splits_sum_across_games() {
    let player = PlayerId(3);
    // Two games: 5/10 FG + 6/12 FG = 11/22 (.500).
    // 2/5 3P + 3/8 3P = 5/13 ≈ .385.
    // 4/4 FT + 5/6 FT = 9/10 (.900).
    let games = vec![
        game(
            40,
            2026,
            1,
            2,
            vec![line_full(3, 16, 4, 3, 1, 0, 30, 5, 10, 2, 5, 4, 4)],
            vec![],
        ),
        game(
            41,
            2026,
            1,
            3,
            vec![line_full(3, 20, 5, 4, 0, 1, 32, 6, 12, 3, 8, 5, 6)],
            vec![],
        ),
    ];
    let rows = aggregate_career(&games, player);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.fg_made, 11);
    assert_eq!(r.fg_att, 22);
    assert!((r.fg_pct() - 0.5).abs() < 1e-6);
    assert_eq!(r.three_made, 5);
    assert_eq!(r.three_att, 13);
    assert!((r.three_pct() - (5.0 / 13.0)).abs() < 1e-6);
    assert_eq!(r.ft_made, 9);
    assert_eq!(r.ft_att, 10);
    assert!((r.ft_pct() - 0.9).abs() < 1e-6);
}
