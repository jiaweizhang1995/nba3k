//! Acceptance tests for the M4 stat-projection model.
//!
//! User principle: "stars produce star box scores, role players don't".
//! These tests pin that intuition with statistical guards over many sims:
//!   - franchise-tagged 92-OVR scoring PG averages a star line
//!   - bench role player averages a bench line
//!   - triple-doubles emerge from primary creators (≥ 5%)
//!   - triple-doubles do NOT emerge from non-star wings (≤ 1%)
//!   - same seed + same input → identical PlayerLine
//!   - file load works; missing file falls back to neutral profile

use chrono::NaiveDate;
use nba3k_core::{
    Contract, ContractYear, InjuryStatus, PlayerId, PlayerRole, Position, Ratings, SeasonId,
    TeamId, Player, BirdRights, Cents,
};
use nba3k_models::stat_projection::{
    infer_archetype, load_archetype_profiles, project_player_line, ArchetypeProfiles,
    StatProjectionInput, ARCHETYPE_PROFILES_PATH,
};
use nba3k_models::star_protection::{load_star_roster, StarRoster, STAR_ROSTER_PATH};
use nba3k_models::weights::StatProjectionWeights;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::path::{Path, PathBuf};

const N_GAMES: usize = 200;

// ---------------------------------------------------------------------------
// Test helpers.
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().expect("crates/")
        .parent().expect("workspace root")
        .to_path_buf()
}

