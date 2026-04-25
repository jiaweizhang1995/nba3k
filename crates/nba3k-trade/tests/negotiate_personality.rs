//! Personality-driven move-distribution tests for the counter-offer engine.
//!
//! These tests exercise `pick_move`'s behaviour through the public
//! `_pick_move_for_test` shim so they don't depend on Workers A or C.

mod cba_common;

use nba3k_core::GMArchetype;
use nba3k_trade::negotiate;
use rand_chacha::ChaCha8Rng;
use rand::SeedableRng;

#[test]
fn negotiate_conservative_gm_never_picks_subtract() {
    let mut rng = ChaCha8Rng::seed_from_u64(42);
    let mut subtract_count = 0usize;
    for _ in 0..100 {
        let kind = negotiate::_pick_move_for_test(GMArchetype::Conservative, &mut rng);
        if kind == "subtract" {
            subtract_count += 1;
        }
    }
    assert_eq!(subtract_count, 0, "Conservative must never select Subtract");
}

#[test]
fn negotiate_old_school_gm_never_picks_subtract() {
    // Sanity: other risk-averse archetypes also avoid Subtract.
    let mut rng = ChaCha8Rng::seed_from_u64(7);
    let mut subtract_count = 0usize;
    for _ in 0..200 {
        let kind = negotiate::_pick_move_for_test(GMArchetype::OldSchool, &mut rng);
        if kind == "subtract" {
            subtract_count += 1;
        }
    }
    assert_eq!(subtract_count, 0, "OldSchool must never select Subtract");
}

#[test]
fn negotiate_aggressive_gm_picks_subtract_meaningfully_often() {
    let mut rng = ChaCha8Rng::seed_from_u64(99);
    let mut subtract_count = 0usize;
    for _ in 0..500 {
        let kind = negotiate::_pick_move_for_test(GMArchetype::Aggressive, &mut rng);
        if kind == "subtract" {
            subtract_count += 1;
        }
    }
    // Aggressive baseline = 0.45 subtract probability. Allow generous slack.
    assert!(
        subtract_count >= 150,
        "Aggressive should pick Subtract often, got {subtract_count}/500"
    );
}
