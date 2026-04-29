//! M13-A — injury system smoke tests.
//!
//! 1. A 100-game sim with seed 7 produces ≥ 5 distinct injuries (loose).
//! 2. `tick_injury` monotonically reduces `games_remaining` to 0.

use chrono::NaiveDate;
use nba3k_core::{
    GameId, InjurySeverity, InjuryStatus, PlayerId, Position, Ratings, SeasonId, TeamId,
};
use nba3k_sim::{
    roll_injuries_from_box, tick_injury, Engine, GameContext, RotationSlot, StatisticalEngine,
    TeamSnapshot,
};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

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

fn fair_team(id: u8, abbrev: &str, base: u8) -> TeamSnapshot {
    let ratings = uniform_ratings(base);
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
    let minutes_share = [1.0, 0.95, 0.95, 0.85, 0.85, 0.45, 0.45, 0.50];
    let usage = [0.22, 0.20, 0.18, 0.14, 0.14, 0.05, 0.04, 0.03];
    let rotation: Vec<RotationSlot> = (0..8)
        .map(|i| RotationSlot {
            player: PlayerId(((id as u32) * 100) + i as u32),
            name: format!("{}{}", abbrev, i),
            position: positions[i],
            minutes_share: minutes_share[i],
            usage: usage[i],
            ratings,
            age: 27,
            overall: base,
            potential: base,
        })
        .collect();
    TeamSnapshot {
        id: TeamId(id),
        abbrev: abbrev.to_string(),
        overall: base,
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
fn one_hundred_game_sim_seed_seven_yields_five_plus_injuries() {
    let engine = StatisticalEngine::with_defaults();
    let home = fair_team(1, "AAA", 78);
    let away = fair_team(2, "BBB", 78);
    let mut rng = ChaCha8Rng::seed_from_u64(7);

    let mut total = 0usize;
    for g in 0..100u64 {
        let result = engine.simulate_game(&home, &away, &ctx(g + 1), &mut rng);
        let new_injuries = roll_injuries_from_box(&result.box_score, &mut rng);
        total += new_injuries.len();
    }
    assert!(
        total >= 5,
        "expected ≥ 5 injuries across 100 games, got {}",
        total
    );
}

#[test]
fn tick_injury_decrements_to_zero() {
    let mut status = InjuryStatus {
        description: "ankle sprain".into(),
        games_remaining: 4,
        severity: InjurySeverity::DayToDay,
    };
    let mut last = status.games_remaining;
    let mut steps = 0;
    while let Some(next) = tick_injury(&status) {
        assert!(next.games_remaining < last, "decrement non-monotonic");
        last = next.games_remaining;
        status = next;
        steps += 1;
        if steps > 10 {
            panic!("did not converge");
        }
    }
    // After the final tick (when games_remaining was 1), tick_injury returns
    // None — the slot is cleared. Steps taken: 4 - 1 = 3 (4→3→2→1, then None).
    assert_eq!(
        steps, 3,
        "expected exactly 3 ticks before clear, got {}",
        steps
    );
}
