use nba3k_core::{Coach, CoachAxes, Scheme, HOT_SEAT_THRESHOLD};

#[test]
fn overall_averages_axes_rounded() {
    let mut c = Coach::default();
    c.axes = CoachAxes {
        strategy: 80.0,
        leadership: 70.0,
        mentorship: 60.0,
        knowledge: 90.0,
        team_management: 50.0,
    };
    // average = 70.0
    assert_eq!(c.overall(), 70);
}

#[test]
fn overall_rounds_to_nearest() {
    let mut c = Coach::default();
    c.axes = CoachAxes {
        strategy: 71.0,
        leadership: 71.0,
        mentorship: 71.0,
        knowledge: 71.0,
        team_management: 74.0,
    };
    // average = 71.6 -> 72
    assert_eq!(c.overall(), 72);
}

#[test]
fn overall_clamps_to_99() {
    let mut c = Coach::default();
    c.axes = CoachAxes {
        strategy: 99.0,
        leadership: 99.0,
        mentorship: 99.0,
        knowledge: 99.0,
        team_management: 99.0,
    };
    assert_eq!(c.overall(), 99);
}

#[test]
fn hot_seat_threshold_matches_docs() {
    let mut c = Coach::default();
    // exactly threshold => not hot seat
    c.axes = CoachAxes {
        strategy: HOT_SEAT_THRESHOLD as f32,
        leadership: HOT_SEAT_THRESHOLD as f32,
        mentorship: HOT_SEAT_THRESHOLD as f32,
        knowledge: HOT_SEAT_THRESHOLD as f32,
        team_management: HOT_SEAT_THRESHOLD as f32,
    };
    assert_eq!(c.overall(), HOT_SEAT_THRESHOLD);
    assert!(!c.on_hot_seat());

    // one below => hot seat
    c.axes.strategy -= 5.0;
    c.axes.leadership -= 5.0;
    assert!(c.overall() < HOT_SEAT_THRESHOLD);
    assert!(c.on_hot_seat());
}

#[test]
fn default_for_is_stable() {
    let a = Coach::default_for("BOS");
    let b = Coach::default_for("BOS");
    assert_eq!(a.scheme_offense, b.scheme_offense);
    assert_eq!(a.scheme_defense, b.scheme_defense);
}

#[test]
fn generated_is_deterministic_for_same_key() {
    let a = Coach::generated("BOS", 42);
    let b = Coach::generated("BOS", 42);
    assert_eq!(a.name, b.name);
    assert_eq!(a.scheme_offense, b.scheme_offense);
    assert_eq!(a.scheme_defense, b.scheme_defense);
    assert_eq!(a.overall(), b.overall());
}

#[test]
fn generated_varies_with_key() {
    let a = Coach::generated("BOS", 1);
    let b = Coach::generated("BOS", 2);
    let c = Coach::generated("BOS", 3);
    // At least one of (name, schemes, overall) should differ across 3 keys.
    let differs = a.name != b.name
        || a.name != c.name
        || a.scheme_offense != b.scheme_offense
        || a.scheme_offense != c.scheme_offense
        || a.overall() != b.overall();
    assert!(differs, "generated coaches collapse to identical output across keys");
    let _ = Scheme::Balanced; // ensure import is used
}
