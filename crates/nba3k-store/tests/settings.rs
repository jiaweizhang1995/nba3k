use nba3k_store::Store;
use tempfile::tempdir;

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("settings.db");
    let store = Store::open(&path).expect("open");
    (dir, store)
}

#[test]
fn settings_roundtrip_and_update() {
    let (_dir, store) = fresh_store();

    assert_eq!(store.read_setting("language").expect("read missing"), None);

    store.write_setting("language", "zh").expect("write zh");
    assert_eq!(
        store.read_setting("language").expect("read zh").as_deref(),
        Some("zh")
    );

    store.write_setting("language", "en").expect("write en");
    assert_eq!(
        store.read_setting("language").expect("read en").as_deref(),
        Some("en")
    );
}
