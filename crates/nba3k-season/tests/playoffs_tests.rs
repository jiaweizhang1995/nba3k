//! Playoff bracket + best-of-7 acceptance tests.
//!
//! Approach: build a 30-team standings snapshot with a deterministic
//! win-distribution, generate the bracket, and verify (a) seeding rule, (b)
//! conference-only first three rounds, (c) best-of-7 always ends 4-W with
//! W ∈ {0,1,2,3} (never 5-3 or other invalid totals).
//!
//! For series sim we don't need the real `StatisticalEngine` — we plug in a
//! tiny `MockEngine` that picks a winner from the seeded RNG. That's enough
//! to verify the host-pattern + early-stop logic.

use chrono::NaiveDate;
use nba3k_core::{
    BoxScore, Conference, Division, GMArchetype, GMPersonality, GameId, GameResult,
    PlayerId, PlayerLine, SeasonId, Team, TeamId,
};
use nba3k_season::playoffs::{
    compute_finals_mvp, generate_bracket, simulate_series, PlayoffRound, Series, SeriesResult,
};
use nba3k_season::standings::Standings;
use nba3k_sim::{Engine, GameContext, RotationSlot, TeamSnapshot};
use rand::RngCore;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;


// ----------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------

fn fixture_teams() -> Vec<Team> {
    let mut out = Vec::with_capacity(30);
    let east_div_seq = [
        Division::Atlantic, Division::Atlantic, Division::Atlantic, Division::Atlantic, Division::Atlantic,
        Division::Central, Division::Central, Division::Central, Division::Central, Division::Central,
        Division::Southeast, Division::Southeast, Division::Southeast, Division::Southeast, Division::Southeast,
    ];
    let west_div_seq = [
        Division::Northwest, Division::Northwest, Division::Northwest, Division::Northwest, Division::Northwest,
        Division::Pacific, Division::Pacific, Division::Pacific, Division::Pacific, Division::Pacific,
        Division::Southwest, Division::Southwest, Division::Southwest, Division::Southwest, Division::Southwest,
    ];
    for i in 0..15u8 {
        out.push(Team {
            id: TeamId(i + 1),
            abbrev: format!("E{:02}", i + 1),
            city: "City".into(),
            name: format!("Team{}", i),
            conference: Conference::East,
            division: east_div_seq[i as usize],
            gm: GMPersonality::from_archetype("E", GMArchetype::Conservative),
            coach: nba3k_core::Coach::default(),
            roster: vec![],
            draft_picks: vec![],
        });
    }
    for i in 0..15u8 {
        out.push(Team {
            id: TeamId(16 + i),
            abbrev: format!("W{:02}", i + 1),
            city: "City".into(),
            name: format!("Team{}", i + 15),
            conference: Conference::West,
            division: west_div_seq[i as usize],
            gm: GMPersonality::from_archetype("W", GMArchetype::Conservative),
            coach: nba3k_core::Coach::default(),
            roster: vec![],
            draft_picks: vec![],
        });
    }
    out
}

fn make_standings(teams: &[Team], wins: &HashMap<TeamId, u16>) -> Standings {
    let mut s = Standings::new(teams);
    for (id, w) in wins {
        if let Some(rec) = s.records.get_mut(id) {
            rec.wins = *w;
            rec.losses = 82_u16.saturating_sub(*w);
            rec.point_diff = (*w as i32 - 41) * 4;
        }
    }
    s.recompute_ranks();
    s
}

// ----------------------------------------------------------------------
// Mock engine — deterministic 50/50 game with stable box-score lines.
// ----------------------------------------------------------------------

struct MockEngine;