fn star_pg(name: &str, ovr: u8) -> Player {
    Player {
        id: PlayerId(1001),
        name: name.to_string(),
        primary_position: Position::PG,
        secondary_position: Some(Position::SG),
        age: 26,
        overall: ovr,
        potential: 95,
        ratings: Ratings::legacy(88, 82, 86, 92, 70, 70, 55, 80),
        contract: Some(simple_contract()),
        team: Some(TeamId(14)), // LAL
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

fn bench_guard() -> Player {
    Player {
        id: PlayerId(2001),
        name: "Reserve Joe".to_string(),
        primary_position: Position::SG,
        secondary_position: Some(Position::PG),
        age: 28,
        overall: 72,
        potential: 74,
        ratings: Ratings::legacy(68, 70, 65, 68, 50, 70, 50, 70),
        contract: Some(simple_contract()),
        team: Some(TeamId(7)),
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

fn untagged_wing() -> Player {
    Player {
        id: PlayerId(3001),
        name: "Wing Rotation".to_string(),
        primary_position: Position::SF,
        secondary_position: Some(Position::SG),
        age: 27,
        overall: 78,
        potential: 80,
        ratings: Ratings::legacy(80, 70, 72, /*playmaking*/ 65, 65, 82, 60, 80),
        contract: Some(simple_contract()),
        team: Some(TeamId(11)),
        injury: None,
        no_trade_clause: false,
        trade_kicker_pct: None,
        role: PlayerRole::RolePlayer,
        morale: 0.5,
    }
}

fn simple_contract() -> Contract {
    Contract {
        years: vec![ContractYear {
            season: SeasonId(2026),
            salary: Cents::from_dollars(20_000_000),
            guaranteed: true,
            team_option: false,
            player_option: false,
        }],
        signed_in_season: SeasonId(2025),
        bird_rights: BirdRights::Full,
    }
}

fn load_profiles_or_default() -> ArchetypeProfiles {
    let path = workspace_root().join(ARCHETYPE_PROFILES_PATH);
    load_archetype_profiles(&path).expect("archetype_profiles.toml must parse")
}

fn load_star_roster_for_test() -> StarRoster {
    let path = workspace_root().join(STAR_ROSTER_PATH);
    load_star_roster(&path).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn star_scoring_pg_averages_star_line() {
    let profiles = load_profiles_or_default();
    let roster_path = workspace_root().join(STAR_ROSTER_PATH);
    let mut roster = load_star_roster(&roster_path).unwrap_or_default();
    // Tag the synthetic player so star uplift fires deterministically.
    roster
        .by_team
        .entry("LAL".to_string())
        .or_default()
        .push("Star PG".to_string());

    let weights = StatProjectionWeights::default();
    let player = star_pg("Star PG", 92);
    let mut rng = ChaCha8Rng::seed_from_u64(20251101);

    let mut total_pts = 0u32;
    let mut max_pts = 0u8;
    let mut min_pts = u8::MAX;
    for _ in 0..N_GAMES {
        let line = project_player_line(
            StatProjectionInput {
                player: &player,
                minutes: 36,
                team_pace: 100.0,
                usage_share: 0.30,
                archetype: "PG-scorer",
                date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
                team_abbrev: "LAL",
            },
            &profiles,
            &weights,
            &roster,
            &mut rng,
        );
        total_pts += line.pts as u32;
        max_pts = max_pts.max(line.pts);
        min_pts = min_pts.min(line.pts);
    }
    let avg_pts = total_pts as f32 / N_GAMES as f32;

    // Expected band: [22, 40] per spec. Real Luka 2023-24 was 33.9 PPG.
    assert!(
        avg_pts >= 22.0 && avg_pts <= 40.0,
        "star PG avg PTS {} outside [22, 40]; min={}, max={}",
        avg_pts, min_pts, max_pts
    );
    // Variance check — not always 30. Range of (max - min) should be > 8.
    assert!(
        max_pts as i32 - min_pts as i32 > 8,
        "star PG should show variance: min={} max={}",
        min_pts, max_pts
    );
}

#[test]
fn bench_player_averages_low_pts() {
    let profiles = load_profiles_or_default();
    let roster = load_star_roster_for_test();
    let weights = StatProjectionWeights::default();
    let player = bench_guard();
    let mut rng = ChaCha8Rng::seed_from_u64(424242);

    let mut total_pts = 0u32;
    for _ in 0..N_GAMES {
        let line = project_player_line(
            StatProjectionInput {
                player: &player,
                minutes: 14,
                team_pace: 100.0,
                usage_share: 0.10,
                archetype: "SG-shooter",
                date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
                team_abbrev: "POR",
            },
            &profiles,
            &weights,
            &roster,
            &mut rng,
        );
        total_pts += line.pts as u32;
    }
    let avg_pts = total_pts as f32 / N_GAMES as f32;
    assert!(
        avg_pts <= 12.0,
        "bench PG avg PTS {} should be ≤ 12",
        avg_pts
    );
}

#[test]
fn star_creator_triple_double_rate_meets_floor() {
    let profiles = load_profiles_or_default();
    let mut roster = load_star_roster_for_test();
    roster
        .by_team
        .entry("LAL".to_string())
        .or_default()
        .push("Star PG".to_string());

    let weights = StatProjectionWeights::default();
    let player = star_pg("Star PG", 95);
    let mut rng = ChaCha8Rng::seed_from_u64(777_001);

    let mut td_count = 0u32;
    for _ in 0..N_GAMES {
        let line = project_player_line(
            StatProjectionInput {
                player: &player,
                minutes: 38,
                team_pace: 102.0,
                usage_share: 0.32,
                archetype: "PG-scorer",
                date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
                team_abbrev: "LAL",
            },
            &profiles,
            &weights,
            &roster,
            &mut rng,
        );
        if line.pts >= 10 && line.reb >= 10 && line.ast >= 10 {
            td_count += 1;
        }
    }
    let td_rate = td_count as f32 / N_GAMES as f32;
    assert!(
        td_rate >= 0.05,
        "primary-creator triple-double rate {:.3} should be ≥ 0.05 ({} TDs / {})",
        td_rate, td_count, N_GAMES
    );
}

#[test]
fn non_star_wing_triple_double_rate_under_ceiling() {
    let profiles = load_profiles_or_default();
    let roster = load_star_roster_for_test();
    let weights = StatProjectionWeights::default();
    let player = untagged_wing();
    let mut rng = ChaCha8Rng::seed_from_u64(1234567);

    let mut td_count = 0u32;
    for _ in 0..N_GAMES {
        let line = project_player_line(
            StatProjectionInput {
                player: &player,
                minutes: 32,
                team_pace: 100.0,
                usage_share: 0.18,
                archetype: "SF-3andD",
                date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
                team_abbrev: "CHI",
            },
            &profiles,
            &weights,
            &roster,
            &mut rng,
        );
        if line.pts >= 10 && line.reb >= 10 && line.ast >= 10 {
            td_count += 1;
        }
    }
    let td_rate = td_count as f32 / N_GAMES as f32;
    assert!(
        td_rate <= 0.01,
        "non-star wing triple-double rate {:.3} should be ≤ 0.01 ({} TDs / {})",
        td_rate, td_count, N_GAMES
    );
}

#[test]
fn deterministic_same_seed_same_line() {
    let profiles = load_profiles_or_default();
    let roster = load_star_roster_for_test();
    let weights = StatProjectionWeights::default();
    let player = star_pg("Star PG", 92);

    let mut rng_a = ChaCha8Rng::seed_from_u64(999);
    let mut rng_b = ChaCha8Rng::seed_from_u64(999);

    let input = StatProjectionInput {
        player: &player,
        minutes: 36,
        team_pace: 100.0,
        usage_share: 0.30,
        archetype: "PG-scorer",
        date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
        team_abbrev: "LAL",
    };

    let a = project_player_line(input, &profiles, &weights, &roster, &mut rng_a);
    let b = project_player_line(input, &profiles, &weights, &roster, &mut rng_b);

    assert_eq!(a.player, b.player);
    assert_eq!(a.minutes, b.minutes);
    assert_eq!(a.pts, b.pts);
    assert_eq!(a.reb, b.reb);
    assert_eq!(a.ast, b.ast);
    assert_eq!(a.stl, b.stl);
    assert_eq!(a.blk, b.blk);
    assert_eq!(a.tov, b.tov);
    assert_eq!(a.fg_made, b.fg_made);
    assert_eq!(a.fg_att, b.fg_att);
    assert_eq!(a.three_made, b.three_made);
    assert_eq!(a.three_att, b.three_att);
    assert_eq!(a.ft_made, b.ft_made);
    assert_eq!(a.ft_att, b.ft_att);
}

#[test]
fn archetype_profiles_load_from_workspace_file() {
    let path = workspace_root().join(ARCHETYPE_PROFILES_PATH);
    let profiles = load_archetype_profiles(&path)
        .expect("archetype_profiles.toml must parse");
    let expected_keys = [
        "PG-distributor", "PG-scorer", "SG-shooter", "SG-slasher",
        "SF-3andD", "SF-creator", "PF-stretch", "PF-banger",
        "C-finisher", "C-stretch",
    ];
    for k in expected_keys {
        assert!(
            profiles.by_archetype.contains_key(k),
            "missing archetype '{}' in archetype_profiles.toml; have: {:?}",
            k, profiles.by_archetype.keys().collect::<Vec<_>>()
        );
        let p = profiles.by_archetype.get(k).unwrap();
        assert!(p.default_usage > 0.05 && p.default_usage < 0.50,
            "archetype '{}' default_usage {} out of [0.05, 0.50]", k, p.default_usage);
        assert!(p.pts_per_100 >= 10.0 && p.pts_per_100 <= 50.0,
            "archetype '{}' pts_per_100 {} out of [10, 50]", k, p.pts_per_100);
    }
}

#[test]
fn missing_file_falls_back_to_empty_profiles() {
    let path = Path::new("/nonexistent/archetype_profiles.toml");
    let profiles = load_archetype_profiles(path)
        .expect("missing file should not error");
    assert!(profiles.by_archetype.is_empty());

    // And `project_player_line` still works without panicking — it should
    // fall back to the neutral profile baked into ArchetypeProfile::default().
    let weights = StatProjectionWeights::default();
    let roster = StarRoster::default();
    let player = bench_guard();
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    let line = project_player_line(
        StatProjectionInput {
            player: &player,
            minutes: 20,
            team_pace: 100.0,
            usage_share: 0.18,
            archetype: "SG-shooter", // not in empty profiles
            date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
            team_abbrev: "POR",
        },
        &profiles,
        &weights,
        &roster,
        &mut rng,
    );
    // Sanity: minutes preserved, stats non-negative implied by u8 type, and
    // no panic. Don't pin specific values — just guard that fallback works.
    assert_eq!(line.minutes, 20);
    assert!(line.pts < 50);
}

#[test]
fn infer_archetype_returns_known_label() {
    let known = [
        "PG-distributor", "PG-scorer", "SG-shooter", "SG-slasher",
        "SF-3andD", "SF-creator", "PF-stretch", "PF-banger",
        "C-finisher", "C-stretch",
    ];

    // PG with very high playmaking → distributor.
    let mut p = star_pg("Test", 85);
    p.ratings.ball_handle = 92;
    p.ratings.three_point = 75;
    p.ratings.driving_layup = 75;
    let arch = infer_archetype(&p);
    assert!(known.contains(&arch.as_str()), "infer returned unknown {}", arch);
    assert_eq!(arch, "PG-distributor");

    // Big with low 3PT → finisher.
    let mut c = star_pg("Test", 85);
    c.primary_position = Position::C;
    c.ratings.three_point = 30;
    c.ratings.driving_layup = 90;
    c.ratings.off_reb = 92;
    c.ratings.def_reb = 92;
    let arch_c = infer_archetype(&c);
    assert_eq!(arch_c, "C-finisher");

    // Big with high 3PT → stretch.
    let mut sc = c.clone();
    sc.ratings.three_point = 80;
    let arch_sc = infer_archetype(&sc);
    assert_eq!(arch_sc, "C-stretch");
}

#[test]
fn injury_throttles_output() {
    let profiles = load_profiles_or_default();
    let roster = load_star_roster_for_test();
    let weights = StatProjectionWeights::default();

    // Healthy.
    let healthy = star_pg("Star PG", 92);
    // Injured (long term).
    let mut hurt = healthy.clone();
    hurt.injury = Some(InjuryStatus {
        description: "knee".into(),
        games_remaining: 20,
        severity: nba3k_core::InjurySeverity::LongTerm,
    });

    let mut rng_h = ChaCha8Rng::seed_from_u64(11);
    let mut rng_i = ChaCha8Rng::seed_from_u64(11);

    let mut healthy_pts = 0u32;
    let mut hurt_pts = 0u32;
    for _ in 0..N_GAMES {
        let lh = project_player_line(
            StatProjectionInput {
                player: &healthy, minutes: 36, team_pace: 100.0,
                usage_share: 0.30, archetype: "PG-scorer",
                date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
                team_abbrev: "LAL",
            },
            &profiles, &weights, &roster, &mut rng_h,
        );
        let li = project_player_line(
            StatProjectionInput {
                player: &hurt, minutes: 36, team_pace: 100.0,
                usage_share: 0.30, archetype: "PG-scorer",
                date: NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
                team_abbrev: "LAL",
            },
            &profiles, &weights, &roster, &mut rng_i,
        );
        healthy_pts += lh.pts as u32;
        hurt_pts += li.pts as u32;
    }
    assert!(hurt_pts < healthy_pts,
        "long-term injury should reduce PTS production: hurt={} healthy={}",
        hurt_pts, healthy_pts);
}
