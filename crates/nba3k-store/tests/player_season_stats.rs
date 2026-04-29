use nba3k_core::{PlayerId, PlayerSeasonStats};
use nba3k_store::Store;
use tempfile::tempdir;

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("pss.db");
    let store = Store::open(&path).expect("open");
    (dir, store)
}

fn sample(pid: u32) -> PlayerSeasonStats {
    PlayerSeasonStats {
        player_id: PlayerId(pid),
        season_year: 2026,
        gp: 64,
        mpg: 35.7,
        ppg: 28.5,
        rpg: 7.7,
        apg: 7.4,
        spg: 1.6,
        bpg: 0.8,
        fg_pct: 0.482,
        three_pct: 0.378,
        ft_pct: 0.789,
        ts_pct: 0.0,
        usage: 0.0,
    }
}

#[test]
fn pss_round_trip_get() {
    let (_dir, store) = fresh_store();
    // Insert a stub player row so the FK passes.
    store.conn().execute(
        "INSERT INTO players(id, name, primary_position, age, overall, potential, ratings_json) \
         VALUES (1, 'Test', 'PG', 25, 80, 85, '{}')",
        [],
    ).expect("seed player");
    let s = sample(1);
    store.upsert_player_season_stats(&s).expect("insert");
    let got = store.get_player_season_stats(PlayerId(1), 2026).unwrap().unwrap();
    assert_eq!(got, s);
}

#[test]
fn pss_upsert_replaces_existing() {
    let (_dir, store) = fresh_store();
    store.conn().execute(
        "INSERT INTO players(id, name, primary_position, age, overall, potential, ratings_json) \
         VALUES (2, 'Test2', 'SG', 26, 82, 86, '{}')",
        [],
    ).expect("seed player");
    let s1 = sample(2);
    store.upsert_player_season_stats(&s1).expect("insert");
    let s2 = PlayerSeasonStats { ppg: 30.1, ..s1.clone() };
    store.upsert_player_season_stats(&s2).expect("update");
    let got = store.get_player_season_stats(PlayerId(2), 2026).unwrap().unwrap();
    assert!((got.ppg - 30.1).abs() < 1e-3);
}

#[test]
fn pss_list_filters_by_season() {
    let (_dir, store) = fresh_store();
    for pid in 3..=5 {
        store.conn().execute(
            "INSERT INTO players(id, name, primary_position, age, overall, potential, ratings_json) \
             VALUES (?1, ?2, 'PG', 25, 80, 85, '{}')",
            rusqlite::params![pid as i64, format!("p{pid}")],
        ).expect("seed player");
        store.upsert_player_season_stats(&sample(pid)).expect("insert");
    }
    let rows = store.list_player_season_stats(2026).unwrap();
    assert_eq!(rows.len(), 3);
    let other = store.list_player_season_stats(2025).unwrap();
    assert!(other.is_empty());
}

#[test]
fn pss_get_missing_returns_none() {
    let (_dir, store) = fresh_store();
    let got = store.get_player_season_stats(PlayerId(99), 2026).unwrap();
    assert!(got.is_none());
}
