//! Acceptance tests for the schedule generator, standings, and phases.

use chrono::NaiveDate;
use nba3k_core::{
    BoxScore, Conference, Division, GMArchetype, GMPersonality, GameId, GameResult, SeasonId,
    SeasonPhase, SeasonState, GameMode, Team, TeamId,
};
use nba3k_season::{
    advance_day, back_to_back_counts, games_per_team, is_after_trade_deadline, matchups,
    Schedule, Standings,
};
use std::collections::{HashMap, HashSet};

fn fixture_teams() -> Vec<Team> {
    // Real 2025-26 NBA divisions/conferences. IDs 1..=30, abbrev = real abbrev.
    let teams: &[(u8, &str, &str, &str, Conference, Division)] = &[
        (1, "BOS", "Boston", "Celtics", Conference::East, Division::Atlantic),
        (2, "BKN", "Brooklyn", "Nets", Conference::East, Division::Atlantic),
        (3, "NYK", "New York", "Knicks", Conference::East, Division::Atlantic),
        (4, "PHI", "Philadelphia", "76ers", Conference::East, Division::Atlantic),
        (5, "TOR", "Toronto", "Raptors", Conference::East, Division::Atlantic),
        (6, "CHI", "Chicago", "Bulls", Conference::East, Division::Central),
        (7, "CLE", "Cleveland", "Cavaliers", Conference::East, Division::Central),
        (8, "DET", "Detroit", "Pistons", Conference::East, Division::Central),
        (9, "IND", "Indiana", "Pacers", Conference::East, Division::Central),
        (10, "MIL", "Milwaukee", "Bucks", Conference::East, Division::Central),
        (11, "ATL", "Atlanta", "Hawks", Conference::East, Division::Southeast),
        (12, "CHA", "Charlotte", "Hornets", Conference::East, Division::Southeast),
        (13, "MIA", "Miami", "Heat", Conference::East, Division::Southeast),
        (14, "ORL", "Orlando", "Magic", Conference::East, Division::Southeast),
        (15, "WAS", "Washington", "Wizards", Conference::East, Division::Southeast),
        (16, "DEN", "Denver", "Nuggets", Conference::West, Division::Northwest),
        (17, "MIN", "Minnesota", "Timberwolves", Conference::West, Division::Northwest),
        (18, "OKC", "Oklahoma City", "Thunder", Conference::West, Division::Northwest),
        (19, "POR", "Portland", "Trail Blazers", Conference::West, Division::Northwest),
        (20, "UTA", "Utah", "Jazz", Conference::West, Division::Northwest),
        (21, "GSW", "Golden State", "Warriors", Conference::West, Division::Pacific),
        (22, "LAC", "LA", "Clippers", Conference::West, Division::Pacific),
        (23, "LAL", "Los Angeles", "Lakers", Conference::West, Division::Pacific),
        (24, "PHX", "Phoenix", "Suns", Conference::West, Division::Pacific),
        (25, "SAC", "Sacramento", "Kings", Conference::West, Division::Pacific),
        (26, "DAL", "Dallas", "Mavericks", Conference::West, Division::Southwest),
        (27, "HOU", "Houston", "Rockets", Conference::West, Division::Southwest),
        (28, "MEM", "Memphis", "Grizzlies", Conference::West, Division::Southwest),
        (29, "NOP", "New Orleans", "Pelicans", Conference::West, Division::Southwest),
        (30, "SAS", "San Antonio", "Spurs", Conference::West, Division::Southwest),
    ];

    teams
        .iter()
        .map(|(id, abbrev, city, name, conf, div)| Team {
            id: TeamId(*id),
            abbrev: (*abbrev).into(),
            city: (*city).into(),
            name: (*name).into(),
            conference: *conf,
            division: *div,
            gm: GMPersonality::from_archetype(
                format!("{} GM", abbrev),
                GMArchetype::Conservative,
            ),
            roster: vec![],
            draft_picks: vec![],
            coach: nba3k_core::Coach::default_for(abbrev),
        })
        .collect()
}

