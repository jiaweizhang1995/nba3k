//! M17-C player notes / favorites store-layer round-trip tests.
//!
//! Covers the three contract points the CLI relies on: add → list shows
//! the row, double-add overwrites text (UPSERT semantics), and remove
//! drops the row so list comes back empty.

use nba3k_core::{
    Conference, Division, GMArchetype, GMPersonality, Player, PlayerId, PlayerRole, Position,
    Ratings, Team, TeamId,
};
use nba3k_store::Store;
use tempfile::tempdir;

const HOME: TeamId = TeamId(1);

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("notes.db");
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

fn make_player(id: u32, name: &str) -> Player {
    Player {
        id: PlayerId(id),
        name: name.into(),
        primary_position: Position::SF,
        secondary_position: None,
        age: 25,
        overall: 80,
        potential: 85,
        ratings: Ratings::default(),
        contract: None,
        team: Some(HOME),
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::Starter,
        morale: 0.6,
    }
}

#[test]
fn add_then_list_returns_row() {
    let (_dir, store) = fresh_store();
    let p = make_player(101, "Jayson Tatum");
    store.upsert_player(&p).expect("upsert");

    store.insert_note(p.id, "watch as draft target").expect("insert note");

    let notes = store.list_notes().expect("list");
    assert_eq!(notes.len(), 1, "one note in, one note out");
    assert_eq!(notes[0].player_id, p.id);
    assert_eq!(notes[0].text.as_deref(), Some("watch as draft target"));
    assert!(
        !notes[0].created_at.is_empty(),
        "created_at must be stamped on insert",
    );
}

#[test]
fn add_same_player_twice_overwrites_text() {
    let (_dir, store) = fresh_store();
    let p = make_player(202, "Jaylen Brown");
    store.upsert_player(&p).expect("upsert");

    store.insert_note(p.id, "first take").expect("first insert");
    let first = store.list_notes().expect("list");
    assert_eq!(first.len(), 1);

    store.insert_note(p.id, "updated take").expect("second insert");
    let second = store.list_notes().expect("list");
    assert_eq!(second.len(), 1, "UPSERT must not duplicate the row");
    assert_eq!(
        second[0].text.as_deref(),
        Some("updated take"),
        "second insert should overwrite the text",
    );
}

#[test]
fn remove_drops_row_and_list_returns_empty() {
    let (_dir, store) = fresh_store();
    let p = make_player(303, "Derrick White");
    store.upsert_player(&p).expect("upsert");

    store.insert_note(p.id, "extension target").expect("insert");
    assert_eq!(store.list_notes().unwrap().len(), 1);

    let n = store.delete_note(p.id).expect("delete");
    assert_eq!(n, 1, "delete should report exactly one row removed");
    assert!(
        store.list_notes().unwrap().is_empty(),
        "list must be empty after removal",
    );

    // Removing again is a no-op the CLI relies on.
    let n2 = store.delete_note(p.id).expect("delete again");
    assert_eq!(n2, 0, "second delete reports zero rows");
}
