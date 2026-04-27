//! Retirement engine tests — see `phases/M11-contracts.md` § Worker B.

use nba3k_core::{
    BirdRights, Cents, Contract, ContractYear, Player, PlayerId, Position, Ratings, SeasonId,
};

fn build_player(id: u32, age: u8, ovr: u8) -> Player {
    Player {
        id: PlayerId(id),
        name: format!("P{}", id),
        primary_position: Position::SF,
        secondary_position: None,
        age,
        overall: ovr,
        potential: ovr,
        ratings: Ratings::default(),
        contract: Some(Contract {
            years: vec![ContractYear {
                season: SeasonId(2026),
                salary: Cents(2_000_000_00),
                guaranteed: true,
                team_option: false,
                player_option: false,
            }],
            signed_in_season: SeasonId(2024),
            bird_rights: BirdRights::Full,
        }),
        team: None,
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: nba3k_core::PlayerRole::default(),
        morale: 0.5,
    }
}

#[test]
fn hard_retire_at_age_41_regardless_of_stats() {
    // Even an MVP-tier 41yo retires under the hard cap.
    let p = build_player(1, 41, 95);
    assert!(nba3k_models::should_retire(&p, 3000));
}

#[test]
fn elite_38yo_with_heavy_minutes_does_not_retire() {
    // 38yo OVR-90 with 2500 min/season — clearly still productive.
    let p = build_player(2, 38, 90);
    assert!(!nba3k_models::should_retire(&p, 2500));
}

#[test]
fn aging_low_overall_low_minutes_retires() {
    // 37yo OVR-65 with 600 min — both conditional triggers fire.
    let p = build_player(3, 37, 65);
    assert!(nba3k_models::should_retire(&p, 600));
}

#[test]
fn stochastic_age_39_is_deterministic_per_player() {
    // 39yo OVR-78 with 1500 min sits in the stochastic bucket: not hard-cap,
    // not conditional (OVR >= 70 AND mins >= 800), so the answer comes from
    // the per-player hash gate. Must be reproducible across calls and not panic.
    let p = build_player(4, 39, 78);
    let first = nba3k_models::should_retire(&p, 1500);
    let second = nba3k_models::should_retire(&p, 1500);
    assert_eq!(first, second, "stochastic gate must be deterministic");
}
