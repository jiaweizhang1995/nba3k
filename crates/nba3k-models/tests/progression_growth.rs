//! Tests for the M5 progression engine — pre-peak growth + post-peak
//! regression. Uses bare `Player` constructors (no LeagueSnapshot) since
//! `progression` is a pure-function module.

use nba3k_core::{Player, PlayerId, PlayerRole, Position, Ratings, SeasonId};
use nba3k_models::progression::{
    apply_progression_step, progress_player, regress_player, update_dynamic_potential,
    PlayerDevelopment,
};

fn make_player(id: u32, age: u8, ovr_target: u8, potential: u8, pos: Position) -> Player {
    // Spread `ovr_target` across all 21 attributes — overall_for averages
    // category buckets so a flat fill yields ~ovr_target overall.
    let r = Ratings {
        close_shot: ovr_target,
        driving_layup: ovr_target,
        driving_dunk: ovr_target,
        standing_dunk: ovr_target,
        post_control: ovr_target,
        mid_range: ovr_target,
        three_point: ovr_target,
        free_throw: ovr_target,
        passing_accuracy: ovr_target,
        ball_handle: ovr_target,
        speed_with_ball: ovr_target,
        interior_defense: ovr_target,
        perimeter_defense: ovr_target,
        steal: ovr_target,
        block: ovr_target,
        off_reb: ovr_target,
        def_reb: ovr_target,
        speed: ovr_target,
        agility: ovr_target,
        strength: ovr_target,
        vertical: ovr_target,
    };
    Player {
        id: PlayerId(id),
        name: format!("Player{}", id),
        primary_position: pos,
        secondary_position: None,
        age,
        overall: r.overall_for(pos),
        potential,
        ratings: r,
        contract: None,
        team: None,
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

fn make_dev(player: &Player, work_ethic: u8) -> PlayerDevelopment {
    PlayerDevelopment {
        player_id: player.id,
        peak_start_age: 25,
        peak_end_age: 30,
        dynamic_potential: player.potential,
        work_ethic,
        last_progressed_season: SeasonId(2025),
    }
}

/// 22yo OVR-78 with potential 90, work_ethic 80, played 35min/82 games
/// (~2870 min) should gain +2..=3 OVR after one progression pass.
#[test]
fn pre_peak_workhorse_gains_two_to_three_ovr() {
    let mut player = make_player(1, 22, 78, 90, Position::SF);
    let mut dev = make_dev(&player, 80);
    let prior_ovr = player.overall;

    let mins = 35 * 82; // 2870
    apply_progression_step(&mut player, &mut dev, mins, 23, SeasonId(2026));

    let gain = player.overall as i32 - prior_ovr as i32;
    assert!(
        gain >= 2 && gain <= 3,
        "expected +2..=3 OVR for 22yo workhorse, got {} (was {} now {})",
        gain,
        prior_ovr,
        player.overall
    );
}

/// 32yo OVR-86 should regress -1..=2 OVR with athleticism declining first.
#[test]
fn post_peak_veteran_regresses_one_to_two_ovr() {
    let mut player = make_player(2, 32, 86, 88, Position::PF);
    let mut dev = make_dev(&player, 70);
    // Athleticism block before
    let pre_ath = player.ratings.speed as i16
        + player.ratings.agility as i16
        + player.ratings.vertical as i16;
    let pre_iq = player.ratings.passing_accuracy as i16 + player.ratings.post_control as i16;

    let prior_ovr = player.overall;
    let mins = 28 * 70; // 1960 — moderate vet load
    apply_progression_step(&mut player, &mut dev, mins, 33, SeasonId(2026));

    let drop = prior_ovr as i32 - player.overall as i32;
    assert!(
        drop >= 1 && drop <= 2,
        "expected -1..=2 OVR for 32yo, got drop={} (was {} now {})",
        drop,
        prior_ovr,
        player.overall
    );

    let post_ath = player.ratings.speed as i16
        + player.ratings.agility as i16
        + player.ratings.vertical as i16;
    let post_iq = player.ratings.passing_accuracy as i16 + player.ratings.post_control as i16;
    let ath_drop = pre_ath - post_ath;
    let iq_drop = pre_iq - post_iq;
    // 3 athletic fields vs 2 IQ-adjacent fields, but per-attribute the
    // athletic drop must out-pace the IQ drop.
    let ath_per_attr = ath_drop as f32 / 3.0;
    let iq_per_attr = iq_drop as f32 / 2.0;
    assert!(
        ath_per_attr >= iq_per_attr,
        "athleticism per-attribute drop should lead IQ: ath={} iq={}",
        ath_per_attr,
        iq_per_attr
    );
}

/// 19yo high-potential player who is on track keeps dynamic_potential.
#[test]
fn pre_peak_on_track_keeps_dynamic_potential() {
    // Project: 19yo OVR-72, potential 90, peak_end 30 — needs ~1.6/yr.
    let player = make_player(3, 19, 72, 90, Position::SG);
    let dev = make_dev(&player, 75);
    let new_dp = update_dynamic_potential(&player, &dev, 19);
    assert_eq!(
        new_dp, dev.dynamic_potential,
        "on-track player should keep dynamic_potential"
    );
}

/// 23yo who hasn't grown in 2 seasons (low minutes) — falls behind.
/// dynamic_potential should drop 2-4 points.
#[test]
fn pre_peak_falling_behind_revises_potential_down() {
    // 23yo at OVR-75 with potential 90 — 7 yrs to peak end (30) means
    // need ~2.1/yr. If actual gain has been 0 the projection should slip.
    // We model "falling behind" as: dynamic_potential is still 90, but
    // overall hasn't moved — gap relative to remaining years exceeds 2.5.
    let player = make_player(4, 23, 75, 90, Position::SG);
    // Bump dynamic_potential to a number that yields needed-per-year > 2.5.
    // Years left = 30 - 23 = 7. Gap = 90 - 75 = 15. Needed = 2.14.
    // To force >2.5 we want gap/yrs > 2.5 — bump dyn_potential to 95.
    let dev = PlayerDevelopment {
        player_id: player.id,
        peak_start_age: 25,
        peak_end_age: 30,
        dynamic_potential: 95,
        work_ethic: 50,
        last_progressed_season: SeasonId(2025),
    };
    let new_dp = update_dynamic_potential(&player, &dev, 23);
    let drop = dev.dynamic_potential as i32 - new_dp as i32;
    assert!(
        (2..=4).contains(&drop),
        "expected dynamic_potential to drop 2-4, got drop={} (was {} now {})",
        drop,
        dev.dynamic_potential,
        new_dp
    );
}

/// Past peak_end — dynamic_potential collapses to current overall.
#[test]
fn post_peak_dyn_potential_matches_current_overall() {
    let player = make_player(5, 35, 78, 92, Position::PG);
    let dev = make_dev(&player, 70);
    let new_dp = update_dynamic_potential(&player, &dev, 35);
    assert_eq!(new_dp, player.ratings.overall_for(player.primary_position));
}

/// Per-attribute caps: any single attribute can't gain more than +3
/// or lose more than -4 in a single pass, no matter the budget.
#[test]
fn per_attribute_caps_are_enforced() {
    let player = make_player(6, 18, 60, 99, Position::C);
    let dev = PlayerDevelopment {
        player_id: player.id,
        peak_start_age: 25,
        peak_end_age: 30,
        dynamic_potential: 99,
        work_ethic: 99,
        last_progressed_season: SeasonId(2025),
    };
    let delta = progress_player(&player, &dev, 3000, 19);
    let fields: [i8; 21] = [
        delta.close_shot,
        delta.driving_layup,
        delta.driving_dunk,
        delta.standing_dunk,
        delta.post_control,
        delta.mid_range,
        delta.three_point,
        delta.free_throw,
        delta.passing_accuracy,
        delta.ball_handle,
        delta.speed_with_ball,
        delta.interior_defense,
        delta.perimeter_defense,
        delta.steal,
        delta.block,
        delta.off_reb,
        delta.def_reb,
        delta.speed,
        delta.agility,
        delta.strength,
        delta.vertical,
    ];
    for f in fields {
        assert!(f <= 3, "no attribute can gain more than +3 (got {})", f);
        assert!(f >= 0, "progress should not produce negative deltas");
    }

    // Decline cap.
    let mut old = make_player(7, 36, 80, 90, Position::SG);
    old.ratings.speed = 90;
    let old_dev = make_dev(&old, 30);
    let d = regress_player(&old, &old_dev, 36);
    let fields: [i8; 21] = [
        d.close_shot,
        d.driving_layup,
        d.driving_dunk,
        d.standing_dunk,
        d.post_control,
        d.mid_range,
        d.three_point,
        d.free_throw,
        d.passing_accuracy,
        d.ball_handle,
        d.speed_with_ball,
        d.interior_defense,
        d.perimeter_defense,
        d.steal,
        d.block,
        d.off_reb,
        d.def_reb,
        d.speed,
        d.agility,
        d.strength,
        d.vertical,
    ];
    for f in fields {
        assert!(f >= -4, "no attribute can lose more than -4 (got {})", f);
        assert!(f <= 0, "regress should not produce positive deltas");
    }
}

/// PlayerDevelopment serializes losslessly through serde_json — verifies
/// the Store roundtrip will work.
#[test]
fn player_development_json_roundtrip() {
    let dev = PlayerDevelopment {
        player_id: PlayerId(42),
        peak_start_age: 25,
        peak_end_age: 30,
        dynamic_potential: 87,
        work_ethic: 73,
        last_progressed_season: SeasonId(2026),
    };
    let json = serde_json::to_string(&dev).expect("ser ok");
    let back: PlayerDevelopment = serde_json::from_str(&json).expect("de ok");
    assert_eq!(back, dev);
}

/// Headroom guard: a player already at their dynamic_potential ceiling
/// should grow much slower than one with significant headroom.
#[test]
fn headroom_throttles_growth_at_ceiling() {
    let p_low = make_player(8, 22, 75, 90, Position::SF);
    let p_high = make_player(9, 22, 88, 90, Position::SF);

    let dev_low = make_dev(&p_low, 80);
    let dev_high = make_dev(&p_high, 80);
    let mins = 35 * 82;
    let d_low = progress_player(&p_low, &dev_low, mins, 23);
    let d_high = progress_player(&p_high, &dev_high, mins, 23);
    assert!(
        d_low.sum_signed() > d_high.sum_signed(),
        "player with more headroom should gain more aggregate: low={} high={}",
        d_low.sum_signed(),
        d_high.sum_signed()
    );
}
