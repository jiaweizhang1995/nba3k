//! Tests for `personality::load_personalities` + the embedded fallback.

use nba3k_core::{GMArchetype, GMPersonality};
use nba3k_trade::personality::{
    embedded_personalities, load_or_embedded, load_personalities, personality_for,
    NBA_TEAM_ABBREVS,
};
use std::path::PathBuf;

fn data_path() -> PathBuf {
    // Workspace root → data/personalities.toml.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
        .join("personalities.toml")
}

#[test]
fn all_30_nba_teams_present_in_toml() {
    let file = load_personalities(&data_path()).expect("personalities.toml loads cleanly");
    assert_eq!(file.len(), 30, "expected exactly 30 entries");
    for abbrev in NBA_TEAM_ABBREVS {
        assert!(
            file.get(abbrev).is_some(),
            "team `{}` missing from personalities.toml",
            abbrev
        );
    }
}

#[test]
fn cheapskate_tax_aversion_exceeds_conservative_baseline() {
    // Regression on archetype seeding — Cheapskate must out-bias Conservative
    // on tax avoidance, no matter how the TOML overrides traits.
    let cheap = GMPersonality::from_archetype("baseline", GMArchetype::Cheapskate);
    let conservative = GMPersonality::from_archetype("baseline", GMArchetype::Conservative);
    assert!(
        cheap.traits.tax_aversion > conservative.traits.tax_aversion,
        "Cheapskate.tax_aversion ({}) must exceed Conservative.tax_aversion ({})",
        cheap.traits.tax_aversion,
        conservative.traits.tax_aversion,
    );

    // Also check that any Cheapskate-archetype team in the seeded TOML
    // retains the dominance after override merge.
    let file = load_personalities(&data_path()).expect("loads");
    for (abbrev, personality) in &file.by_abbrev {
        if matches!(personality.archetype, GMArchetype::Cheapskate) {
            assert!(
                personality.traits.tax_aversion > conservative.traits.tax_aversion,
                "{} (Cheapskate) tax_aversion={} should still exceed Conservative baseline {}",
                abbrev,
                personality.traits.tax_aversion,
                conservative.traits.tax_aversion,
            );
        }
    }
}

#[test]
fn embedded_fallback_covers_every_team() {
    let file = embedded_personalities();
    assert_eq!(file.len(), 30);
    for abbrev in NBA_TEAM_ABBREVS {
        let p = file.get(abbrev).expect("present in fallback");
        assert!(matches!(p.archetype, GMArchetype::Conservative));
    }
}

#[test]
fn load_or_embedded_returns_fallback_when_path_missing() {
    let bogus = PathBuf::from("/tmp/nba3k-nonexistent-personalities-xyz.toml");
    assert!(!bogus.exists(), "test precondition: path must not exist");
    let file = load_or_embedded(&bogus).expect("fallback succeeds");
    assert_eq!(file.len(), 30);
    // Sanity check one team — should be Conservative.
    let bos = file.get("BOS").expect("BOS in fallback");
    assert!(matches!(bos.archetype, GMArchetype::Conservative));
}

#[test]
fn load_or_embedded_uses_real_data_when_path_exists() {
    let file = load_or_embedded(&data_path()).expect("real path loads");
    let bos = file.get("BOS").expect("BOS present");
    // Real seed says Boston is WinNow, not the fallback Conservative.
    assert!(matches!(bos.archetype, GMArchetype::WinNow));
}

#[test]
fn personality_for_falls_back_for_unknown_abbrev() {
    let file = embedded_personalities();
    let unknown = personality_for("ZZZ", &file);
    assert!(matches!(unknown.archetype, GMArchetype::Conservative));
    assert_eq!(unknown.name, "ZZZ");
}

#[test]
fn override_actually_overrides_archetype_default() {
    // BRK is Rebuilder; the TOML overrides pick_value_multiplier to 1.6.
    let file = load_personalities(&data_path()).expect("loads");
    let brk = file.get("BRK").expect("BRK present");
    assert!(matches!(brk.archetype, GMArchetype::Rebuilder));
    assert!(
        (brk.traits.pick_value_multiplier - 1.6).abs() < 1e-5,
        "BRK pick_value_multiplier override expected 1.6, got {}",
        brk.traits.pick_value_multiplier,
    );
}
