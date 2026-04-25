//! Phase advancement: PreSeason → Regular → Playoffs.

use crate::schedule::Schedule;
use crate::standings::Standings;
use chrono::NaiveDate;
use nba3k_core::{SeasonPhase, SeasonState};

/// Last day of preseason (inclusive). PreSeason runs days 0..=6, switches to
/// Regular on day 7. Mirrors the NBA's roughly 1-week training-camp window.
pub const PRESEASON_LAST_DAY: u32 = 6;

/// 2025-26 NBA trade deadline date.
pub const TRADE_DEADLINE: (i32, u32, u32) = (2026, 2, 5);

/// Compute the next phase given current state, full schedule, and current
/// standings. Pure function — does not mutate.
pub fn next_phase(state: &SeasonState, schedule: &Schedule, standings: &Standings) -> SeasonPhase {
    match state.phase {
        SeasonPhase::PreSeason => {
            if state.day > PRESEASON_LAST_DAY {
                SeasonPhase::Regular
            } else {
                SeasonPhase::PreSeason
            }
        }
        SeasonPhase::Regular | SeasonPhase::TradeDeadlinePassed => {
            if regular_season_complete(schedule, standings) {
                SeasonPhase::Playoffs
            } else {
                state.phase
            }
        }
        other => other,
    }
}

/// Advance one day. Returns the (possibly updated) phase. Does not mutate.
pub fn advance_day(state: &SeasonState, schedule: &Schedule, standings: &Standings) -> SeasonPhase {
    next_phase(state, schedule, standings)
}

/// True when every team has played at least 82 games. We accept "exactly"
/// or "more than" since playoffs may slot before the final regular game in
/// edge cases — but for our schedule generator, every team plays exactly 82.
pub fn regular_season_complete(schedule: &Schedule, standings: &Standings) -> bool {
    let total_per_team = crate::schedule::games_per_team(schedule);
    standings.records.keys().all(|t| {
        let played = standings
            .records
            .get(t)
            .map(|r| r.games_played())
            .unwrap_or(0) as u32;
        played >= *total_per_team.get(t).unwrap_or(&0)
    })
}

/// Whether `date` falls strictly after the 2025-26 NBA trade deadline.
pub fn is_after_trade_deadline(date: NaiveDate) -> bool {
    let deadline = NaiveDate::from_ymd_opt(TRADE_DEADLINE.0, TRADE_DEADLINE.1, TRADE_DEADLINE.2)
        .expect("valid deadline date");
    date > deadline
}

/// Whether `date` is the trade deadline itself (the day the deadline window
/// closes — used by store/CLI to gate Standard-mode trade submission).
pub fn is_trade_deadline_day(date: NaiveDate) -> bool {
    let deadline = NaiveDate::from_ymd_opt(TRADE_DEADLINE.0, TRADE_DEADLINE.1, TRADE_DEADLINE.2)
        .expect("valid deadline date");
    date == deadline
}

/// True when a phase transition is moving from `Playoffs` → `OffSeason`.
/// Used by the orchestrator as the trigger to run the season-end
/// progression pass (M5 Worker C). Playoffs → OffSeason is the only
/// edge that should fire progression; PreSeason → Regular and others
/// should not.
pub fn transitioning_to_offseason(prev: SeasonPhase, next: SeasonPhase) -> bool {
    matches!(prev, SeasonPhase::Playoffs) && matches!(next, SeasonPhase::OffSeason)
}
