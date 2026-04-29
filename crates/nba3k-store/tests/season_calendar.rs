use chrono::NaiveDate;
use nba3k_core::{SeasonCalendar, SeasonId};
use nba3k_store::Store;
use tempfile::tempdir;

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("calendar.db");
    let store = Store::open(&path).expect("open");
    (dir, store)
}

#[test]
fn season_calendar_2026_seeded_by_migration() {
    let (_dir, store) = fresh_store();
    let cal = store
        .get_season_calendar(SeasonId(2026))
        .expect("read")
        .expect("V016 seeds 2026 row");
    assert_eq!(cal.season_year, 2026);
    assert_eq!(cal.start_date, NaiveDate::from_ymd_opt(2025, 10, 21).unwrap());
    assert_eq!(cal.end_date, NaiveDate::from_ymd_opt(2026, 4, 12).unwrap());
    assert_eq!(cal.trade_deadline, NaiveDate::from_ymd_opt(2026, 2, 5).unwrap());
    assert_eq!(cal.all_star_day, 41);
    assert_eq!(cal.cup_final_day, 55);
}

#[test]
fn season_calendar_upsert_round_trip_and_replace() {
    let (_dir, store) = fresh_store();
    let next = SeasonCalendar {
        season_year: 2027,
        start_date: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        end_date: NaiveDate::from_ymd_opt(2027, 4, 11).unwrap(),
        trade_deadline: NaiveDate::from_ymd_opt(2027, 2, 4).unwrap(),
        all_star_day: 41,
        cup_group_day: 30,
        cup_qf_day: 45,
        cup_sf_day: 53,
        cup_final_day: 55,
    };
    store.upsert_season_calendar(&next).expect("insert 2027");
    let read = store
        .get_season_calendar(SeasonId(2027))
        .expect("read")
        .expect("row exists");
    assert_eq!(read, next);

    // Replace: shift trade deadline a day later, confirm row updates.
    let updated = SeasonCalendar {
        trade_deadline: NaiveDate::from_ymd_opt(2027, 2, 5).unwrap(),
        ..next
    };
    store.upsert_season_calendar(&updated).expect("upsert 2027");
    let after = store.get_season_calendar(SeasonId(2027)).unwrap().unwrap();
    assert_eq!(after.trade_deadline, NaiveDate::from_ymd_opt(2027, 2, 5).unwrap());
}

#[test]
fn season_calendar_default_for_extrapolates() {
    // Pure helper, no store touch.
    let cal = SeasonCalendar::default_for(2027);
    assert_eq!(cal.season_year, 2027);
    // Default extrapolation = anchor + 365 days.
    assert_eq!(cal.start_date, NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
}

#[test]
fn season_calendar_missing_returns_none() {
    let (_dir, store) = fresh_store();
    let cal = store
        .get_season_calendar(SeasonId(2099))
        .expect("read");
    assert!(cal.is_none());
}