fn opponent(home: TeamId, away: TeamId, me: TeamId) -> TeamId {
    if home == me {
        away
    } else {
        home
    }
}

#[test]
fn matchup_solver_produces_1230_pairs() {
    let teams = fixture_teams();
    let pairs = matchups(&teams, 42);
    assert_eq!(pairs.len(), 1230, "expected 1230 game pairs from matchup solver");
}

#[test]
fn matchup_solver_distribution_is_nba_shape() {
    let teams = fixture_teams();
    let by_id: HashMap<TeamId, &Team> = teams.iter().map(|t| (t.id, t)).collect();
    let pairs = matchups(&teams, 42);

    // For each unordered team pair, count games (home or away combined).
    let mut counts: HashMap<(TeamId, TeamId), u32> = HashMap::new();
    for (h, a) in &pairs {
        let lo = TeamId(h.0.min(a.0));
        let hi = TeamId(h.0.max(a.0));
        *counts.entry((lo, hi)).or_insert(0) += 1;
    }

    for (a, b) in counts.keys().copied() {
        let ta = by_id[&a];
        let tb = by_id[&b];
        let n = counts[&(a, b)];
        let same_div = ta.division == tb.division;
        let same_conf = ta.conference == tb.conference;
        if same_div {
            assert_eq!(n, 4, "div opponents {} vs {} should play 4×, got {}", ta.abbrev, tb.abbrev, n);
        } else if same_conf {
            assert!(
                n == 3 || n == 4,
                "conf-non-div {} vs {} should be 3 or 4, got {}",
                ta.abbrev,
                tb.abbrev,
                n
            );
        } else {
            assert_eq!(n, 2, "inter-conf {} vs {} should play 2×, got {}", ta.abbrev, tb.abbrev, n);
        }
    }

    // Each team plays exactly 82 in the matchup list.
    let mut per_team: HashMap<TeamId, u32> = HashMap::new();
    for (h, a) in &pairs {
        *per_team.entry(*h).or_insert(0) += 1;
        *per_team.entry(*a).or_insert(0) += 1;
    }
    for t in &teams {
        assert_eq!(per_team[&t.id], 82, "{} should have 82 matchups, got {}", t.abbrev, per_team[&t.id]);
    }

    // Per team: 6 conf-non-div opponents at 4×, 4 at 3×.
    let mut per_team_conf_buckets: HashMap<TeamId, (u32, u32)> = HashMap::new();
    for (a, b) in counts.keys().copied() {
        let ta = by_id[&a];
        let tb = by_id[&b];
        let same_div = ta.division == tb.division;
        let same_conf = ta.conference == tb.conference;
        if same_conf && !same_div {
            let n = counts[&(a, b)];
            let entry_a = per_team_conf_buckets.entry(a).or_insert((0, 0));
            if n == 4 {
                entry_a.0 += 1;
            } else {
                entry_a.1 += 1;
            }
            let entry_b = per_team_conf_buckets.entry(b).or_insert((0, 0));
            if n == 4 {
                entry_b.0 += 1;
            } else {
                entry_b.1 += 1;
            }
        }
    }
    for t in &teams {
        let (fours, threes) = per_team_conf_buckets[&t.id];
        assert_eq!(
            (fours, threes),
            (6, 4),
            "{} should have 6×4-game and 4×3-game conf-non-div opponents, got ({}, {})",
            t.abbrev,
            fours,
            threes
        );
    }
}

#[test]
fn schedule_has_exactly_1230_games() {
    let teams = fixture_teams();
    let schedule = Schedule::generate(SeasonId(2026), 42, &teams);
    assert_eq!(schedule.games.len(), 1230);
}

#[test]
fn every_team_plays_exactly_82_games() {
    let teams = fixture_teams();
    let schedule = Schedule::generate(SeasonId(2026), 42, &teams);
    let counts = games_per_team(&schedule);
    for t in &teams {
        assert_eq!(
            counts.get(&t.id).copied().unwrap_or(0),
            82,
            "{} should play 82 games, got {}",
            t.abbrev,
            counts.get(&t.id).copied().unwrap_or(0)
        );
    }
}