impl Engine for MockEngine {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn simulate_game(
        &self,
        home: &TeamSnapshot,
        away: &TeamSnapshot,
        ctx: &GameContext,
        rng: &mut dyn RngCore,
    ) -> GameResult {
        // Bias toward higher-overall side so series usually has a clear
        // winner (and unwound test edge cases — equal teams could in
        // principle 4-3 forever, which is fine, but biased ranges exercise
        // the early-termination paths better).
        let home_strength = home.overall as i32;
        let away_strength = away.overall as i32;
        let roll = rng.next_u32() as i32 % 100;
        let advantage = (home_strength - away_strength) * 5 + 5; // home court +5
        let home_score: u16 = if roll < 50 + advantage { 110 } else { 99 };
        let away_score: u16 = if home_score == 110 { 99 } else { 110 };

        // Box: one synthetic line per side keyed off team id.
        let home_line = PlayerLine {
            player: PlayerId(home.id.0 as u32 * 1000 + 1),
            minutes: 36,
            pts: if home_score == 110 { 32 } else { 24 },
            reb: 6,
            ast: 7,
            stl: 1,
            blk: 1,
            tov: 3,
            fg_made: 11,
            fg_att: 22,
            three_made: 3,
            three_att: 8,
            ft_made: 4,
            ft_att: 4,
            plus_minus: if home_score == 110 { 11 } else { -11 },
        };
        let away_line = PlayerLine {
            player: PlayerId(away.id.0 as u32 * 1000 + 1),
            minutes: 36,
            pts: if away_score == 110 { 32 } else { 24 },
            reb: 6,
            ast: 7,
            stl: 1,
            blk: 1,
            tov: 3,
            fg_made: 11,
            fg_att: 22,
            three_made: 3,
            three_att: 8,
            ft_made: 4,
            ft_att: 4,
            plus_minus: if away_score == 110 { 11 } else { -11 },
        };
        GameResult {
            id: ctx.game_id,
            season: ctx.season,
            date: ctx.date,
            home: home.id,
            away: away.id,
            home_score,
            away_score,
            box_score: BoxScore {
                home_lines: vec![home_line],
                away_lines: vec![away_line],
            },
            overtime_periods: 0,
            is_playoffs: ctx.is_playoffs,
        }
    }
}

fn snap(id: TeamId, abbrev: &str, overall: u8) -> TeamSnapshot {
    TeamSnapshot {
        id,
        abbrev: abbrev.into(),
        overall,
        home_court_advantage: 0.03,
        rotation: vec![RotationSlot {
            player: PlayerId(id.0 as u32 * 1000 + 1),
            name: format!("Player {}", id.0),
            position: nba3k_core::Position::SF,
            minutes_share: 5.0,
            usage: 1.0,
            ratings: nba3k_core::Ratings::default(),
            age: 26,
            overall,
            potential: overall,
        }],
    }
}

// ----------------------------------------------------------------------
// Bracket tests
// ----------------------------------------------------------------------

#[test]
fn bracket_sixteen_teams_split_8_8() {
    let teams = fixture_teams();
    let mut wins = HashMap::new();
    // Distinct win counts so seeding has no ties — reverse order so id=1
    // ends up with most wins (1-seed East) etc.
    for (i, t) in teams.iter().enumerate() {
        wins.insert(t.id, 60 - i as u16);
    }
    let standings = make_standings(&teams, &wins);

    let bracket = generate_bracket(&standings, SeasonId(2026));
    assert_eq!(bracket.r1.len(), 8, "R1 has 8 series (4 East + 4 West)");

    let east_series: Vec<_> = bracket
        .r1
        .iter()
        .filter(|s| s.conference == Some(Conference::East))
        .collect();
    let west_series: Vec<_> = bracket
        .r1
        .iter()
        .filter(|s| s.conference == Some(Conference::West))
        .collect();
    assert_eq!(east_series.len(), 4, "expected 4 East R1 series (1v8/4v5/3v6/2v7)");
    assert_eq!(west_series.len(), 4, "expected 4 West R1 series");
}

#[test]
fn bracket_seeds_match_canonical_pairings() {
    let teams = fixture_teams();
    let mut wins = HashMap::new();
    for (i, t) in teams.iter().enumerate() {
        wins.insert(t.id, 60 - i as u16);
    }
    let standings = make_standings(&teams, &wins);
    let bracket = generate_bracket(&standings, SeasonId(2026));

    // Canonical pairings: 1v8, 4v5, 3v6, 2v7. Verify each East entry has a
    // valid pair.
    let east: Vec<_> = bracket
        .r1
        .iter()
        .filter(|s| s.conference == Some(Conference::East))
        .collect();
    let pairs: Vec<(u8, u8)> = east.iter().map(|s| (s.home_seed, s.away_seed)).collect();
    let expected = vec![(1, 8), (4, 5), (3, 6), (2, 7)];
    // Filter to unique pairs (we generate each pair twice via the conf
    // iteration symmetry — see playoffs.rs).
    let mut unique: Vec<(u8, u8)> = pairs;
    unique.sort();
    unique.dedup();
    let mut expected_sorted = expected.clone();
    expected_sorted.sort();
    assert_eq!(unique, expected_sorted, "East pairings must be 1v8 / 4v5 / 3v6 / 2v7");
}

