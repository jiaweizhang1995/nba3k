//! M16-A NBA Cup: store-layer round-trip + bracket-shape invariants.
//!
//! These tests exercise the store API on its own — the day-30/45/53/55
//! triggers in the CLI are tested separately via the integration smoke
//! suite. Here we just confirm the cup_match table accepts and replays
//! all four rounds in order, and that the canonical 30-team layout
//! produces 60 group matches + 7 KO matches.

use nba3k_core::{SeasonId, TeamId};
use nba3k_store::Store;
use tempfile::tempdir;

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("cup.db");
    let store = Store::open(&path).expect("open");
    (dir, store)
}

/// Round-robin pairings for a 5-team group. Each team plays 4 — every
/// other team — so the group totals 10 matches (= 5 choose 2).
fn group_round_robin(teams: &[TeamId]) -> Vec<(TeamId, TeamId)> {
    let mut out = Vec::new();
    for i in 0..teams.len() {
        for j in (i + 1)..teams.len() {
            out.push((teams[i], teams[j]));
        }
    }
    out
}

#[test]
fn empty_save_has_no_cup_matches() {
    let (_dir, store) = fresh_store();
    let rows = store.read_cup_matches(SeasonId(2025)).unwrap();
    assert!(rows.is_empty(), "fresh save reports zero cup matches");
}

#[test]
fn group_stage_round_robin_records_60_matches_for_30_teams() {
    let (_dir, store) = fresh_store();
    let season = SeasonId(2025);

    // 30 teams sorted by id, partitioned into 6 groups of 5 (3 East + 3 West)
    // exactly as the day-30 trigger does.
    let groups: Vec<(&str, Vec<TeamId>)> = vec![
        ("east-A", (1..=5).map(TeamId).collect()),
        ("east-B", (6..=10).map(TeamId).collect()),
        ("east-C", (11..=15).map(TeamId).collect()),
        ("west-A", (16..=20).map(TeamId).collect()),
        ("west-B", (21..=25).map(TeamId).collect()),
        ("west-C", (26..=30).map(TeamId).collect()),
    ];

    for (gid, teams) in &groups {
        for (home, away) in group_round_robin(teams) {
            store
                .record_cup_match(season, "group", Some(gid), home, away, 100, 95, 30)
                .expect("record group match");
        }
    }

    let rows = store.read_cup_matches(season).unwrap();
    let group_rows: Vec<_> = rows.iter().filter(|r| r.round == "group").collect();
    assert_eq!(
        group_rows.len(),
        60,
        "6 groups × 10 matches per round-robin = 60 group matches",
    );

    // Each group contributes exactly 10 rows.
    for (gid, _) in &groups {
        let n = group_rows
            .iter()
            .filter(|r| r.group_id.as_deref() == Some(gid))
            .count();
        assert_eq!(n, 10, "group {} should record 10 matches", gid);
    }
}

#[test]
fn ko_bracket_records_qf_sf_final_with_correct_widths() {
    let (_dir, store) = fresh_store();
    let season = SeasonId(2025);

    // QF: 8 teams → 4 matches.
    let qf: Vec<(TeamId, TeamId)> = (1..=4).map(|i| (TeamId(i), TeamId(9 - i))).collect();
    for (h, a) in &qf {
        store
            .record_cup_match(season, "qf", None, *h, *a, 110, 100, 45)
            .expect("record qf");
    }

    // SF: 4 → 2.
    let sf: Vec<(TeamId, TeamId)> = vec![(TeamId(1), TeamId(4)), (TeamId(2), TeamId(3))];
    for (h, a) in &sf {
        store
            .record_cup_match(season, "sf", None, *h, *a, 105, 99, 53)
            .expect("record sf");
    }

    // Final: 1 match.
    store
        .record_cup_match(season, "final", None, TeamId(1), TeamId(2), 112, 108, 55)
        .expect("record final");

    let rows = store.read_cup_matches(season).unwrap();
    let qf_rows: Vec<_> = rows.iter().filter(|r| r.round == "qf").collect();
    let sf_rows: Vec<_> = rows.iter().filter(|r| r.round == "sf").collect();
    let final_rows: Vec<_> = rows.iter().filter(|r| r.round == "final").collect();
    assert_eq!(qf_rows.len(), 4, "QF: 8 → 4 matches");
    assert_eq!(sf_rows.len(), 2, "SF: 4 → 2 matches");
    assert_eq!(final_rows.len(), 1, "Final: 2 → 1 match");

    // Total KO matches across the bracket.
    let ko = qf_rows.len() + sf_rows.len() + final_rows.len();
    assert_eq!(ko, 7, "8-team single-elim KO has 7 matches total");

    // KO rows must have NULL group_id; group rows carry one.
    for row in rows.iter().filter(|r| r.round != "group") {
        assert!(row.group_id.is_none(), "KO rows leave group_id NULL");
    }
}

#[test]
fn read_cup_matches_preserves_insertion_order() {
    let (_dir, store) = fresh_store();
    let season = SeasonId(2025);

    // Insert one row per round in order.
    store
        .record_cup_match(
            season,
            "group",
            Some("east-A"),
            TeamId(1),
            TeamId(2),
            100,
            90,
            30,
        )
        .unwrap();
    store
        .record_cup_match(season, "qf", None, TeamId(1), TeamId(2), 110, 100, 45)
        .unwrap();
    store
        .record_cup_match(season, "sf", None, TeamId(1), TeamId(2), 99, 95, 53)
        .unwrap();
    store
        .record_cup_match(season, "final", None, TeamId(1), TeamId(2), 112, 108, 55)
        .unwrap();

    let rounds: Vec<String> = store
        .read_cup_matches(season)
        .unwrap()
        .into_iter()
        .map(|r| r.round)
        .collect();
    assert_eq!(rounds, vec!["group", "qf", "sf", "final"]);
}
