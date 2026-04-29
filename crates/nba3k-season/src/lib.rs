//! Season orchestrator: schedule, phase transitions, standings.
//! M2 fills schedule/standings; M4 fills playoff bracket; M5 fills draft.

pub mod awards;
pub mod career;
pub mod phases;
pub mod playoffs;
pub mod progression_pass;
pub mod schedule;
pub mod standings;

pub use awards::{
    aggregate_season, compute_all_awards, compute_all_defensive, compute_all_nba, compute_all_star,
    compute_coy, compute_dpoy, compute_dpoy_race, compute_mip, compute_mip_race, compute_mvp,
    compute_mvp_race, compute_roy, compute_roy_race, compute_sixth_man, compute_sixth_man_race,
    AllStarRoster, AwardKind, AwardResult, AwardsBundle, PlayerSeason, SeasonAggregate,
    TeamAwardResult,
};
pub use career::{aggregate_career, career_totals, SeasonAvgRow};
pub use phases::{
    advance_day, is_after_trade_deadline, is_after_trade_deadline_for, is_trade_deadline_day,
    is_trade_deadline_day_for, next_phase, regular_season_complete, trade_deadline,
    transitioning_to_offseason, PRESEASON_LAST_DAY, TRADE_DEADLINE,
};
pub use playoffs::{
    compute_finals_mvp, generate_bracket, simulate_series, Bracket, PlayoffRound, Series,
    SeriesResult,
};
pub use progression_pass::{aggregate_season_minutes, run_progression_pass, ProgressionSummary};
pub use schedule::{
    back_to_back_counts, games_per_team, matchups, Schedule, ScheduledGame, SEASON_END,
    SEASON_START,
};
pub use standings::{compare_tiebreakers, Standings, TeamRecord};
