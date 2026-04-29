//! Team chemistry — explainable team-cohesion score in 0.0..=1.0.
//!
//! Components:
//! 1. Positional balance (PG/SG/SF/PF/C coverage at top of rotation).
//! 2. Role distribution (1-2 Stars, healthy spread of Starter/RolePlayer/Bench).
//! 3. Scheme fit (top-8 archetypes vs `Coach.scheme_offense/defense`).
//! 4. Morale average across roster.
//!
//! Output is a `Score` whose `value` ∈ [0, 1]; reasons are signed deltas
//! around a 0.70 baseline so the renderer can show "+0.06 from balanced
//! rotation" / "−0.12 from star stack".

use nba3k_core::{Coach, LeagueSnapshot, Player, PlayerRole, Position, Scheme, TeamId};

use crate::stat_projection::infer_archetype;
use crate::Score;

const BASELINE: f64 = 0.70;
const W_BALANCE: f64 = 0.20;
const W_ROLES: f64 = 0.20;
const W_SCHEME: f64 = 0.20;
const W_MORALE: f64 = 0.20;

/// Compute team chemistry. Returns `Score` with value in `[0.0, 1.0]`.
/// Empty rosters return value = 0.0 with a single "empty roster" reason.
pub fn team_chemistry(snap: &LeagueSnapshot<'_>, team_id: TeamId) -> Score {
    let mut score = Score::new(BASELINE);
    score = score.with_reason("baseline", BASELINE);

    let roster = snap.roster(team_id);
    if roster.is_empty() {
        return Score::new(0.0).with_reason("empty roster", -BASELINE);
    }

    let team = snap.team(team_id);
    let coach = team.map(|t| &t.coach);

    score.add(
        "positional balance",
        positional_balance(&roster) * W_BALANCE,
    );
    score.add("role distribution", role_distribution(&roster) * W_ROLES);
    if let Some(coach) = coach {
        score.add("scheme fit", scheme_fit_team(&roster, coach) * W_SCHEME);
    }
    score.add("morale", morale_avg(&roster) * W_MORALE);

    // Clamp final value to [0, 1].
    score.value = score.value.clamp(0.0, 1.0);
    score.sort_reasons();
    score
}

/// Per-player scheme-fit score in `[-1.0, 1.0]`. Positive = good fit.
/// Used by `team_chemistry` and exposed for per-player explanations.
pub fn scheme_fit(player: &Player, coach: &Coach) -> f32 {
    let archetype = infer_archetype(player);
    let off = scheme_archetype_match(coach.scheme_offense, &archetype);
    let def = defense_match(coach.scheme_defense, player);
    (off + def) / 2.0
}

// ---------------------------------------------------------------------------
// Components — each returns a signed delta scaled around 0 so the
// caller's BASELINE + sum lands in `[0.0, 1.0]` after clamping.
// ---------------------------------------------------------------------------

fn positional_balance(roster: &[&Player]) -> f64 {
    // Look at top-8 by overall — the on-floor rotation. Need ≥1 of each
    // position, no position with ≥4 stacked.
    let mut top: Vec<&&Player> = roster.iter().collect();
    top.sort_by(|a, b| b.overall.cmp(&a.overall));
    top.truncate(8);

    let mut counts = [0u8; 5];
    for p in &top {
        counts[pos_idx(p.primary_position)] += 1;
    }
    let missing = counts.iter().filter(|&&c| c == 0).count() as i32;
    let stacked = counts.iter().filter(|&&c| c >= 4).count() as i32;
    // Each missing pos: -0.4. Each stack: -0.5. Cap at 1.0.
    let raw = -0.4 * missing as f64 - 0.5 * stacked as f64;
    raw.clamp(-1.0, 1.0)
}

