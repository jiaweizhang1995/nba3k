//! Per-season CBA constants. Values are encoded once per league year and
//! consumed by both the scraper (sanity checks) and the M3 trade engine.
//!
//! Sources for 2025-26 figures (see RESEARCH.md item 4):
//!   - Salary cap, tax, aprons, MLEs, BAE, min team salary:
//!     https://www.nba.com/news/nba-salary-cap-set-2025-26-season
//!     https://www.hoopsrumors.com/2025/06/values-of-2025-26-mid-level-bi-annual-exceptions.html
//!   - Trade cash limit:
//!     https://www.hoopsrumors.com/2025/08/cash-sent-received-in-nba-trades-for-2025-26.html
//!
//! All amounts are in `Cents` (i64) — never `f64` for money.
//!
//! `season_id` follows the convention used elsewhere in the codebase:
//! the *ending* year of the season, e.g. 2025-26 → `SeasonId(2026)`.
//!
//! Per-season constants are static, but the struct is `Clone`/`Copy` because
//! callers (including the trade engine) may want to thread a value across
//! evaluations without holding a reference.
//!
//! Add new league years by appending an entry to `LEAGUE_YEARS`.

use crate::{Cents, SeasonId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeagueYear {
    pub season: SeasonId,
    /// Salary cap.
    pub cap: Cents,
    /// Luxury tax line.
    pub tax: Cents,
    /// First apron.
    pub apron_1: Cents,
    /// Second apron — hard cap for teams above it.
    pub apron_2: Cents,
    /// Non-taxpayer Mid-Level Exception (full MLE).
    pub mle_non_taxpayer: Cents,
    /// Taxpayer MLE.
    pub mle_taxpayer: Cents,
    /// Room MLE (cap-room teams).
    pub mle_room: Cents,
    /// Bi-Annual Exception starting salary.
    pub bae: Cents,
    /// Minimum team salary (90% of cap).
    pub min_team_salary: Cents,
    /// Max cash that can be sent or received in trades (separate caps).
    pub max_trade_cash: Cents,
}

impl LeagueYear {
    /// Look up the encoded constants for a season's *ending* year
    /// (e.g. `SeasonId(2026)` → 2025-26 league year). Future seasons that
    /// aren't explicitly encoded auto-extrapolate from the most recent
    /// encoded year using a 5%/year cap-growth rule (matches NBA's recent
    /// trajectory; mild over-estimate for far-future seasons but better
    /// than panicking).
    pub fn for_season(season: SeasonId) -> Option<Self> {
        if let Some(exact) = LEAGUE_YEARS.iter().copied().find(|y| y.season == season) {
            return Some(exact);
        }
        let latest = LEAGUE_YEARS.iter().copied().max_by_key(|y| y.season.0)?;
        if season.0 < latest.season.0 {
            // Past seasons we never encoded — fall back to the earliest known.
            let earliest = LEAGUE_YEARS.iter().copied().min_by_key(|y| y.season.0)?;
            return Some(earliest);
        }
        let years_ahead = season.0.saturating_sub(latest.season.0) as u32;
        let factor = 1.0_f64 + 0.05 * years_ahead as f64;
        let scale = |c: Cents| -> Cents { Cents(((c.0 as f64) * factor).round() as i64) };
        Some(LeagueYear {
            season,
            cap: scale(latest.cap),
            tax: scale(latest.tax),
            apron_1: scale(latest.apron_1),
            apron_2: scale(latest.apron_2),
            mle_non_taxpayer: scale(latest.mle_non_taxpayer),
            mle_taxpayer: scale(latest.mle_taxpayer),
            mle_room: scale(latest.mle_room),
            bae: scale(latest.bae),
            min_team_salary: scale(latest.min_team_salary),
            max_trade_cash: scale(latest.max_trade_cash),
        })
    }

    /// Parse "YYYY-YY" (e.g. "2025-26") into the matching `LeagueYear`.
    pub fn for_label(label: &str) -> Option<Self> {
        let (start, end) = label.split_once('-')?;
        let start: u16 = start.parse().ok()?;
        let end_two: u16 = end.parse().ok()?;
        let end_full = if end_two < 50 {
            2000 + end_two
        } else {
            1900 + end_two
        };
        if end_full != start + 1 {
            return None;
        }
        Self::for_season(SeasonId(end_full))
    }
}

/// Encoded league years. Append, never reorder — the scraper's sanity check
/// reads this table and the trade engine looks up by `SeasonId`.
pub const LEAGUE_YEARS: &[LeagueYear] = &[LeagueYear {
    season: SeasonId(2026),
    cap: Cents(15_464_700_000),
    tax: Cents(18_789_500_000),
    apron_1: Cents(19_594_500_000),
    apron_2: Cents(20_782_400_000),
    mle_non_taxpayer: Cents(1_410_400_000),
    mle_taxpayer: Cents(568_500_000),
    mle_room: Cents(878_100_000),
    bae: Cents(513_400_000),
    min_team_salary: Cents(13_918_200_000),
    max_trade_cash: Cents(796_400_000),
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_2025_26() {
        let ly = LeagueYear::for_season(SeasonId(2026)).expect("2025-26 encoded");
        assert_eq!(ly.cap.as_dollars(), 154_647_000);
        assert_eq!(ly.tax.as_dollars(), 187_895_000);
        assert_eq!(ly.apron_2.as_dollars(), 207_824_000);
        assert!(ly.apron_1 < ly.apron_2);
        assert!(ly.cap < ly.tax);
        assert!(ly.tax < ly.apron_1);
    }

    #[test]
    fn label_parses() {
        let by_season = LeagueYear::for_season(SeasonId(2026)).unwrap();
        let by_label = LeagueYear::for_label("2025-26").unwrap();
        assert_eq!(by_season, by_label);
        assert!(LeagueYear::for_label("2025-99").is_none());
        assert!(LeagueYear::for_label("nope").is_none());
    }
}
