//! Trade-kicker asymmetry tests. Per RESEARCH.md item 7:
//! sender uses pre-kicker, receiver uses post-kicker prorated.

mod cba_common;

use cba_common::*;
use nba3k_core::Cents;
use nba3k_trade::cba::{incoming_salary_post_kicker, outgoing_salary_pre_kicker};

#[test]
fn kicker_one_year_remaining_full_bump_year_one() {
    // $20M for 1 remaining guaranteed year, 15% kicker.
    // Total kicker = $3M, prorated over 1 year = $3M.
    // Receiver year-1 cap hit = $20M + $3M = $23M.
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let mut p = player_on(150, TEAM_A, 1, 20_000_000);
    p.trade_kicker_pct = Some(15);

    let mut players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        p,
        player_on(250, TEAM_B, 1, 30_000_000),
    ];
    let _ = &mut players; // keep `players` mutable for future expansion if any
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot();

    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[150]),
        TEAM_B,
        assets_players(&[250]),
    );

    // Sender side (TEAM_A): $20M (pre-kicker).
    let out_a = outgoing_salary_pre_kicker(TEAM_A, &offer, &snap);
    assert_eq!(out_a.as_dollars(), 20_000_000);

    // Receiver side (TEAM_B): reads $23M for incoming player (post-kicker).
    let in_b = incoming_salary_post_kicker(TEAM_B, &offer, &snap);
    assert_eq!(in_b.as_dollars(), 23_000_000);
}

#[test]
fn kicker_three_years_prorates() {
    // $20M/yr for 3 remaining guaranteed years, 15% kicker.
    // Total kicker base = 3 * $20M = $60M. Total kicker = $9M.
    // Prorated over 3 years = $3M/yr. Year-1 cap hit = $20M + $3M = $23M.
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let mut p = player_on(150, TEAM_A, 3, 20_000_000);
    p.trade_kicker_pct = Some(15);

    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        p,
        player_on(250, TEAM_B, 1, 30_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot();

    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[150]),
        TEAM_B,
        assets_players(&[250]),
    );

    let in_b = incoming_salary_post_kicker(TEAM_B, &offer, &snap);
    assert_eq!(in_b.as_dollars(), 23_000_000);
}

#[test]
fn no_kicker_means_post_equals_pre() {
    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        player_on(150, TEAM_A, 1, 20_000_000),
        player_on(250, TEAM_B, 1, 30_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot();

    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[150]),
        TEAM_B,
        assets_players(&[250]),
    );
    let out = outgoing_salary_pre_kicker(TEAM_A, &offer, &snap);
    let inc = incoming_salary_post_kicker(TEAM_B, &offer, &snap);
    assert_eq!(out.as_dollars(), 20_000_000);
    // Receiver TEAM_B receives the $30M player from TEAM_A? No — TEAM_B
    // receives whatever TEAM_A sends. TEAM_A sent player 150 ($20M, no
    // kicker), so TEAM_B incoming = $20M.
    assert_eq!(inc.as_dollars(), 20_000_000);
}

#[test]
fn kicker_skips_unexercised_options() {
    // 3 contract years, but year 2 is a player option, year 3 is a team
    // option. Only year 1 ($20M) counts toward kicker base.
    // Total kicker = 0.15 * $20M = $3M. Prorated over 1 guaranteed year =
    // $3M. Year-1 cap hit = $23M.
    use nba3k_core::{
        BirdRights, Contract, ContractYear, Player, PlayerId, Position, Ratings, SeasonId,
    };

    let teams = vec![make_team(TEAM_A, "AAA"), make_team(TEAM_B, "BBB")];
    let kickered = Player {
        id: PlayerId(150),
        name: "Kickered".into(),
        primary_position: Position::SF,
        secondary_position: None,
        age: 27,
        overall: 80,
        potential: 82,
        ratings: Ratings::default(),
        contract: Some(Contract {
            years: vec![
                ContractYear {
                    season: SeasonId(SEASON.0),
                    salary: Cents::from_dollars(20_000_000),
                    guaranteed: true,
                    team_option: false,
                    player_option: false,
                },
                ContractYear {
                    season: SeasonId(SEASON.0 + 1),
                    salary: Cents::from_dollars(20_000_000),
                    guaranteed: true,
                    team_option: false,
                    player_option: true,
                },
                ContractYear {
                    season: SeasonId(SEASON.0 + 2),
                    salary: Cents::from_dollars(20_000_000),
                    guaranteed: true,
                    team_option: true,
                    player_option: false,
                },
            ],
            signed_in_season: SEASON,
            bird_rights: BirdRights::Full,
        }),
        team: Some(TEAM_A),
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: Some(15),
        role: nba3k_core::PlayerRole::default(),
        morale: 0.5,
    };

    let players = vec![
        player_on(101, TEAM_A, 1, 160_000_000),
        player_on(201, TEAM_B, 1, 160_000_000),
        kickered,
        player_on(250, TEAM_B, 1, 30_000_000),
    ];
    let mut w = World::new(teams, players);
    pad_roster(&mut w, TEAM_A, 14, 1_000);
    pad_roster(&mut w, TEAM_B, 14, 2_000);
    let snap = w.snapshot();

    let offer = two_team_offer(
        TEAM_A,
        assets_players(&[150]),
        TEAM_B,
        assets_players(&[250]),
    );
    let in_b = incoming_salary_post_kicker(TEAM_B, &offer, &snap);
    assert_eq!(in_b.as_dollars(), 23_000_000);
}