#[test]
fn no_team_plays_itself() {
    let teams = fixture_teams();
    let schedule = Schedule::generate(SeasonId(2026), 42, &teams);
    for g in &schedule.games {
        assert_ne!(g.home, g.away, "self-game in schedule");
    }
}

#[test]
fn at_most_one_game_per_team_per_day() {
    let teams = fixture_teams();
    let schedule = Schedule::generate(SeasonId(2026), 42, &teams);
    let mut team_day_seen: HashSet<(TeamId, NaiveDate)> = HashSet::new();
    for g in &schedule.games {
        assert!(
            team_day_seen.insert((g.home, g.date)),
            "team {} double-booked on {}",
            g.home,
            g.date
        );
        assert!(
            team_day_seen.insert((g.away, g.date)),
            "team {} double-booked on {}",
            g.away,
            g.date
        );
    }
}

#[test]
fn season_window_is_oct21_to_apr12() {
    let teams = fixture_teams();
    let schedule = Schedule::generate(SeasonId(2026), 42, &teams);
    let start = NaiveDate::from_ymd_opt(2025, 10, 21).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 4, 12).unwrap();
    assert_eq!(schedule.start, start);
    assert_eq!(schedule.end, end);
    for g in &schedule.games {
        assert!(g.date >= start, "game before season start: {}", g.date);
        assert!(g.date <= end, "game after season end: {}", g.date);
    }
}

#[test]
fn back_to_back_counts_are_in_loose_nba_range() {
    let teams = fixture_teams();
    let schedule = Schedule::generate(SeasonId(2026), 42, &teams);
    let b2b = back_to_back_counts(&schedule);
    for t in &teams {
        let n = *b2b.get(&t.id).unwrap_or(&0);
        assert!(
            (10..=18).contains(&n),
            "{} has {} back-to-backs, outside loose [10, 18] range",
            t.abbrev,
            n
        );
    }
}

#[test]
fn schedule_is_deterministic_for_seed() {
    let teams = fixture_teams();
    let s1 = Schedule::generate(SeasonId(2026), 42, &teams);
    let s2 = Schedule::generate(SeasonId(2026), 42, &teams);
    assert_eq!(s1.games.len(), s2.games.len());
    for (g1, g2) in s1.games.iter().zip(s2.games.iter()) {
        assert_eq!(g1.date, g2.date);
        assert_eq!(g1.home, g2.home);
        assert_eq!(g1.away, g2.away);
    }
}

fn dummy_game(id: u64, home: TeamId, away: TeamId, hs: u16, as_: u16) -> GameResult {
    GameResult {
        id: GameId(id),
        season: SeasonId(2026),
        date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
        home,
        away,
        home_score: hs,
        away_score: as_,
        box_score: BoxScore {
            home_lines: vec![],
            away_lines: vec![],
        },
        overtime_periods: 0,
        is_playoffs: false,
    }
}

#[test]
fn standings_record_game_result_updates_wins_losses() {
    let teams = fixture_teams();
    let mut s = Standings::new(&teams);
    let bos = TeamId(1);
    let lal = TeamId(23);
    s.record_game_result(&dummy_game(1, bos, lal, 110, 100));
    assert_eq!(s.records[&bos].wins, 1);
    assert_eq!(s.records[&bos].losses, 0);
    assert_eq!(s.records[&lal].wins, 0);
    assert_eq!(s.records[&lal].losses, 1);
    assert_eq!(s.records[&bos].point_diff, 10);
    assert_eq!(s.records[&lal].point_diff, -10);
    // BOS is East, LAL is West → not same conference, no conf splits.
    assert_eq!(s.records[&bos].conference_wins, 0);
}

