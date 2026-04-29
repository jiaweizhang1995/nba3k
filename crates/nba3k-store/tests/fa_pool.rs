//! Free-agency v2 (M10-C) integration tests.
//!
//! Exercises the round-trip between roster, free-agent pool, and prospect
//! pool through the live SQLite migrations. Each test opens a fresh
//! tempfile DB so there's no cross-test bleed.

use nba3k_core::{
    Conference, Division, GMArchetype, GMPersonality, Player, PlayerId, PlayerRole, Position,
    Ratings, Team, TeamId,
};
use nba3k_store::Store;
use tempfile::tempdir;

const HOME: TeamId = TeamId(1);

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("fa.db");
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

fn make_player(id: u32, team: Option<TeamId>, overall: u8) -> Player {
    Player {
        id: PlayerId(id),
        name: format!("Player{id}"),
        primary_position: Position::SF,
        secondary_position: None,
        age: 27,
        overall,
        potential: overall,
        ratings: Ratings::default(),
        contract: None,
        team,
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

#[test]
fn roundtrip_cut_then_sign() {
    let (_dir, store) = fresh_store();
    let p = make_player(101, Some(HOME), 75);
    store.upsert_player(&p).expect("upsert");

    // Baseline.
    assert_eq!(store.roster_for_team(HOME).unwrap().len(), 1);
    assert!(store.list_free_agents().unwrap().is_empty());

    // Cut → roster shrinks, FA pool grows.
    store.cut_player(p.id).expect("cut");
    assert_eq!(store.roster_for_team(HOME).unwrap().len(), 0);
    let pool = store.list_free_agents().unwrap();
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].id, p.id);
    assert!(pool[0].team.is_none());

    // Sign back → roster grows, FA pool empties. Going through
    // `assign_player_to_team` is what the CLI does — must clear `is_free_agent`.
    store.assign_player_to_team(p.id, HOME).expect("assign");
    assert_eq!(store.roster_for_team(HOME).unwrap().len(), 1);
    assert!(
        store.list_free_agents().unwrap().is_empty(),
        "signing must clear the FA flag"
    );
}

#[test]
fn prospects_do_not_appear_in_fa_pool() {
    let (_dir, store) = fresh_store();
    // Prospect: no team, is_free_agent default 0.
    let prospect = make_player(201, None, 70);
    store.upsert_player(&prospect).expect("upsert prospect");

    // Cut a rostered player to create one true FA.
    let veteran = make_player(202, Some(HOME), 78);
    store.upsert_player(&veteran).expect("upsert vet");
    store.cut_player(veteran.id).expect("cut vet");

    // The two pools are disjoint.
    let fa = store.list_free_agents().unwrap();
    assert_eq!(fa.len(), 1, "only the cut player is a free agent");
    assert_eq!(fa[0].id, veteran.id);

    let prospects = store.list_prospects().unwrap();
    let prospect_ids: Vec<_> = prospects.iter().map(|p| p.id).collect();
    assert!(prospect_ids.contains(&prospect.id));
    assert!(
        !prospect_ids.contains(&veteran.id),
        "FAs must not leak into the prospect pool"
    );
}

#[test]
fn list_free_agents_orders_by_overall_desc() {
    let (_dir, store) = fresh_store();
    for (id, ovr) in [(301_u32, 70_u8), (302, 80), (303, 75)] {
        let p = make_player(id, Some(HOME), ovr);
        store.upsert_player(&p).expect("upsert");
        store.cut_player(p.id).expect("cut");
    }
    let fa = store.list_free_agents().unwrap();
    let overalls: Vec<u8> = fa.iter().map(|p| p.overall).collect();
    assert_eq!(overalls, vec![80, 75, 70]);
}

#[test]
fn cut_then_resign_round_trips_cleanly() {
    let (_dir, store) = fresh_store();
    let p = make_player(401, Some(HOME), 72);
    store.upsert_player(&p).expect("upsert");

    store.cut_player(p.id).expect("cut");
    assert_eq!(store.list_free_agents().unwrap().len(), 1);
    store.assign_player_to_team(p.id, HOME).expect("re-sign");

    // Cut a second time after re-signing — the FA flag must come back on,
    // not stay stale from an earlier flip.
    store.cut_player(p.id).expect("cut again");
    let fa = store.list_free_agents().unwrap();
    assert_eq!(fa.len(), 1);
    assert_eq!(fa[0].id, p.id);
}

/// Helper-level rehearsal of the 18-player CBA cap — the CLI guard reads
/// `roster_for_team(...).len() >= 18`. We assert the underlying read agrees.
#[test]
fn roster_cap_is_observable_from_store() {
    let (_dir, store) = fresh_store();
    for id in 0..18 {
        let p = make_player(500 + id, Some(HOME), 70);
        store.upsert_player(&p).expect("upsert");
    }
    let roster = store.roster_for_team(HOME).unwrap();
    assert_eq!(roster.len(), 18);
    // The CLI refuses to sign at this point; assert there's no signed-FA
    // mechanism in the store layer that would silently bypass the guard.
    assert!(store.list_free_agents().unwrap().is_empty());
}
