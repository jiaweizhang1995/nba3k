//! M18-A owner mandate store-layer tests. Locks down the round-trip the
//! CLI relies on (record/read keyed on (season, team, kind), upsert
//! semantics on collision) plus the small grade-math helper that turns a
//! list of (weight, pass-rate) pairs into the A/B/C/D/F letter the
//! `nba3k mandate` command surfaces at season end.

use nba3k_core::{
    Conference, Division, GMArchetype, GMPersonality, SeasonId, Team, TeamId,
};
use nba3k_store::Store;
use tempfile::tempdir;

const HOME: TeamId = TeamId(1);
const SEASON: SeasonId = SeasonId(2026);

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("mandate.db");
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

/// Re-implementation of the CLI's grade math kept in sync via the test —
/// any drift here is a flag that the grade thresholds rendered by
/// `cmd_mandate` need to be updated too.
fn grade_letter(score: f32) -> &'static str {
    if score >= 0.85 {
        "A"
    } else if score >= 0.70 {
        "B"
    } else if score >= 0.55 {
        "C"
    } else if score >= 0.40 {
        "D"
    } else {
        "F"
    }
}

fn weighted_score(rows: &[(f32, f32)]) -> f32 {
    let total_weight: f32 = rows.iter().map(|(w, _)| *w).sum();
    if total_weight <= 0.0 {
        return 0.0;
    }
    let weighted: f32 = rows.iter().map(|(w, p)| w * p.clamp(0.0, 1.0)).sum();
    weighted / total_weight
}

#[test]
fn record_then_read_returns_rows_in_kind_order() {
    let (_dir, store) = fresh_store();

    store
        .record_mandate(SEASON, HOME, "wins", 50, 0.40)
        .expect("record wins");
    store
        .record_mandate(SEASON, HOME, "make_playoffs", 1, 0.30)
        .expect("record playoffs");
    store
        .record_mandate(SEASON, HOME, "champion", 1, 0.30)
        .expect("record champion");

    let rows = store.read_mandates(SEASON, HOME).expect("read");
    assert_eq!(rows.len(), 3, "three mandates round-trip");

    // Ordered ASC by kind: champion, make_playoffs, wins.
    let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
    assert_eq!(kinds, ["champion", "make_playoffs", "wins"]);

    let wins = rows.iter().find(|r| r.kind == "wins").unwrap();
    assert_eq!(wins.target, 50);
    assert!((wins.weight - 0.40).abs() < 1e-6);
}

#[test]
fn record_same_kind_twice_overwrites_target_and_weight() {
    let (_dir, store) = fresh_store();

    store
        .record_mandate(SEASON, HOME, "wins", 42, 0.50)
        .expect("first");
    store
        .record_mandate(SEASON, HOME, "wins", 50, 0.40)
        .expect("second");

    let rows = store.read_mandates(SEASON, HOME).expect("read");
    assert_eq!(rows.len(), 1, "UPSERT must not duplicate the row");
    assert_eq!(rows[0].target, 50);
    assert!((rows[0].weight - 0.40).abs() < 1e-6);
}

#[test]
fn read_mandates_for_other_team_or_season_returns_empty() {
    let (_dir, store) = fresh_store();
    store
        .record_mandate(SEASON, HOME, "wins", 50, 1.0)
        .expect("seed");

    assert!(
        store
            .read_mandates(SeasonId(SEASON.0 + 1), HOME)
            .unwrap()
            .is_empty(),
        "different season is isolated",
    );
    assert!(
        store
            .read_mandates(SEASON, TeamId(2))
            .unwrap()
            .is_empty(),
        "different team is isolated",
    );
}

#[test]
fn grade_math_matches_weighted_pass_rate() {
    // All targets passed → 1.0 → A.
    let perfect = weighted_score(&[(0.30, 1.0), (0.40, 1.0), (0.30, 1.0)]);
    assert!((perfect - 1.0).abs() < 1e-6);
    assert_eq!(grade_letter(perfect), "A");

    // 0.30 + 0.40 passed, 0.30 failed → 0.70 → B (boundary).
    let mixed = weighted_score(&[(0.30, 1.0), (0.40, 1.0), (0.30, 0.0)]);
    assert!((mixed - 0.70).abs() < 1e-6);
    assert_eq!(grade_letter(mixed), "B");

    // Half pass-rate on a single goal → 0.5 → D.
    let partial = weighted_score(&[(1.0, 0.5)]);
    assert!((partial - 0.5).abs() < 1e-6);
    assert_eq!(grade_letter(partial), "D");

    // Nothing passed → F.
    let busted = weighted_score(&[(0.5, 0.0), (0.5, 0.0)]);
    assert_eq!(grade_letter(busted), "F");

    // Zero weights guard: empty input does not panic and returns 0.0.
    assert!((weighted_score(&[]) - 0.0).abs() < 1e-6);
}