#[test]
fn standings_track_division_and_conference_splits() {
    let teams = fixture_teams();
    let mut s = Standings::new(&teams);
    // BOS (Atlantic East) over PHI (Atlantic East): same div, same conf.
    s.record_game_result(&dummy_game(1, TeamId(1), TeamId(4), 100, 95));
    assert_eq!(s.records[&TeamId(1)].division_wins, 1);
    assert_eq!(s.records[&TeamId(1)].conference_wins, 1);
    assert_eq!(s.records[&TeamId(4)].division_losses, 1);
    assert_eq!(s.records[&TeamId(4)].conference_losses, 1);

    // BOS over CHI (Central East): same conf, different div.
    s.record_game_result(&dummy_game(2, TeamId(1), TeamId(6), 100, 90));
    assert_eq!(s.records[&TeamId(1)].division_wins, 1);
    assert_eq!(s.records[&TeamId(1)].conference_wins, 2);
}

#[test]
fn standings_tiebreaker_is_deterministic() {
    let teams = fixture_teams();
    let mut s = Standings::new(&teams);
    // Two East teams tied 1-1; head-to-head decides.
    let a = TeamId(1);
    let b = TeamId(3);
    s.record_game_result(&dummy_game(1, a, b, 100, 95)); // a wins
    s.record_game_result(&dummy_game(2, b, a, 110, 100)); // b wins
    s.record_game_result(&dummy_game(3, a, b, 120, 110)); // a wins again → a leads h2h 2-1

    s.recompute_ranks();

    let r_a = s.records[&a].conf_rank;
    let r_b = s.records[&b].conf_rank;
    assert!(
        r_a < r_b,
        "team {:?} should rank above {:?} via h2h, got {} vs {}",
        a,
        b,
        r_a,
        r_b
    );
}

#[test]
fn phase_preseason_advances_to_regular_after_day_seven() {
    let teams = fixture_teams();
    let schedule = Schedule::generate(SeasonId(2026), 42, &teams);
    let standings = Standings::new(&teams);
    let mut state = SeasonState {
        season: SeasonId(2026),
        phase: SeasonPhase::PreSeason,
        day: 0,
        user_team: TeamId(1),
        mode: GameMode::Standard,
        rng_seed: 42,
    };
    // Day 0..=6 stays in preseason.
    for d in 0..=6 {
        state.day = d;
        assert_eq!(advance_day(&state, &schedule, &standings), SeasonPhase::PreSeason);
    }
    // Day 7+ flips to Regular.
    state.day = 7;
    assert_eq!(advance_day(&state, &schedule, &standings), SeasonPhase::Regular);
}

#[test]
fn phase_regular_advances_to_playoffs_after_82_games_each() {
    let teams = fixture_teams();
    let schedule = Schedule::generate(SeasonId(2026), 42, &teams);
    let mut standings = Standings::new(&teams);
    let state = SeasonState {
        season: SeasonId(2026),
        phase: SeasonPhase::Regular,
        day: 100,
        user_team: TeamId(1),
        mode: GameMode::Standard,
        rng_seed: 42,
    };
    // Mid-season: not all 82 played → stays Regular.
    assert_eq!(advance_day(&state, &schedule, &standings), SeasonPhase::Regular);

    // Simulate every game: home team always wins by 1.
    for g in &schedule.games {
        standings.record_game_result(&dummy_game(g.id.0, g.home, g.away, 101, 100));
    }
    assert_eq!(advance_day(&state, &schedule, &standings), SeasonPhase::Playoffs);
}

#[test]
fn trade_deadline_check_is_calendar_based() {
    let before = NaiveDate::from_ymd_opt(2026, 2, 4).unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 2, 5).unwrap();
    let after = NaiveDate::from_ymd_opt(2026, 2, 6).unwrap();
    assert!(!is_after_trade_deadline(before));
    assert!(!is_after_trade_deadline(day));
    assert!(is_after_trade_deadline(after));
}

#[test]
fn opponent_helper_works() {
    // sanity check on the test helper itself.
    assert_eq!(opponent(TeamId(1), TeamId(2), TeamId(1)), TeamId(2));
    assert_eq!(opponent(TeamId(1), TeamId(2), TeamId(2)), TeamId(1));
}