fn role_distribution(roster: &[&Player]) -> f64 {
    let stars = roster.iter().filter(|p| p.role == PlayerRole::Star).count() as i32;
    let starters = roster
        .iter()
        .filter(|p| p.role == PlayerRole::Starter)
        .count() as i32;
    let bench = roster
        .iter()
        .filter(|p| {
            matches!(
                p.role,
                PlayerRole::SixthMan | PlayerRole::RolePlayer | PlayerRole::BenchWarmer
            )
        })
        .count() as i32;

    let mut delta: f64 = 0.0;
    // Sweet spot: 1-2 stars, 3-5 starters, 5+ bench. Star-stacks hurt
    // hard — 2K MyGM "too many alphas" tooltip is a known archetype.
    delta += match stars {
        0 => -0.4,
        1 | 2 => 0.3,
        3 => -0.6,
        4 => -1.2,
        _ => -1.8, // 5+ stars: roster meltdown
    };
    delta += if (3..=5).contains(&starters) {
        0.2
    } else {
        -0.2
    };
    delta += if bench >= 5 { 0.1 } else { -0.1 };

    // Star slotted as BenchWarmer = chemistry tank.
    let mismatched_stars = roster
        .iter()
        .filter(|p| p.overall >= 88 && matches!(p.role, PlayerRole::BenchWarmer))
        .count() as i32;
    delta -= 0.5 * mismatched_stars as f64;

    delta.clamp(-1.0, 1.0)
}

fn scheme_fit_team(roster: &[&Player], coach: &Coach) -> f64 {
    let mut top: Vec<&&Player> = roster.iter().collect();
    top.sort_by(|a, b| b.overall.cmp(&a.overall));
    top.truncate(8);
    if top.is_empty() {
        return 0.0;
    }
    let sum: f32 = top.iter().map(|p| scheme_fit(p, coach)).sum();
    (sum / top.len() as f32) as f64
}

