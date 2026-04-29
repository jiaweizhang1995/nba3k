//! M11-C — sim FG% calibration.
//!
//! Asserts that per-game FG/3P/FT% land in NBA-realistic bands and that
//! aggregating across many games does not blow past `[0.30, 0.65]` for FG%.
//! The QA-FIX-LOG flagged a Curry .887 season aggregate; the fix samples
//! per-game FG% from a tight Normal centered on rating-derived means, then
//! derives `fg_made = round(fg_pct * fg_att)` so a single game can never go
//! above the per-game ceiling.

use chrono::NaiveDate;
use nba3k_core::{GameId, PlayerId, Position, Ratings, SeasonId, TeamId};
use nba3k_sim::{Engine, GameContext, RotationSlot, StatisticalEngine, TeamSnapshot};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

fn shooter_ratings(three_point: u8, mid_range: u8, free_throw: u8, base: u8) -> Ratings {
    Ratings {
        close_shot: base,
        driving_layup: base,
        driving_dunk: base,
        standing_dunk: base,
        post_control: base,
        mid_range,
        three_point,
        free_throw,
        passing_accuracy: base,
        ball_handle: base,
        speed_with_ball: base,
        interior_defense: base,
        perimeter_defense: base,
        steal: base,
        block: base,
        off_reb: base,
        def_reb: base,
        speed: base,
        agility: base,
        strength: base,
        vertical: base,
    }
}

fn uniform_ratings(base: u8) -> Ratings {
    Ratings {
        close_shot: base,
        driving_layup: base,
        driving_dunk: base,
        standing_dunk: base,
        post_control: base,
        mid_range: base,
        three_point: base,
        free_throw: base,
        passing_accuracy: base,
        ball_handle: base,
        speed_with_ball: base,
        interior_defense: base,
        perimeter_defense: base,
        steal: base,
        block: base,
        off_reb: base,
        def_reb: base,
        speed: base,
        agility: base,
        strength: base,
        vertical: base,
    }
}

/// Build a team where the SG (slot 1) is the OVR-90 sniper under test.
fn team_with_sniper(id: u8, abbrev: &str, sniper: Ratings, sniper_overall: u8) -> TeamSnapshot {
    let baseline = uniform_ratings(75);
    let positions = [
        Position::PG,
        Position::SG,
        Position::SF,
        Position::PF,
        Position::C,
        Position::PG,
        Position::SG,
        Position::C,
    ];
    let minutes_share = [1.0, 1.0, 0.95, 0.85, 0.85, 0.45, 0.45, 0.45];
    let usage = [0.20, 0.28, 0.18, 0.14, 0.12, 0.04, 0.02, 0.02];

    let rotation: Vec<RotationSlot> = (0..8)
        .map(|i| {
            let (ratings, overall) = if i == 1 {
                (sniper, sniper_overall)
            } else {
                (baseline, 75)
            };
            RotationSlot {
                player: PlayerId(((id as u32) * 100) + i as u32),
                name: format!("{}{}", abbrev, i),
                position: positions[i],
                minutes_share: minutes_share[i],
                usage: usage[i],
                ratings,
                age: 27,
                overall,
                potential: overall,
            }
        })
        .collect();

    TeamSnapshot {
        id: TeamId(id),
        abbrev: abbrev.to_string(),
        overall: 78,
        home_court_advantage: 2.0,
        rotation,
    }
}

fn ctx(seed_n: u64) -> GameContext {
    GameContext {
        game_id: GameId(seed_n),
        season: SeasonId(2026),
        date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
        is_playoffs: false,
        home_back_to_back: false,
        away_back_to_back: false,
    }
}

