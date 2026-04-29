use crate::{SeasonId, TeamId};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeasonPhase {
    PreSeason,
    Regular,
    TradeDeadlinePassed,
    Playoffs,
    OffSeason,
    Draft,
    FreeAgency,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameMode {
    #[default]
    Standard,
    God,
    Hardcore,
    Sandbox,
}

impl GameMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "standard" | "std" => Some(Self::Standard),
            "god" => Some(Self::God),
            "hardcore" | "hc" => Some(Self::Hardcore),
            "sandbox" | "sb" => Some(Self::Sandbox),
            _ => None,
        }
    }

    pub fn enforces_cba(self) -> bool {
        matches!(self, Self::Standard | Self::Hardcore)
    }
}

impl std::fmt::Display for GameMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Standard => "standard",
                Self::God => "god",
                Self::Hardcore => "hardcore",
                Self::Sandbox => "sandbox",
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonState {
    pub season: SeasonId,
    pub phase: SeasonPhase,
    /// Sim day index from start of season.
    pub day: u32,
    pub user_team: TeamId,
    pub mode: GameMode,
    /// Seedable RNG for deterministic replays.
    pub rng_seed: u64,
}

/// Per-season calendar. Drives `Schedule::generate_with_dates`, the
/// trade-deadline phase check, and the all-star / cup day triggers.
/// Lives in the `season_calendar` table (one row per season_year).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonCalendar {
    pub season_year: u16,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub trade_deadline: NaiveDate,
    pub all_star_day: u32,
    pub cup_group_day: u32,
    pub cup_qf_day: u32,
    pub cup_sf_day: u32,
    pub cup_final_day: u32,
}

impl SeasonCalendar {
    /// Hardcoded 2025-26 calendar, used as fallback when no row exists.
    /// Mirrors the SQL `INSERT` in V016 and the legacy `SEASON_START` /
    /// `SEASON_END` constants in `nba3k_season::schedule`.
    pub fn default_for(season_year: u16) -> Self {
        // For 2025-26 we use the precise real-world dates. For other years
        // we extrapolate by 365 days off the 2025-26 anchors so callers
        // never get a panic; M33's `season-advance` writes a real row for
        // each new year so this fallback is only used for fresh saves.
        let anchor_year = 2026_i32;
        let anchor_start = NaiveDate::from_ymd_opt(2025, 10, 21).expect("anchor start");
        let anchor_end = NaiveDate::from_ymd_opt(2026, 4, 12).expect("anchor end");
        let anchor_deadline = NaiveDate::from_ymd_opt(2026, 2, 5).expect("anchor deadline");
        let delta_years = season_year as i32 - anchor_year;
        let shift = chrono::Duration::days(delta_years as i64 * 365);
        Self {
            season_year,
            start_date: anchor_start + shift,
            end_date: anchor_end + shift,
            trade_deadline: anchor_deadline + shift,
            all_star_day: 41,
            cup_group_day: 30,
            cup_qf_day: 45,
            cup_sf_day: 53,
            cup_final_day: 55,
        }
    }
}