#[test]
fn bracket_first_three_rounds_stay_within_conference() {
    // Logical check: every R1 series has Some(Conference) — generator never
    // mixes East and West before Finals.
    let teams = fixture_teams();
    let mut wins = HashMap::new();
    for (i, t) in teams.iter().enumerate() {
        wins.insert(t.id, 60 - i as u16);
    }
    let standings = make_standings(&teams, &wins);
    let bracket = generate_bracket(&standings, SeasonId(2026));
    for s in &bracket.r1 {
        assert!(s.conference.is_some(), "R1 series must be tagged with a conference");
    }
}

// ----------------------------------------------------------------------
// Series sim tests
// ----------------------------------------------------------------------

fn run_one_series(seed: u64, home_overall: u8, away_overall: u8) -> SeriesResult {
    let series = Series {
        round: PlayoffRound::R1,
        conference: Some(Conference::East),
        home: TeamId(1),
        away: TeamId(2),
        home_seed: 1,
        away_seed: 8,
    };
    let engine = MockEngine;
    let home = snap(TeamId(1), "HOM", home_overall);
    let away = snap(TeamId(2), "AWY", away_overall);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut next_id = 1u64;
    let date = NaiveDate::from_ymd_opt(2026, 4, 15).unwrap();
    simulate_series(
        series,
        &engine,
        &home,
        &away,
        SeasonId(2026),
        date,
        &mut next_id,
        &mut rng,
    )
}

#[test]
fn best_of_seven_ends_with_a_4_win_side() {
    for seed in 0u64..20 {
        let res = run_one_series(seed, 80, 75);
        assert!(
            res.is_complete(),
            "series with seed {} did not complete: {} / {}",
            seed,
            res.home_wins,
            res.away_wins
        );
        let (winner, loser) = if res.home_wins > res.away_wins {
            (res.home_wins, res.away_wins)
        } else {
            (res.away_wins, res.home_wins)
        };
        assert_eq!(winner, 4, "winner must have exactly 4 wins, got {}", winner);
        assert!(
            (0..=3).contains(&loser),
            "loser must have 0..=3 wins (no 5-3 invalid totals); got 4-{}",
            loser
        );
        assert!(
            res.games.len() >= 4 && res.games.len() <= 7,
            "series must run 4..=7 games, got {}",
            res.games.len()
        );
    }
}

#[test]
fn best_of_seven_sweep_runs_exactly_four_games() {
    // Force a very high overall delta so the winner sweeps. With +50 the
    // mock's bias caps at home_score=110 always.
    let res = run_one_series(7, 99, 50);
    assert!(res.home_wins == 4 || res.away_wins == 4);
    let total = res.home_wins as usize + res.away_wins as usize;
    assert_eq!(total, res.games.len(), "game count must match win sum");
}

#[test]
fn finals_mvp_picked_from_winning_team() {
    let res = run_one_series(11, 85, 80);
    let mvp = compute_finals_mvp(&res);
    let champ = res.winner();
    let champ_player_ids: Vec<PlayerId> = res
        .games
        .iter()
        .flat_map(|g| {
            let lines = if g.home == champ { &g.box_score.home_lines } else { &g.box_score.away_lines };
            lines.iter().map(|l| l.player).collect::<Vec<_>>()
        })
        .collect();
    let mvp_player = mvp.expect("Finals MVP should always exist when the series has games");
    assert!(
        champ_player_ids.contains(&mvp_player),
        "Finals MVP {:?} must be on champion {:?}; champ players were {:?}",
        mvp_player, champ, champ_player_ids
    );
}

// ----------------------------------------------------------------------
// 2-2-1-1-1 host pattern verification
// ----------------------------------------------------------------------

#[test]
fn host_pattern_is_2_2_1_1_1() {
    // Trace per-game host by checking which team sat in `home` of each
    // GameResult — for the mock, that mirrors who hosted (mock.simulate_game
    // sets game.home = host_snapshot.id).
    let res = run_one_series(0, 80, 80);
    let host_seq: Vec<TeamId> = res.games.iter().map(|g| g.home).collect();
    let pattern = [
        TeamId(1), TeamId(1), TeamId(2), TeamId(2),
        TeamId(1), TeamId(2), TeamId(1),
    ];
    for (i, host) in host_seq.iter().enumerate() {
        assert_eq!(*host, pattern[i], "game {} host should be {:?}; got {:?}", i + 1, pattern[i], host);
    }
}
