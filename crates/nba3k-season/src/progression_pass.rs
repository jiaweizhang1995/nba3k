//! Season-end progression pass.
//!
//! Walks every active player at the OffSeason transition, looks up their
//! total minutes played for the season, and runs the
//! `nba3k_models::progression` engine on each in turn. Pure orchestration
//! — no I/O. The orchestrator (CLI) is responsible for loading players +
//! development records from the Store, calling `run_progression_pass`,
//! and persisting the mutated structs back.

use nba3k_core::{BoxScore, GameResult, Player, PlayerId, SeasonId};
use nba3k_models::progression::{apply_progression_step, PlayerDevelopment};
use std::collections::HashMap;

/// Aggregate per-player minutes from a season's worth of game results.
/// Only regular-season games count toward progression — playoff minutes
/// are bonus reps but the engine already biases growth via `mins_played`.
/// Worker D's playoff sims will set `is_playoffs = true`; we skip those
/// here to avoid double-counting in mixed-result inputs.
pub fn aggregate_season_minutes(games: &[GameResult]) -> HashMap<PlayerId, u32> {
    let mut totals: HashMap<PlayerId, u32> = HashMap::new();
    for g in games {
        if g.is_playoffs {
            continue;
        }
        accumulate_box(&g.box_score, &mut totals);
    }
    totals
}

fn accumulate_box(bs: &BoxScore, totals: &mut HashMap<PlayerId, u32>) {
    for line in bs.home_lines.iter().chain(bs.away_lines.iter()) {
        *totals.entry(line.player).or_insert(0) += line.minutes as u32;
    }
}

/// Result of one season-end progression pass — useful for telemetry,
/// CLI summaries, and tests.
#[derive(Debug, Clone, Default)]
pub struct ProgressionSummary {
    /// Number of players touched.
    pub processed: u32,
    /// Total |signed delta| sum across all players. Coarse measure of
    /// how much aggregate movement the league saw.
    pub total_signed_delta: i32,
    /// Players whose dynamic_potential was revised. Useful for narrative
    /// callouts ("Player X's projection has slipped").
    pub potential_revisions: u32,
}

/// Apply the season-end progression pass to a slice of players. Each
/// `players[i]` must have a matching `devs[i]` in the same order; the
/// function mutates both in place. `minutes` provides per-player
/// minutes for the season just played; missing players default to 0
/// (treated as "did not play").
///
/// `next_season` is the season the player is *about to enter* — passed
/// to `dev.last_progressed_season` so a re-run within the same season
/// is a no-op upstream. The age tick happens here as well: each
/// player's `age` is incremented before progression, so a 22-year-old
/// who just finished the season enters the engine as 23.
pub fn run_progression_pass(
    players: &mut [Player],
    devs: &mut [PlayerDevelopment],
    minutes: &HashMap<PlayerId, u32>,
    next_season: SeasonId,
) -> ProgressionSummary {
    debug_assert_eq!(players.len(), devs.len(), "players and devs must align");

    let mut summary = ProgressionSummary::default();
    for (player, dev) in players.iter_mut().zip(devs.iter_mut()) {
        if dev.last_progressed_season >= next_season {
            // Already processed this season — guard against double-apply.
            continue;
        }
        let prior_dyn_potential = dev.dynamic_potential;
        // Tick age first: progression runs against the upcoming season.
        player.age = player.age.saturating_add(1);
        let mins = minutes.get(&player.id).copied().unwrap_or(0);
        let signed = apply_progression_step(player, dev, mins, player.age, next_season);

        summary.processed += 1;
        summary.total_signed_delta += signed;
        if dev.dynamic_potential != prior_dyn_potential {
            summary.potential_revisions += 1;
        }
    }
    summary
}
