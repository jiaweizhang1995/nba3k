//! Awards engine acceptance tests. Builds synthetic season aggregates
//! (no real games) and verifies the composite + ballot machinery picks the
//! correct winner / produces valid lineup compositions.
//!
//! Why no real-game fixtures? The MVP/DPOY composites read from
//! `PlayerSeason` totals — once we've verified `aggregate_season` (covered
//! by `aggregate_smoke` below), every other test can build aggregates
//! directly and skip the sim.

use chrono::NaiveDate;
use nba3k_core::{
    BoxScore, Conference, Division, GMArchetype, GMPersonality, GameId, GameResult, PlayerId,
    PlayerLine, Position, SeasonId, Team, TeamId,
};
use nba3k_season::awards::{
    aggregate_season, compute_all_defensive, compute_all_nba, compute_all_star, compute_dpoy,
    compute_mip, compute_mvp, compute_roy, compute_sixth_man, AwardKind, PlayerSeason,
    SeasonAggregate,
};
use nba3k_season::standings::Standings;
use std::collections::HashMap;

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

fn fixture_teams() -> Vec<Team> {
    let mut out = Vec::with_capacity(30);
    let east_div_seq = [
        Division::Atlantic,
        Division::Atlantic,
        Division::Atlantic,
        Division::Atlantic,
        Division::Atlantic,
        Division::Central,
        Division::Central,
        Division::Central,
        Division::Central,
        Division::Central,
        Division::Southeast,
        Division::Southeast,
        Division::Southeast,
        Division::Southeast,
        Division::Southeast,
    ];
    let west_div_seq = [
        Division::Northwest,
        Division::Northwest,
        Division::Northwest,
        Division::Northwest,
        Division::Northwest,
        Division::Pacific,
        Division::Pacific,
        Division::Pacific,
        Division::Pacific,
        Division::Pacific,
        Division::Southwest,
        Division::Southwest,
        Division::Southwest,
        Division::Southwest,
        Division::Southwest,
    ];
    for i in 0..15 {
        out.push(Team {
            id: TeamId(i + 1),
            abbrev: format!("E{:02}", i + 1),
            city: format!("EastCity{}", i),
            name: format!("Team{}", i),
            conference: Conference::East,
            division: east_div_seq[i as usize],
            gm: GMPersonality::from_archetype("E", GMArchetype::Conservative),
            coach: nba3k_core::Coach::default(),
            roster: vec![],
            draft_picks: vec![],
        });
    }
    for i in 0..15 {
        out.push(Team {
            id: TeamId(16 + i),
            abbrev: format!("W{:02}", i + 1),
            city: format!("WestCity{}", i),
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
            rec.point_diff = (*w as i32 - 41) * 3;
        }
    }
    s.recompute_ranks();
    s
}

fn season_with(
    player: PlayerId,
    team: TeamId,
    games: u16,
    pts: u32,
    reb: u32,
    ast: u32,
    stl: u32,
    blk: u32,
    tov: u32,
    minutes: u32,
) -> PlayerSeason {
    PlayerSeason {
        player,
        team: Some(team),
        games,
        minutes,
        pts,
        reb,
        ast,
        stl,
        blk,
        tov,
        fg_made: 0,
        fg_att: 0,
        three_made: 0,
        three_att: 0,
        ft_made: 0,
        ft_att: 0,
    }
}

// ----------------------------------------------------------------------
// Aggregation smoke
// ----------------------------------------------------------------------

#[test]
fn aggregate_smoke_one_game() {
    let line_home = PlayerLine {
        player: PlayerId(1),
        minutes: 35,
        pts: 30,
        reb: 5,
        ast: 8,
        stl: 2,
        blk: 1,
        tov: 3,
        fg_made: 11,
        fg_att: 22,
        three_made: 4,
        three_att: 9,
        ft_made: 4,
        ft_att: 4,
        plus_minus: 6,
    };
    let line_away = PlayerLine {
        player: PlayerId(2),
        minutes: 36,
        pts: 28,
        reb: 4,
        ast: 7,
        stl: 1,
        blk: 0,
        tov: 2,
        fg_made: 10,
        fg_att: 20,
        three_made: 3,
        three_att: 8,
        ft_made: 5,
        ft_att: 6,
        plus_minus: -6,
    };
    let g = GameResult {
        id: GameId(1),
        season: SeasonId(2026),
        date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        home: TeamId(1),
        away: TeamId(2),
        home_score: 110,
        away_score: 104,
        box_score: BoxScore {
            home_lines: vec![line_home],
            away_lines: vec![line_away],
        },
        overtime_periods: 0,
        is_playoffs: false,
    };
    let agg = aggregate_season(&[g]);
    let p1 = agg.by_player.get(&PlayerId(1)).expect("p1 line");
    assert_eq!(p1.games, 1);
    assert_eq!(p1.pts, 30);
    assert_eq!(p1.team, Some(TeamId(1)));
}

// ----------------------------------------------------------------------
// MVP — Luka beats Embiid (50-win team gates Embiid's bigger box)
// ----------------------------------------------------------------------

#[test]
fn mvp_luka_shape_beats_embiid_shape() {
    let teams = fixture_teams();
    // Luka: 32 PPG / 8 RPG / 8 APG, 50-win team in East.
    // Embiid: 28 PPG / 11 RPG / 5 APG, 35-win team in West.
    // (35 wins > 30-win gate so Embiid still scores; Luka should still win
    // because (a) more wins gate his composite higher, (b) bigger AST/STL.)
    let luka_id = PlayerId(101);
    let embiid_id = PlayerId(102);
    let luka_team = TeamId(1);
    let embiid_team = TeamId(20);

    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    by_player.insert(
        luka_id,
        season_with(
            luka_id,
            luka_team,
            70,
            32 * 70,
            8 * 70,
            8 * 70,
            1 * 70,
            1 * 70,
            4 * 70,
            36 * 70,
        ),
    );
    by_player.insert(
        embiid_id,
        season_with(
            embiid_id,
            embiid_team,
            65,
            28 * 65,
            11 * 65,
            5 * 65,
            1 * 65,
            2 * 65,
            3 * 65,
            35 * 65,
        ),
    );
    let mut team_drtg: HashMap<TeamId, f32> = HashMap::new();
    team_drtg.insert(luka_team, 110.0);
    team_drtg.insert(embiid_team, 110.0);
    let agg = SeasonAggregate {
        by_player,
        team_drtg,
    };

    let mut wins = HashMap::new();
    wins.insert(luka_team, 50);
    wins.insert(embiid_team, 35);
    // Fill the rest at 41 wins so standings is well-formed.
    for t in &teams {
        wins.entry(t.id).or_insert(41);
    }
    let standings = make_standings(&teams, &wins);

    let mvp = compute_mvp(&agg, &standings, SeasonId(2026));
    assert_eq!(mvp.kind, AwardKind::MVP);
    assert_eq!(
        mvp.winner,
        Some(luka_id),
        "Luka shape should beat Embiid shape; ballot was {:?}",
        mvp.ballot
    );
}

// ----------------------------------------------------------------------
// MVP — sub-30-win team scorer scores 0 (gate cuts noise)
// ----------------------------------------------------------------------

#[test]
fn mvp_below_30_win_gate_zeroes_score() {
    let teams = fixture_teams();
    let scorer = PlayerId(201);
    let scorer_team = TeamId(15);
    let star = PlayerId(202);
    let star_team = TeamId(16);

    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    // Tank-team monster: 35 PPG, 10 APG but team won only 22 games → gate=0.
    by_player.insert(
        scorer,
        season_with(
            scorer,
            scorer_team,
            70,
            35 * 70,
            6 * 70,
            10 * 70,
            70,
            70,
            5 * 70,
            36 * 70,
        ),
    );
    // Quietly elite player on a 60-win team — should win MVP because tank
    // monster's gate zeroes him out.
    by_player.insert(
        star,
        season_with(
            star,
            star_team,
            70,
            25 * 70,
            7 * 70,
            7 * 70,
            70,
            70,
            3 * 70,
            33 * 70,
        ),
    );
    let agg = SeasonAggregate {
        by_player,
        team_drtg: HashMap::new(),
    };

    let mut wins = HashMap::new();
    wins.insert(scorer_team, 22);
    wins.insert(star_team, 60);
    for t in &teams {
        wins.entry(t.id).or_insert(41);
    }
    let standings = make_standings(&teams, &wins);

    let mvp = compute_mvp(&agg, &standings, SeasonId(2026));
    assert_eq!(
        mvp.winner,
        Some(star),
        "tank-team monster should be gated; ballot {:?}",
        mvp.ballot
    );
}

// ----------------------------------------------------------------------
// DPOY — Wemby shape (high BLK/STL on top-5 defense) > average wing
// ----------------------------------------------------------------------

#[test]
fn dpoy_wemby_shape_beats_average_wing() {
    let wemby = PlayerId(301);
    let wing = PlayerId(302);
    let wemby_team = TeamId(1);
    let wing_team = TeamId(2);

    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    by_player.insert(
        wemby,
        season_with(
            wemby,
            wemby_team,
            70,
            22 * 70,
            11 * 70,
            4 * 70,
            1 * 70,
            4 * 70,
            3 * 70,
            33 * 70,
        ),
    );
    by_player.insert(
        wing,
        season_with(
            wing,
            wing_team,
            70,
            18 * 70,
            4 * 70,
            4 * 70,
            1 * 70,
            1 * 70,
            2 * 70,
            32 * 70,
        ),
    );
    // Wemby on top-5 defense (low DRtg); wing on average defense.
    let mut team_drtg: HashMap<TeamId, f32> = HashMap::new();
    team_drtg.insert(wemby_team, 102.0);
    team_drtg.insert(wing_team, 113.0);
    let agg = SeasonAggregate {
        by_player,
        team_drtg,
    };

    let dpoy = compute_dpoy(&agg, SeasonId(2026));
    assert_eq!(
        dpoy.winner,
        Some(wemby),
        "Wemby shape should win DPOY; ballot was {:?}",
        dpoy.ballot
    );
}

// ----------------------------------------------------------------------
// ROY — only rookie pool considered
// ----------------------------------------------------------------------

#[test]
fn roy_filters_to_rookies_only() {
    let rookie = PlayerId(401);
    let veteran_scorer = PlayerId(402);
    let team = TeamId(1);

    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    by_player.insert(
        rookie,
        season_with(
            rookie,
            team,
            60,
            18 * 60,
            5 * 60,
            4 * 60,
            60,
            60,
            2 * 60,
            30 * 60,
        ),
    );
    by_player.insert(
        veteran_scorer,
        season_with(
            veteran_scorer,
            team,
            70,
            30 * 70,
            5 * 70,
            5 * 70,
            70,
            70,
            3 * 70,
            35 * 70,
        ),
    );
    let agg = SeasonAggregate {
        by_player,
        team_drtg: HashMap::new(),
    };

    let roy = compute_roy(&agg, SeasonId(2026), |id| id == rookie);
    assert_eq!(roy.winner, Some(rookie));
    assert!(
        roy.ballot.iter().all(|(p, _)| *p == rookie),
        "ROY ballot should only contain rookies; got {:?}",
        roy.ballot
    );
}

// ----------------------------------------------------------------------
// Sixth Man — 18..28 mpg gate
// ----------------------------------------------------------------------

#[test]
fn sixth_man_only_rotation_non_starters() {
    let sixth = PlayerId(501);
    let starter = PlayerId(502);
    let dnp = PlayerId(503);
    let team = TeamId(1);

    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    // 24 mpg, 18 ppg.
    by_player.insert(
        sixth,
        season_with(
            sixth,
            team,
            70,
            18 * 70,
            4 * 70,
            5 * 70,
            70,
            70,
            2 * 70,
            24 * 70,
        ),
    );
    // Starter at 36 mpg — gated out.
    by_player.insert(
        starter,
        season_with(
            starter,
            team,
            70,
            28 * 70,
            6 * 70,
            6 * 70,
            70,
            70,
            3 * 70,
            36 * 70,
        ),
    );
    // DNP-level — gated out.
    by_player.insert(
        dnp,
        season_with(dnp, team, 70, 4 * 70, 1 * 70, 1 * 70, 70, 70, 70, 8 * 70),
    );
    let agg = SeasonAggregate {
        by_player,
        team_drtg: HashMap::new(),
    };

    let sm = compute_sixth_man(&agg, SeasonId(2026));
    assert_eq!(sm.winner, Some(sixth));
}

// ----------------------------------------------------------------------
// MIP — biggest YoY improvement wins
// ----------------------------------------------------------------------

#[test]
fn mip_picks_biggest_improver() {
    let breakout = PlayerId(601);
    let steady = PlayerId(602);
    let team = TeamId(1);

    let mut prev: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    prev.insert(
        breakout,
        season_with(
            breakout,
            team,
            60,
            8 * 60,
            3 * 60,
            2 * 60,
            60,
            60,
            60,
            18 * 60,
        ),
    );
    prev.insert(
        steady,
        season_with(
            steady,
            team,
            70,
            22 * 70,
            5 * 70,
            5 * 70,
            70,
            70,
            2 * 70,
            33 * 70,
        ),
    );
    let prev_agg = SeasonAggregate {
        by_player: prev,
        team_drtg: HashMap::new(),
    };

    let mut curr: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    curr.insert(
        breakout,
        season_with(
            breakout,
            team,
            70,
            22 * 70,
            7 * 70,
            5 * 70,
            70,
            70,
            2 * 70,
            32 * 70,
        ),
    );
    curr.insert(
        steady,
        season_with(
            steady,
            team,
            70,
            23 * 70,
            5 * 70,
            5 * 70,
            70,
            70,
            2 * 70,
            33 * 70,
        ),
    );
    let curr_agg = SeasonAggregate {
        by_player: curr,
        team_drtg: HashMap::new(),
    };

    let mip = compute_mip(&curr_agg, &prev_agg, SeasonId(2026));
    assert_eq!(mip.winner, Some(breakout));
}

// ----------------------------------------------------------------------
// All-NBA — 15 players, positional balance per team
// ----------------------------------------------------------------------

#[test]
fn all_nba_three_teams_positional_balance() {
    let teams = fixture_teams();
    // Build a deep candidate pool: 6 PG/SG, 6 SF/PF, 3 C — exactly enough
    // to fill 3 teams with the 2G+2F+1C shape.
    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    let mut wins = HashMap::new();
    let mut positions: HashMap<PlayerId, Position> = HashMap::new();
    let positions_seq: Vec<Position> = vec![
        Position::PG,
        Position::PG,
        Position::PG,
        Position::SG,
        Position::SG,
        Position::SG,
        Position::SF,
        Position::SF,
        Position::SF,
        Position::PF,
        Position::PF,
        Position::PF,
        Position::C,
        Position::C,
        Position::C,
    ];
    for (i, pos) in positions_seq.iter().enumerate() {
        let pid = PlayerId(700 + i as u32);
        // Spread teams across 15 different teams (1..=15) — guarantees ranks 1..15.
        let team = TeamId((i as u8) + 1);
        wins.insert(team, (60 - i as u16).max(35));
        positions.insert(pid, *pos);
        // Box-score scaled to candidate index so ballot has a clear order.
        let pts_base = (35 - i) as u32 * 70;
        by_player.insert(
            pid,
            season_with(
                pid,
                team,
                70,
                pts_base,
                5 * 70,
                5 * 70,
                70,
                70,
                2 * 70,
                33 * 70,
            ),
        );
    }
    for t in &teams {
        wins.entry(t.id).or_insert(35);
    }
    let standings = make_standings(&teams, &wins);
    let agg = SeasonAggregate {
        by_player,
        team_drtg: HashMap::new(),
    };

    let all_nba = compute_all_nba(&agg, &standings, SeasonId(2026), |pid| {
        positions.get(&pid).copied()
    });
    let total: usize = all_nba.iter().map(|t| t.ballot.len()).sum();
    assert_eq!(total, 15, "All-NBA must have 15 players across 3 teams");
    for (i, team) in all_nba.iter().enumerate() {
        assert_eq!(
            team.ballot.len(),
            5,
            "All-NBA team {} must have 5 players",
            i + 1
        );
        let mut g = 0;
        let mut f = 0;
        let mut c = 0;
        for (pid, _) in &team.ballot {
            match positions[pid] {
                Position::PG | Position::SG => g += 1,
                Position::SF | Position::PF => f += 1,
                Position::C => c += 1,
            }
        }
        assert_eq!(
            (g, f, c),
            (2, 2, 1),
            "team {} positional shape {:?}",
            i + 1,
            (g, f, c)
        );
    }
}

// ----------------------------------------------------------------------
// All-Defensive — 10 players, 2 teams
// ----------------------------------------------------------------------

#[test]
fn all_defensive_two_teams_positional_balance() {
    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    let mut team_drtg: HashMap<TeamId, f32> = HashMap::new();
    let mut positions: HashMap<PlayerId, Position> = HashMap::new();
    let positions_seq: Vec<Position> = vec![
        Position::PG,
        Position::PG,
        Position::PG,
        Position::PG,
        Position::SG,
        Position::SG,
        Position::SG,
        Position::SG,
        Position::SF,
        Position::SF,
        Position::SF,
        Position::SF,
        Position::PF,
        Position::PF,
        Position::PF,
        Position::PF,
        Position::C,
        Position::C,
        Position::C,
        Position::C,
    ];
    for (i, pos) in positions_seq.iter().enumerate() {
        let pid = PlayerId(800 + i as u32);
        let team = TeamId((i as u8 % 15) + 1);
        positions.insert(pid, *pos);
        team_drtg.insert(team, 105.0 + (i as f32) * 0.5);
        let stocks = 4 - (i as u32 / 5);
        by_player.insert(
            pid,
            season_with(
                pid,
                team,
                70,
                12 * 70,
                6 * 70,
                3 * 70,
                stocks * 70 / 2,
                stocks * 70 / 2,
                70,
                30 * 70,
            ),
        );
    }
    let agg = SeasonAggregate {
        by_player,
        team_drtg,
    };

    let all_def = compute_all_defensive(&agg, SeasonId(2026), |pid| positions.get(&pid).copied());
    let total: usize = all_def.iter().map(|t| t.ballot.len()).sum();
    assert_eq!(
        total, 10,
        "All-Defensive must have 10 players across 2 teams"
    );
    for (i, team) in all_def.iter().enumerate() {
        assert_eq!(
            team.ballot.len(),
            5,
            "All-Defensive team {} must have 5",
            i + 1
        );
    }
}

// ----------------------------------------------------------------------
// All-Star — 24 players, 12 per conference, 2 starters per position
// ----------------------------------------------------------------------

#[test]
fn all_star_24_players_12_per_conference() {
    let teams = fixture_teams();
    let mut by_player: HashMap<PlayerId, PlayerSeason> = HashMap::new();
    let mut positions: HashMap<PlayerId, Position> = HashMap::new();
    let mut player_team: HashMap<PlayerId, TeamId> = HashMap::new();

    // Put 15 candidates per conference (more than enough to fill 12 slots
    // per side with positional balance).
    let pos_cycle = [
        Position::PG,
        Position::SG,
        Position::SF,
        Position::PF,
        Position::C,
    ];
    for c in 0..30 {
        let pid = PlayerId(900 + c as u32);
        let team_id = teams[c].id;
        positions.insert(pid, pos_cycle[c % 5]);
        player_team.insert(pid, team_id);
        // High-scoring rotation player so we clear the volume floor.
        by_player.insert(
            pid,
            season_with(
                pid,
                team_id,
                50,
                22 * 50,
                5 * 50,
                5 * 50,
                50,
                50,
                2 * 50,
                33 * 50,
            ),
        );
    }

    let mut wins = HashMap::new();
    for t in &teams {
        wins.insert(t.id, 41);
    }
    let standings = make_standings(&teams, &wins);
    let agg = SeasonAggregate {
        by_player,
        team_drtg: HashMap::new(),
    };

    let all_star = compute_all_star(
        &agg,
        &standings,
        SeasonId(2026),
        |pid| positions.get(&pid).copied(),
        |pid| player_team.get(&pid).copied(),
    );
    let east = &all_star[0];
    let west = &all_star[1];
    assert_eq!(
        east.starters.len() + east.reserves.len(),
        12,
        "East roster must be 12"
    );
    assert_eq!(
        west.starters.len() + west.reserves.len(),
        12,
        "West roster must be 12"
    );
    assert_eq!(east.starters.len(), 5);
    assert_eq!(west.starters.len(), 5);

    // 2 guards + 2 forwards + 1 center per conference's starting five.
    for roster in [east, west] {
        let mut g = 0;
        let mut f = 0;
        let mut c = 0;
        for s in &roster.starters {
            match positions[s] {
                Position::PG | Position::SG => g += 1,
                Position::SF | Position::PF => f += 1,
                Position::C => c += 1,
            }
        }
        assert_eq!(
            (g, f, c),
            (2, 2, 1),
            "starters must be 2G+2F+1C; got {:?}",
            (g, f, c)
        );
    }

    // Total = 24 across both conferences.
    let total =
        east.starters.len() + east.reserves.len() + west.starters.len() + west.reserves.len();
    assert_eq!(total, 24);
}
