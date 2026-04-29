//! M14-B scouting fog: store-layer round-trip for `scouted` flag and the
//! `list_prospects_visible` read API the CLI consumes.

use nba3k_core::{
    Conference, Division, GMArchetype, GMPersonality, Player, PlayerId, PlayerRole, Position,
    Ratings, Team, TeamId,
};
use nba3k_store::Store;
use tempfile::tempdir;

const HOME: TeamId = TeamId(1);

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("scouting.db");
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

fn make_prospect(id: u32, name: &str, overall: u8, potential: u8) -> Player {
    Player {
        id: PlayerId(id),
        name: name.into(),
        primary_position: Position::SF,
        secondary_position: None,
        age: 19,
        overall,
        potential,
        ratings: Ratings::default(),
        contract: None,
        team: None,
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::Prospect,
        morale: 0.5,
    }
}

#[test]
fn prospects_default_unscouted() {
    let (_dir, store) = fresh_store();
    let p = make_prospect(101, "Alpha Rookie", 70, 85);
    store.upsert_player(&p).expect("upsert");

    assert!(
        !store.is_player_scouted(p.id).unwrap(),
        "fresh prospects must default to scouted = 0"
    );

    let visible = store.list_prospects_visible().unwrap();
    let row = visible
        .iter()
        .find(|(pp, _)| pp.id == p.id)
        .expect("prospect present");
    assert!(!row.1, "list_prospects_visible reports scouted = false");
}

#[test]
fn set_player_scouted_flips_flag() {
    let (_dir, store) = fresh_store();
    let p = make_prospect(202, "Beta Rookie", 73, 88);
    store.upsert_player(&p).expect("upsert");

    store.set_player_scouted(p.id, true).expect("scout");
    assert!(store.is_player_scouted(p.id).unwrap());

    let visible = store.list_prospects_visible().unwrap();
    let row = visible
        .iter()
        .find(|(pp, _)| pp.id == p.id)
        .expect("prospect present after scout");
    assert!(
        row.1,
        "scout flag must surface through list_prospects_visible"
    );
    assert_eq!(row.0.overall, 73, "ratings remain truthful in store");
    assert_eq!(row.0.potential, 88);

    // Idempotent: flipping back works.
    store.set_player_scouted(p.id, false).expect("unscout");
    assert!(!store.is_player_scouted(p.id).unwrap());
}

#[test]
fn list_prospects_visible_orders_scouted_first_then_unscouted_alpha() {
    let (_dir, store) = fresh_store();
    // Three prospects: Charlie(low pot, scouted), Bravo(high pot, scouted),
    // Alpha (un-scouted), Delta (un-scouted). Expected order:
    //   Bravo (scouted, pot 92), Charlie (scouted, pot 80),
    //   Alpha (un-scouted, alpha sort), Delta (un-scouted).
    let alpha = make_prospect(11, "Alpha Z", 60, 70); // un-scouted
    let bravo = make_prospect(12, "Bravo Y", 78, 92); // scouted
    let charlie = make_prospect(13, "Charlie X", 75, 80); // scouted
    let delta = make_prospect(14, "Delta W", 65, 88); // un-scouted

    for p in [&alpha, &bravo, &charlie, &delta] {
        store.upsert_player(p).expect("upsert");
    }
    store.set_player_scouted(bravo.id, true).expect("scout B");
    store.set_player_scouted(charlie.id, true).expect("scout C");

    let visible = store.list_prospects_visible().unwrap();
    let order: Vec<&str> = visible.iter().map(|(p, _)| p.name.as_str()).collect();
    assert_eq!(
        order,
        vec!["Bravo Y", "Charlie X", "Alpha Z", "Delta W"],
        "scouted-first by potential desc, un-scouted tail alphabetical"
    );

    // The fog flag pairs correctly per row.
    let scouted_flags: Vec<bool> = visible.iter().map(|(_, s)| *s).collect();
    assert_eq!(scouted_flags, vec![true, true, false, false]);
}

#[test]
fn retired_and_signed_players_excluded_from_visible_list() {
    let (_dir, store) = fresh_store();
    // Prospect (un-signed, not retired) — should appear.
    let prospect = make_prospect(301, "Active Prospect", 70, 80);
    store.upsert_player(&prospect).expect("upsert prospect");

    // Rostered player — must not leak into the prospect board.
    let mut rostered = make_prospect(302, "Already Drafted", 78, 80);
    rostered.team = Some(HOME);
    rostered.role = PlayerRole::RolePlayer;
    store.upsert_player(&rostered).expect("upsert rostered");

    // Retired player — must not leak even though team_id will be NULL.
    let retired = make_prospect(303, "Hung Em Up", 72, 80);
    store.upsert_player(&retired).expect("upsert retired");
    store.set_player_retired(retired.id).expect("retire");

    let visible = store.list_prospects_visible().unwrap();
    let ids: Vec<PlayerId> = visible.iter().map(|(p, _)| p.id).collect();
    assert!(ids.contains(&prospect.id));
    assert!(!ids.contains(&rostered.id));
    assert!(!ids.contains(&retired.id));
}