#[test]
fn ovr90_sg_50_games_fg_calibration() {
    // OVR-90 SG sniper: three_point=92, mid_range=88, free_throw=90.
    let sniper = shooter_ratings(92, 88, 90, 80);
    let home = team_with_sniper(1, "AAA", sniper, 90);
    let away = team_with_sniper(2, "BBB", uniform_ratings(75), 75);

    let engine = StatisticalEngine::with_defaults();
    let mut rng = ChaCha8Rng::seed_from_u64(42);

    let mut fg_pcts = Vec::with_capacity(50);
    let mut three_pcts = Vec::with_capacity(50);
    let mut games_with_attempts = 0;

    for g in 0..50 {
        let r = engine.simulate_game(&home, &away, &ctx(g + 1), &mut rng);
        // Sniper is SG (rotation index 1) on the home team.
        let line = r
            .box_score
            .home_lines
            .iter()
            .find(|l| l.player == PlayerId(101))
            .expect("sniper line present");
        if line.fg_att > 0 {
            fg_pcts.push(line.fg_made as f32 / line.fg_att as f32);
            games_with_attempts += 1;
        }
        if line.three_att > 0 {
            three_pcts.push(line.three_made as f32 / line.three_att as f32);
        }

        // Hard sanity: a single game must never break NBA-realistic per-game band.
        if line.fg_att > 0 {
            let pct = line.fg_made as f32 / line.fg_att as f32;
            assert!(
                pct >= 0.20 && pct <= 0.75,
                "game {} fg_pct {} (made={}, att={}) outside [0.20, 0.75]",
                g,
                pct,
                line.fg_made,
                line.fg_att
            );
        }
    }

    assert!(
        games_with_attempts >= 40,
        "sniper had FG attempts in only {} of 50 games",
        games_with_attempts
    );

    let mean_fg: f32 = fg_pcts.iter().sum::<f32>() / fg_pcts.len() as f32;
    let mean_three: f32 = three_pcts.iter().sum::<f32>() / three_pcts.len().max(1) as f32;
    let min_fg = fg_pcts.iter().cloned().fold(1.0_f32, f32::min);
    let max_fg = fg_pcts.iter().cloned().fold(0.0_f32, f32::max);

    assert!(
        mean_fg >= 0.40 && mean_fg <= 0.55,
        "mean FG% {} outside [0.40, 0.55] (50 games, three_point=92, mid_range=88)",
        mean_fg
    );
    assert!(
        min_fg > 0.20,
        "min FG% {} <= 0.20 — single game should not crash so low",
        min_fg
    );
    assert!(
        max_fg < 0.75,
        "max FG% {} >= 0.75 — single game should not exceed band",
        max_fg
    );

    if !three_pcts.is_empty() {
        assert!(
            mean_three >= 0.30 && mean_three <= 0.45,
            "mean 3P% {} outside [0.30, 0.45]",
            mean_three
        );
    }
}

#[test]
fn season_aggregate_fg_pct_in_band_100_games() {
    // 100 games — aggregated across the whole rotation, FG% per team must be
    // realistic. This is the regression for the .887 Curry bug.
    let engine = StatisticalEngine::with_defaults();
    let home = team_with_sniper(3, "HHH", shooter_ratings(95, 90, 92, 82), 90);
    let away = team_with_sniper(4, "AAA", uniform_ratings(75), 75);
    let mut rng = ChaCha8Rng::seed_from_u64(2025);

    let mut fg_made_total: u32 = 0;
    let mut fg_att_total: u32 = 0;
    let mut sniper_made: u32 = 0;
    let mut sniper_att: u32 = 0;
    for g in 0..100 {
        let r = engine.simulate_game(&home, &away, &ctx(g + 1), &mut rng);
        for line in r
            .box_score
            .home_lines
            .iter()
            .chain(r.box_score.away_lines.iter())
        {
            fg_made_total += line.fg_made as u32;
            fg_att_total += line.fg_att as u32;
        }
        let sniper_line = r
            .box_score
            .home_lines
            .iter()
            .find(|l| l.player == PlayerId(301))
            .expect("sniper line");
        sniper_made += sniper_line.fg_made as u32;
        sniper_att += sniper_line.fg_att as u32;
    }

    let team_fg_pct = fg_made_total as f32 / fg_att_total as f32;
    assert!(
        team_fg_pct >= 0.40 && team_fg_pct <= 0.55,
        "season-aggregate league FG% {} outside [0.40, 0.55]",
        team_fg_pct
    );

    let sniper_fg_pct = sniper_made as f32 / sniper_att as f32;
    assert!(
        sniper_fg_pct >= 0.40 && sniper_fg_pct <= 0.60,
        "100-game sniper FG% {} outside [0.40, 0.60] — Curry .887 bug regression",
        sniper_fg_pct
    );
}
