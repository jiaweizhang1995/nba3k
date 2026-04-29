use nba3k_core::{
    Conference, Division, DraftPick, DraftPickId, GMArchetype, GMPersonality, Protection,
    ProtectionHistoryEntry, SeasonId, Team, TeamId,
};
use nba3k_store::Store;
use tempfile::tempdir;

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("draft_picks.db");
    let store = Store::open(&path).expect("open");
    (dir, store)
}

fn make_team(id: TeamId, abbrev: &str) -> Team {
    Team {
        id,
        abbrev: abbrev.to_string(),
        city: abbrev.to_string(),
        name: abbrev.to_string(),
        conference: Conference::East,
        division: Division::Atlantic,
        gm: GMPersonality::from_archetype(format!("{abbrev} GM"), GMArchetype::Conservative),
        roster: Vec::new(),
        draft_picks: Vec::new(),
        coach: nba3k_core::Coach::default_for(abbrev),
    }
}

fn seed_teams(store: &Store) {
    store
        .upsert_team(&make_team(TeamId(1), "AAA"))
        .expect("team A");
    store
        .upsert_team(&make_team(TeamId(2), "BBB"))
        .expect("team B");
}

fn sample_pick() -> DraftPick {
    DraftPick {
        id: DraftPickId(2027101),
        original_team: TeamId(1),
        current_owner: TeamId(1),
        season: SeasonId(2027),
        round: 1,
        protections: Protection::Unprotected,
        protection_text: None,
        resolved: false,
        protection_history: Vec::new(),
    }
}

#[test]
fn draft_pick_v018_fields_roundtrip() {
    let (_dir, store) = fresh_store();
    seed_teams(&store);
    let pick = DraftPick {
        id: DraftPickId(2027101),
        original_team: TeamId(1),
        current_owner: TeamId(2),
        season: SeasonId(2027),
        round: 1,
        protections: Protection::TopNProtected(4),
        protection_text: Some("top-4 protected".to_string()),
        resolved: false,
        protection_history: vec![ProtectionHistoryEntry {
            season: SeasonId(2026),
            original_team_record: "22-60".to_string(),
            action: "deferred".to_string(),
        }],
    };

    store.upsert_draft_pick(&pick).expect("upsert");
    let loaded = store
        .find_draft_pick(SeasonId(2027), TeamId(1), 1)
        .expect("find")
        .expect("pick exists");

    assert_eq!(loaded.current_owner, TeamId(2));
    assert_eq!(loaded.protections, Protection::TopNProtected(4));
    assert_eq!(loaded.protection_text.as_deref(), Some("top-4 protected"));
    assert_eq!(loaded.protection_history.len(), 1);

    store.mark_draft_pick_resolved(loaded.id).expect("resolved");
    let resolved = store
        .find_draft_pick(SeasonId(2027), TeamId(1), 1)
        .expect("find")
        .expect("pick exists");
    assert!(resolved.resolved);
}

#[test]
fn transfer_draft_pick_updates_owner_only() {
    let (_dir, store) = fresh_store();
    seed_teams(&store);
    let pick = sample_pick();
    store.upsert_draft_pick(&pick).expect("upsert");
    store
        .transfer_draft_pick(pick.id, TeamId(2))
        .expect("transfer");

    let loaded = store
        .find_draft_pick(pick.season, pick.original_team, pick.round)
        .expect("find")
        .expect("pick exists");
    assert_eq!(loaded.current_owner, TeamId(2));
    assert_eq!(loaded.original_team, TeamId(1));
    assert!(!loaded.resolved);
}

#[test]
fn upsert_if_absent_does_not_clobber_existing_swap() {
    let (_dir, store) = fresh_store();
    seed_teams(&store);
    let mut traded = sample_pick();
    traded.current_owner = TeamId(2);
    store.upsert_draft_pick(&traded).expect("upsert traded");

    let vanilla = sample_pick();
    store
        .upsert_if_absent_draft_pick(&vanilla)
        .expect("insert ignored");
    let loaded = store
        .find_draft_pick(vanilla.season, vanilla.original_team, vanilla.round)
        .expect("find")
        .expect("pick exists");
    assert_eq!(loaded.current_owner, TeamId(2));
}

#[test]
fn defer_draft_pick_replaces_next_year_vanilla_row() {
    let (_dir, store) = fresh_store();
    seed_teams(&store);
    let mut owed = sample_pick();
    owed.current_owner = TeamId(2);
    store.upsert_draft_pick(&owed).expect("upsert owed");
    let next_vanilla = DraftPick {
        id: DraftPickId(2028101),
        season: SeasonId(2028),
        ..sample_pick()
    };
    store
        .upsert_draft_pick(&next_vanilla)
        .expect("upsert next vanilla");
    let history = vec![ProtectionHistoryEntry {
        season: SeasonId(2027),
        original_team_record: "protected".to_string(),
        action: "deferred".to_string(),
    }];

    store
        .defer_draft_pick_once(owed.id, SeasonId(2028), &history)
        .expect("defer");

    assert!(store
        .find_draft_pick(SeasonId(2027), TeamId(1), 1)
        .expect("find old")
        .is_none());
    let loaded = store
        .find_draft_pick(SeasonId(2028), TeamId(1), 1)
        .expect("find new")
        .expect("deferred pick exists");
    assert_eq!(loaded.id, owed.id);
    assert_eq!(loaded.current_owner, TeamId(2));
    assert_eq!(loaded.protections, Protection::Unprotected);
    assert_eq!(loaded.protection_history.len(), 1);
}