fn morale_avg(roster: &[&Player]) -> f64 {
    if roster.is_empty() {
        return 0.0;
    }
    let sum: f32 = roster.iter().map(|p| p.morale).sum();
    let avg = sum / roster.len() as f32;
    // Map [0..1] morale to delta in [-0.5, +0.5]. Average morale 0.5 = 0 delta.
    ((avg - 0.5) * 1.0) as f64
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pos_idx(p: Position) -> usize {
    match p {
        Position::PG => 0,
        Position::SG => 1,
        Position::SF => 2,
        Position::PF => 3,
        Position::C => 4,
    }
}

/// Map a coach scheme to an archetype-fit score in [-1, 1].
fn scheme_archetype_match(scheme: Scheme, archetype: &str) -> f32 {
    let (good, bad): (&[&str], &[&str]) = match scheme {
        Scheme::PaceAndSpace => (
            &["SG-shooter", "PG-distributor", "SF-3and-d", "PF-stretch"],
            &["C-rim-protector", "PF-bruiser"],
        ),
        Scheme::PostCentric => (
            &["C-rim-protector", "PF-bruiser", "SF-slasher"],
            &["SG-shooter", "PG-distributor"],
        ),
        Scheme::PerimeterCentric => (
            &["SG-shooter", "PG-scorer", "SF-3and-d"],
            &["C-rim-protector", "PF-bruiser"],
        ),
        Scheme::SevenSeconds => (
            &["PG-distributor", "PG-scorer", "SG-shooter", "SF-slasher"],
            &["C-rim-protector"],
        ),
        Scheme::Triangle => (
            &["SG-shooter", "PF-stretch", "C-rim-protector"],
            &["PG-distributor"],
        ),
        Scheme::GritAndGrind => (
            &["PF-bruiser", "C-rim-protector", "SF-3and-d"],
            &["PG-distributor"],
        ),
        Scheme::Defense | Scheme::Balanced => (&[], &[]),
    };
    if good.iter().any(|a| *a == archetype) {
        0.6
    } else if bad.iter().any(|a| *a == archetype) {
        -0.4
    } else {
        0.0
    }
}

/// Defensive scheme fit. `Defense` rewards high-perimeter / interior defense;
/// other schemes neutral.
fn defense_match(scheme: Scheme, player: &Player) -> f32 {
    if !matches!(scheme, Scheme::Defense | Scheme::GritAndGrind) {
        return 0.0;
    }
    let r = &player.ratings;
    let d =
        (r.interior_defense as i32 + r.perimeter_defense as i32 + r.steal as i32 + r.block as i32)
            / 4;
    if d >= 80 {
        0.6
    } else if d >= 70 {
        0.2
    } else {
        -0.3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba3k_core::{
        Conference, Division, DraftPickId, GMArchetype, GMPersonality, LeagueYear, Player,
        PlayerId, PlayerRole, Position, Ratings, SeasonId, SeasonPhase, Team, TeamId,
        TeamRecordSummary,
    };
    use std::collections::HashMap;

    fn make_player(id: u32, ovr: u8, pos: Position, role: PlayerRole, team: u8) -> Player {
        Player {
            id: PlayerId(id),
            name: format!("P{}", id),
            primary_position: pos,
            secondary_position: None,
            age: 27,
            overall: ovr,
            potential: ovr,
            ratings: Ratings::legacy(70, 70, 70, 70, 70, 70, 70, 70),
            contract: None,
            team: Some(TeamId(team)),
            injury: None,
            no_trade_clause: false,
            trade_kicker_pct: None,
            role,
            morale: 0.6,
        }
    }

    fn make_team(id: u8, abbrev: &str) -> Team {
        Team {
            id: TeamId(id),
            abbrev: abbrev.to_string(),
            city: "City".to_string(),
            name: "Team".to_string(),
            conference: Conference::East,
            division: Division::Atlantic,
            gm: GMPersonality::from_archetype(abbrev, GMArchetype::Conservative),
            roster: vec![],
            draft_picks: vec![],
            coach: Coach::default_for(abbrev),
        }
    }

    fn run_team_chemistry(team: Team, players: Vec<Player>) -> Score {
        let team_id = team.id;
        let teams = vec![team];
        let mut p_map: HashMap<PlayerId, Player> = HashMap::new();
        for p in players {
            p_map.insert(p.id, p);
        }
        let st: HashMap<TeamId, TeamRecordSummary> = HashMap::new();
        let picks: HashMap<DraftPickId, nba3k_core::DraftPick> = HashMap::new();
        let snap = LeagueSnapshot {
            current_season: SeasonId(2026),
            current_phase: SeasonPhase::Regular,
            current_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            league_year: LeagueYear::for_season(SeasonId(2026)).expect("2025-26 encoded"),
            teams: &teams,
            players_by_id: &p_map,
            picks_by_id: &picks,
            standings: &st,
        };
        team_chemistry(&snap, team_id)
    }

    #[test]
    fn balanced_rotation_scores_above_baseline() {
        let team = make_team(1, "BOS");
        let mut players = vec![];
        let positions = [
            Position::PG,
            Position::SG,
            Position::SF,
            Position::PF,
            Position::C,
        ];
        for (i, &pos) in positions.iter().enumerate() {
            players.push(make_player(i as u32 + 1, 80, pos, PlayerRole::Starter, 1));
        }
        players[0].role = PlayerRole::Star;
        for (i, &pos) in positions.iter().take(4).enumerate() {
            players.push(make_player(
                20 + i as u32,
                70,
                pos,
                PlayerRole::RolePlayer,
                1,
            ));
        }
        let s = run_team_chemistry(team, players);
        assert!(
            s.value >= 0.65,
            "balanced roster should score ≥0.65, got {}",
            s.value
        );
    }

    #[test]
    fn star_stack_penalizes() {
        let team = make_team(1, "BOS");
        let mut players = vec![];
        for i in 0..5 {
            players.push(make_player(i, 92, Position::SG, PlayerRole::Star, 1));
        }
        for i in 0..3 {
            players.push(make_player(
                10 + i,
                70,
                Position::SF,
                PlayerRole::RolePlayer,
                1,
            ));
        }
        let s = run_team_chemistry(team, players);
        assert!(
            s.value < 0.65,
            "star-stack roster should score below 0.65, got {}",
            s.value
        );
    }

    #[test]
    fn star_in_bench_role_tanks_chemistry() {
        let team = make_team(1, "BOS");
        let mut players = vec![];
        let positions = [
            Position::PG,
            Position::SG,
            Position::SF,
            Position::PF,
            Position::C,
        ];
        for (i, &pos) in positions.iter().enumerate() {
            players.push(make_player(i as u32 + 1, 80, pos, PlayerRole::Starter, 1));
        }
        let mut bench_star = make_player(99, 92, Position::SG, PlayerRole::BenchWarmer, 1);
        bench_star.morale = 0.2;
        players.push(bench_star);
        let s = run_team_chemistry(team, players);
        assert!(
            s.value < 0.6,
            "bench-star team should drop below 0.6, got {}",
            s.value
        );
    }
}
