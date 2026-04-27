//! M21 Rotation Level A store-layer round-trip tests.
//!
//! Covers: upsert→read round-trip, single-slot clear, clear-all wipe,
//! and the API-side validation of unknown position strings.

use nba3k_core::{
    Conference, Division, GMArchetype, GMPersonality, PlayerId, Starters, Team, TeamId,
};
use nba3k_store::{Store, StoreError};
use tempfile::tempdir;

const HOME: TeamId = TeamId(1);

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("rotation.db");
    let store = Store::open(&path).expect("open");
    let team = Team {
        id: HOME,
        abbrev: "BOS".into(),
        city: "Boston".into(),
        name: "Celtics".into(),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
        coach: nba3k_core::Coach::default_for("BOS"),
        roster: Vec::new(),
        draft_picks: Vec::new(),
    };
    store.upsert_team(&team).expect("upsert team");
    (dir, store)
}

#[test]
fn empty_team_reads_default_starters() {
    let (_dir, store) = fresh_store();
    let starters = store.read_starters(HOME).expect("read");
    assert_eq!(starters, Starters::default());
    assert!(!starters.is_complete());
}

#[test]
fn test_starters_set_and_read() {
    let (_dir, store) = fresh_store();
    store.upsert_starter(HOME, "PG", PlayerId(101)).expect("pg");
    store.upsert_starter(HOME, "SG", PlayerId(102)).expect("sg");
    store.upsert_starter(HOME, "SF", PlayerId(103)).expect("sf");
    store.upsert_starter(HOME, "PF", PlayerId(104)).expect("pf");
    store.upsert_starter(HOME, "C", PlayerId(105)).expect("c");

    let starters = store.read_starters(HOME).expect("read");
    assert_eq!(starters.pg, Some(PlayerId(101)));
    assert_eq!(starters.sg, Some(PlayerId(102)));
    assert_eq!(starters.sf, Some(PlayerId(103)));
    assert_eq!(starters.pf, Some(PlayerId(104)));
    assert_eq!(starters.c, Some(PlayerId(105)));
    assert!(starters.is_complete());

    // Re-upserting the same slot replaces, not duplicates.
    store.upsert_starter(HOME, "PG", PlayerId(999)).expect("re-upsert");
    let starters = store.read_starters(HOME).expect("read");
    assert_eq!(starters.pg, Some(PlayerId(999)));
}

#[test]
fn test_starters_clear_one() {
    let (_dir, store) = fresh_store();
    for (pos, pid) in [("PG", 101), ("SG", 102), ("SF", 103), ("PF", 104), ("C", 105)] {
        store.upsert_starter(HOME, pos, PlayerId(pid)).expect("set");
    }
    store.clear_starter(HOME, "PG").expect("clear pg");

    let starters = store.read_starters(HOME).expect("read");
    assert_eq!(starters.pg, None, "PG must be cleared");
    assert_eq!(starters.sg, Some(PlayerId(102)));
    assert_eq!(starters.sf, Some(PlayerId(103)));
    assert_eq!(starters.pf, Some(PlayerId(104)));
    assert_eq!(starters.c, Some(PlayerId(105)));
    assert!(!starters.is_complete(), "partial lineup must not be complete");

    // Clearing an already-empty slot is a no-op the UI relies on.
    store.clear_starter(HOME, "PG").expect("clear pg again");
}

#[test]
fn test_starters_clear_all() {
    let (_dir, store) = fresh_store();
    for (pos, pid) in [("PG", 101), ("SG", 102), ("SF", 103), ("PF", 104), ("C", 105)] {
        store.upsert_starter(HOME, pos, PlayerId(pid)).expect("set");
    }
    store.clear_all_starters(HOME).expect("clear all");

    let starters = store.read_starters(HOME).expect("read");
    assert_eq!(starters, Starters::default());
}

#[test]
fn test_invalid_position_rejected() {
    let (_dir, store) = fresh_store();
    let err = store
        .upsert_starter(HOME, "XX", PlayerId(101))
        .expect_err("invalid pos must error");
    assert!(
        matches!(err, StoreError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );

    let err = store
        .clear_starter(HOME, "lol")
        .expect_err("invalid pos must error on clear too");
    assert!(matches!(err, StoreError::InvalidInput(_)));
}

#[test]
fn test_starters_isolated_per_team() {
    let (_dir, store) = fresh_store();
    let other = TeamId(2);
    let team = Team {
        id: other,
        abbrev: "LAL".into(),
        city: "Los Angeles".into(),
        name: "Lakers".into(),
        conference: Conference::West,
        division: Division::Pacific,
        gm: GMPersonality::from_archetype("Anon", GMArchetype::Conservative),
        coach: nba3k_core::Coach::default_for("LAL"),
        roster: Vec::new(),
        draft_picks: Vec::new(),
    };
    store.upsert_team(&team).expect("upsert second team");

    store.upsert_starter(HOME, "PG", PlayerId(11)).expect("bos pg");
    store.upsert_starter(other, "PG", PlayerId(22)).expect("lal pg");

    assert_eq!(store.read_starters(HOME).unwrap().pg, Some(PlayerId(11)));
    assert_eq!(store.read_starters(other).unwrap().pg, Some(PlayerId(22)));

    store.clear_all_starters(HOME).expect("wipe bos");
    assert_eq!(store.read_starters(HOME).unwrap().pg, None);
    assert_eq!(
        store.read_starters(other).unwrap().pg,
        Some(PlayerId(22)),
        "wiping BOS must not touch LAL"
    );
}
