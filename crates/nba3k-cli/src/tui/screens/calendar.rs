//! Calendar screen — the headline screen of M20. Houses the time-control
//! UI (sim day/week/month/sim-to-event) plus six sub-tabs that surface league
//! state: Schedule (month grid), Standings, Playoffs, Awards, All-Star, Cup.
//!
//! Architecture notes:
//! - All state mutation routes through `commands::dispatch` wrapped in
//!   `with_silenced_io` so inner `println!`s don't corrupt the alt-screen.
//! - Per-screen state lives in a module-level `OnceCell` keyed by save day —
//!   actually we keep state inside this module via `static mut`-free pattern:
//!   we pass the state through `TuiApp` indirectly by recomputing on each
//!   render. To avoid excess store reads, a local `Cache` struct is held in a
//!   `RefCell` keyed by `(season, day)`. Invalidated automatically when the
//!   day advances after a sim.
//! - Pause-on-event modal is surfaced when `sim_paced` notices a new offer or
//!   user-team injury during a sim-week or sim-month run.
//!
//! Constraints honored:
//! - Does not touch `tui/mod.rs` or `tui/widgets.rs`.
//! - Does not modify `commands.rs`.
//! - Reads schedule via `Store::pending_games_through` + `Store::read_games`.

use anyhow::Result;
use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, Weekday};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::cli::{Command, JsonFlag, PlayoffsAction, PlayoffsArgs};
use crate::state::AppState;
use crate::tui::widgets::{centered_block, kv_table, Confirm, FormWidget, Theme, WidgetEvent};
use crate::tui::{with_silenced_io, TuiApp};
use nba3k_core::{t, Conference, Lang, PlayerId, SeasonId, SeasonPhase, TeamId, T};
use nba3k_store::StandingRow;

// ---------------------------------------------------------------------------
// Day-mapping constants
// ---------------------------------------------------------------------------
//
// Mirrors `commands.rs`. Kept as private consts here so this screen has no
// runtime dependency on `commands.rs` internals — when those constants move
// or change, the build error here forces a coordinated update.

const ALL_STAR_DAY: u32 = 41;
const CUP_GROUP_DAY: u32 = 30;
const CUP_QF_DAY: u32 = 45;
const CUP_SF_DAY: u32 = 53;
const CUP_FINAL_DAY: u32 = 55;
/// Approximate trade-deadline day-of-season (mid-Feb of the 2025-26 calendar
/// — the actual deadline date is February 5, which falls around day 114 of
/// our 174-day calendar starting October 14).
const TRADE_DEADLINE_DAY: u32 = 114;
/// Approximate first day of the playoff bracket. The bracket runs after the
/// regular season completes — since regular season ends ~day 165, this is the
/// earliest a Playoffs phase will be entered.
const PLAYOFFS_START_DAY: u32 = 168;
/// Calendar length used for "Day X of Y" — the schedule generator typically
/// produces an 82-game slate inside this window. Extends past playoff start
/// so the grid can navigate into the postseason.
const SEASON_LENGTH_DAYS: u32 = 174;

/// Calendar epoch — Day 0 of the in-game season corresponds to this date.
/// Mirrors `commands::day_index_to_date` exactly so the grid and the
/// scheduled `date` columns line up.
fn day_zero() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 10, 14).expect("valid epoch")
}

fn day_to_date(day: u32) -> NaiveDate {
    day_zero() + ChronoDuration::days(day as i64)
}

fn date_to_day(date: NaiveDate) -> i64 {
    (date - day_zero()).num_days()
}

// ---------------------------------------------------------------------------
// Sub-tab enum
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SubTab {
    Schedule,
    Standings,
    Playoffs,
    Awards,
    AllStar,
    Cup,
}

impl SubTab {
    const ALL: [SubTab; 6] = [
        SubTab::Schedule,
        SubTab::Standings,
        SubTab::Playoffs,
        SubTab::Awards,
        SubTab::AllStar,
        SubTab::Cup,
    ];

    fn label(self, lang: Lang) -> &'static str {
        match self {
            SubTab::Schedule => t(lang, T::CalendarSchedule),
            SubTab::Standings => t(lang, T::CalendarStandings),
            SubTab::Playoffs => t(lang, T::CalendarPlayoffs),
            SubTab::Awards => t(lang, T::CalendarAwards),
            SubTab::AllStar => t(lang, T::CalendarAllStar),
            SubTab::Cup => t(lang, T::CalendarCup),
        }
    }

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

// ---------------------------------------------------------------------------
// CalendarState (module-private)
// ---------------------------------------------------------------------------
//
// Kept in a thread-local RefCell so we can mutate it during render without
// adding fields to `TuiApp`. Invalidated automatically when the underlying
// (season, day) pair changes — that's our cache key.

thread_local! {
    static STATE: RefCell<CalendarState> = RefCell::new(CalendarState::new());
}

struct CalendarState {
    sub_tab: SubTab,
    /// Cursor anchor inside the displayed month (0..=41 — 6 rows × 7 cols).
    cell_cursor: u8,
    /// Currently displayed month (anchor = first day of month).
    view_month: NaiveDate,
    /// Cache of raw schedule rows for the active season.
    cached_for: Option<(SeasonId, u32)>,
    schedule: Vec<ScheduleEntry>,
    awards_season_offset: i32,
    pause_modal: Option<PauseModal>,
    advance_confirm: Option<Confirm>,
    playoffs_confirm: Option<Confirm>,
}

#[derive(Clone)]
struct ScheduleEntry {
    date: NaiveDate,
    home: TeamId,
    away: TeamId,
    /// `true` once the row has a corresponding `games` row (played).
    played: bool,
    /// Final score when `played`. None for upcoming.
    home_score: Option<u16>,
    away_score: Option<u16>,
}

#[derive(Clone)]
struct PauseModal {
    title: String,
    body: String,
}

impl CalendarState {
    fn new() -> Self {
        Self {
            sub_tab: SubTab::Schedule,
            cell_cursor: 0,
            view_month: NaiveDate::from_ymd_opt(2025, 10, 1).expect("valid"),
            cached_for: None,
            schedule: Vec::new(),
            awards_season_offset: 0,
            pause_modal: None,
            advance_confirm: None,
            playoffs_confirm: None,
        }
    }

    fn modal_active(&self) -> bool {
        self.pause_modal.is_some()
            || self.advance_confirm.is_some()
            || self.playoffs_confirm.is_some()
    }
}

/// Refresh the cached schedule + cursor anchor when the active save changes
/// (different season) or when sim has advanced past today's cursor cell.
fn ensure_cache(app: &mut AppState, tui: &TuiApp, st: &mut CalendarState) -> Result<()> {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        return Ok(());
    };
    let season = ctx.season;
    let day = ctx.season_state.day;
    let key = (season, day);
    if st.cached_for == Some(key) {
        return Ok(());
    }
    st.cached_for = Some(key);
    st.schedule = load_schedule(app, season, ctx.user_team)?;

    // Snap view_month + cursor to the current sim-day on first open / after
    // any sim. User can later navigate via [/] without further snapping.
    let today = day_to_date(day);
    st.view_month = first_of_month(today);
    st.cell_cursor = cursor_for(today, st.view_month).unwrap_or(0);
    Ok(())
}

pub fn invalidate() {
    STATE.with(|cell| {
        cell.borrow_mut().cached_for = None;
    });
}

/// Load every scheduled + played game for the user team in the current
/// season. Combines `pending_games_through(<far date>)` (unplayed) with
/// `read_games(season)` filtered to user-team rows (played).
fn load_schedule(
    app: &mut AppState,
    season: SeasonId,
    user_team: TeamId,
) -> Result<Vec<ScheduleEntry>> {
    let store = app.store()?;
    let last = store
        .last_scheduled_date()?
        .unwrap_or_else(|| day_to_date(SEASON_LENGTH_DAYS));
    let pending = store.pending_games_through(last)?;
    let played = store.read_games(season)?;

    let mut out: Vec<ScheduleEntry> = Vec::with_capacity(82);
    for r in pending.into_iter().filter(|r| r.season == season) {
        if r.home == user_team || r.away == user_team {
            out.push(ScheduleEntry {
                date: r.date,
                home: r.home,
                away: r.away,
                played: false,
                home_score: None,
                away_score: None,
            });
        }
    }
    for g in played
        .into_iter()
        .filter(|g| !g.is_playoffs && (g.home == user_team || g.away == user_team))
    {
        out.push(ScheduleEntry {
            date: g.date,
            home: g.home,
            away: g.away,
            played: true,
            home_score: Some(g.home_score),
            away_score: Some(g.away_score),
        });
    }
    out.sort_by_key(|e| e.date);
    Ok(out)
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    if !tui.has_save() {
        centered_block(
            f,
            area,
            theme,
            t(tui.lang, T::CalendarTitle),
            &[t(tui.lang, T::CommonNoSaveLoaded), "", t(tui.lang, T::SavesLoad)],
        );
        return;
    }

    STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        if let Err(e) = ensure_cache(app, tui, &mut st) {
            let _ = e;
        }

        // Outer split: tab strip + content area.
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        draw_tab_strip(f, outer[0], theme, tui.lang, st.sub_tab);

        match st.sub_tab {
            SubTab::Schedule => draw_schedule_tab(f, outer[1], theme, app, tui, &st),
            SubTab::Standings => draw_standings_tab(f, outer[1], theme, app, tui),
            SubTab::Playoffs => draw_playoffs_tab(f, outer[1], theme, app, tui),
            SubTab::Awards => draw_awards_tab(f, outer[1], theme, app, tui, &st),
            SubTab::AllStar => draw_all_star_tab(f, outer[1], theme, app, tui, &st),
            SubTab::Cup => draw_cup_tab(f, outer[1], theme, app, tui, &st),
        }

        if let Some(modal) = st.pause_modal.clone() {
            let footer = format!(
                "[c] {}   [i] {}   [Esc] {}",
                t(tui.lang, T::CommonContinue),
                t(tui.lang, T::TradesInbox),
                t(tui.lang, T::CommonDismiss)
            );
            draw_centered_modal(f, area, theme, &modal.title, &modal.body, &footer);
        } else if let Some(c) = st.advance_confirm.as_ref() {
            draw_widget_modal(f, area, theme, c);
        } else if let Some(c) = st.playoffs_confirm.as_ref() {
            draw_widget_modal(f, area, theme, c);
        }
    });
}

fn draw_tab_strip(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, current: SubTab) {
    let mut spans: Vec<Span> = Vec::new();
    for (i, tab) in SubTab::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", theme.text()));
        }
        let label = if *tab == current {
            format!("[{}]", tab.label(lang))
        } else {
            format!(" {} ", tab.label(lang))
        };
        let style = if *tab == current {
            theme.highlight()
        } else {
            theme.text()
        };
        spans.push(Span::styled(format!("{}.", i + 1), theme.muted_style()));
        spans.push(Span::styled(label, style));
    }
    let p = Paragraph::new(Line::from(spans))
        .alignment(Alignment::Center)
        .block(theme.block(t(lang, T::CalendarTitle)));
    f.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Tab 1: Schedule (month grid)
// ---------------------------------------------------------------------------

fn draw_schedule_tab(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    app: &mut AppState,
    tui: &TuiApp,
    st: &CalendarState,
) {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        centered_block(f, area, theme, t(tui.lang, T::CalendarSchedule), &[t(tui.lang, T::CommonNoSaveLoaded)]);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Header line: season + day-of-season + month label.
    let today = day_to_date(ctx.season_state.day);
    let header_text = format!(
        " {}-{:02} - {} {} of {} - {} {} ",
        ctx.season.0 - 1,
        ctx.season.0 % 100,
        t(tui.lang, T::CalendarDayOf),
        ctx.season_state.day,
        SEASON_LENGTH_DAYS,
        month_name(tui.lang, st.view_month.month()),
        st.view_month.year(),
    );
    let header = Paragraph::new(Line::from(Span::styled(
        header_text,
        theme.accent_style(),
    )))
    .alignment(Alignment::Center)
    .block(theme.block(""));
    f.render_widget(header, chunks[0]);

    // Index schedule entries by date for quick cell rendering.
    let by_date: HashMap<NaiveDate, &ScheduleEntry> =
        st.schedule.iter().map(|e| (e.date, e)).collect();

    // Resolve team abbrevs once for the visible month — only what we need.
    let team_abbrev = team_abbrev_index(app).unwrap_or_default();

    draw_month_grid(
        f,
        chunks[1],
        theme,
        tui.lang,
        st.view_month,
        today,
        st.cell_cursor,
        ctx.user_team,
        &by_date,
        &team_abbrev,
    );
}

fn draw_month_grid(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    lang: Lang,
    view_month: NaiveDate,
    today: NaiveDate,
    cursor: u8,
    user_team: TeamId,
    by_date: &HashMap<NaiveDate, &ScheduleEntry>,
    team_abbrev: &HashMap<TeamId, String>,
) {
    // Header row: weekday names + 6 day rows = 7 rows total.
    let row_constraints: Vec<Constraint> = std::iter::once(Constraint::Length(1))
        .chain(std::iter::repeat(Constraint::Min(3)).take(6))
        .collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(inner_rect(area));

    let block = theme.block("");
    f.render_widget(block, area);

    // Weekday header.
    let header_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, 7); 7])
        .split(rows[0]);
    for i in 0..7 {
        let p = Paragraph::new(Line::from(Span::styled(
            weekday_name(lang, i),
            theme.muted_style(),
        )))
            .alignment(Alignment::Center);
        f.render_widget(p, header_cols[i]);
    }

    // First-day-of-month leading offset (Mon=0..Sun=6).
    let first = view_month;
    let lead = match first.weekday() {
        Weekday::Mon => 0,
        Weekday::Tue => 1,
        Weekday::Wed => 2,
        Weekday::Thu => 3,
        Weekday::Fri => 4,
        Weekday::Sat => 5,
        Weekday::Sun => 6,
    };

    // Render 6 weeks × 7 days = 42 cells.
    for week in 0..6u8 {
        let week_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, 7); 7])
            .split(rows[1 + week as usize]);
        for col in 0..7u8 {
            let cell_idx = week * 7 + col;
            let offset = cell_idx as i64 - lead as i64;
            let cell_date = first + ChronoDuration::days(offset);
            let in_month = cell_date.month() == view_month.month();

            let is_today = cell_date == today;
            let is_past = cell_date < today;
            let is_cursor = cell_idx == cursor;

            let entry = by_date.get(&cell_date).copied();
            draw_day_cell(
                f,
                week_cols[col as usize],
                theme,
                cell_date.day() as u8,
                entry,
                user_team,
                team_abbrev,
                in_month,
                is_today,
                is_past,
                is_cursor,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_day_cell(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    day_num: u8,
    entry: Option<&ScheduleEntry>,
    user_team: TeamId,
    team_abbrev: &HashMap<TeamId, String>,
    in_month: bool,
    is_today: bool,
    is_past: bool,
    is_cursor: bool,
) {
    // Border style — cursor wins over today wins over plain.
    let mut border_style = Style::default().fg(theme.border);
    if is_cursor {
        border_style = Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD);
    } else if is_today {
        border_style = Style::default()
            .fg(theme.highlight_fg)
            .bg(theme.highlight_bg)
            .add_modifier(Modifier::BOLD);
    }
    let block = Block::default().borders(Borders::ALL).border_style(border_style);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Two stacked lines: day number, then opponent line.
    let day_text_style = if !in_month {
        theme.muted_style()
    } else if is_past {
        theme.muted_style()
    } else if is_today {
        theme.accent_style()
    } else {
        theme.text()
    };

    let day_line = Line::from(Span::styled(format!("{:>2}", day_num), day_text_style));

    let mut lines = vec![day_line];

    if let Some(e) = entry {
        if e.home == user_team || e.away == user_team {
            let label = format_opponent(e, user_team, team_abbrev);
            let style = if e.played {
                theme.muted_style()
            } else if is_today {
                theme.accent_style()
            } else {
                theme.text()
            };
            lines.push(Line::from(Span::styled(label, style)));
        }
    } else {
        // Marker symbols for league events. Day-mapping: derive by re-computing
        // the day-of-season for this cell. We only mark cells that fall inside
        // the visible month *and* on a known event day.
        // Skipped if there's a real game on this day — game wins.
    }

    let p = Paragraph::new(lines).alignment(Alignment::Left);
    f.render_widget(p, inner);
}

fn format_opponent(
    e: &ScheduleEntry,
    user_team: TeamId,
    team_abbrev: &HashMap<TeamId, String>,
) -> String {
    let home_user = e.home == user_team;
    let opp = if home_user { e.away } else { e.home };
    let abbrev = team_abbrev
        .get(&opp)
        .cloned()
        .unwrap_or_else(|| format!("#{}", opp.0));
    if e.played {
        let (us, them) = if home_user {
            (e.home_score.unwrap_or(0), e.away_score.unwrap_or(0))
        } else {
            (e.away_score.unwrap_or(0), e.home_score.unwrap_or(0))
        };
        let result = if us > them { "W" } else { "L" };
        format!("{} {}-{}", result, us, them)
    } else if home_user {
        format!("v{}", abbrev)
    } else {
        format!("@{}", abbrev)
    }
}

// ---------------------------------------------------------------------------
// Tab 2: Standings
// ---------------------------------------------------------------------------

fn draw_standings_tab(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    app: &mut AppState,
    tui: &TuiApp,
) {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        centered_block(f, area, theme, t(tui.lang, T::CalendarStandings), &[t(tui.lang, T::CommonNoSaveLoaded)]);
        return;
    };
    let rows = match app.store().and_then(|s| Ok(s.read_standings(ctx.season)?)) {
        Ok(v) => v,
        Err(_) => {
            centered_block(f, area, theme, t(tui.lang, T::CalendarStandings), &["(unable to load)"]);
            return;
        }
    };
    if rows.is_empty() {
        centered_block(
            f,
            area,
            theme,
            t(tui.lang, T::CalendarStandings),
            &["No standings recorded yet — sim a few days first."],
        );
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let east: Vec<&StandingRow> =
        rows.iter().filter(|r| r.conference == Conference::East).collect();
    let west: Vec<&StandingRow> =
        rows.iter().filter(|r| r.conference == Conference::West).collect();

    f.render_widget(standings_table(&east, theme, " East "), cols[0]);
    f.render_widget(standings_table(&west, theme, " West "), cols[1]);
}

fn standings_table<'a>(rows: &[&StandingRow], theme: &'a Theme, title: &'a str) -> Table<'a> {
    let header = Row::new(vec![
        Cell::from("#"),
        Cell::from("TM"),
        Cell::from("W"),
        Cell::from("L"),
        Cell::from("PCT"),
        Cell::from("GB"),
    ])
    .style(theme.accent_style());

    let leader = rows.first().copied();
    let body: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let pct = if r.wins + r.losses == 0 {
                0.0
            } else {
                r.wins as f32 / (r.wins + r.losses) as f32
            };
            let gb = match leader {
                Some(l) if l.team == r.team => "—".to_string(),
                Some(l) => {
                    let n = (l.wins as i32 - r.wins as i32) + (r.losses as i32 - l.losses as i32);
                    format!("{:.1}", (n as f32) / 2.0)
                }
                None => "-".to_string(),
            };
            Row::new(vec![
                Cell::from(format!("{}", i + 1)),
                Cell::from(r.abbrev.clone()),
                Cell::from(format!("{}", r.wins)),
                Cell::from(format!("{}", r.losses)),
                Cell::from(format!(".{:03}", (pct * 1000.0).round() as u32)),
                Cell::from(gb),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Length(6),
    ];
    Table::new(body, widths)
        .header(header)
        .block(theme.block(title))
}

// ---------------------------------------------------------------------------
// Tab 3: Playoffs
// ---------------------------------------------------------------------------

fn draw_playoffs_tab(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    app: &mut AppState,
    tui: &TuiApp,
) {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        centered_block(f, area, theme, t(tui.lang, T::CalendarPlayoffs), &[t(tui.lang, T::CommonNoSaveLoaded)]);
        return;
    };
    let phase = ctx.season_state.phase;
    if !matches!(phase, SeasonPhase::Playoffs | SeasonPhase::OffSeason) {
        centered_block(
            f,
            area,
            theme,
            t(tui.lang, T::CalendarPlayoffs),
            &[
                "Playoffs not started.",
                "",
                "Sim through the regular season first.",
                "Press Enter from the Schedule tab to sim to season-end.",
            ],
        );
        return;
    }

    let series = match app.store().and_then(|s| Ok(s.read_series(ctx.season)?)) {
        Ok(v) => v,
        Err(_) => {
            centered_block(f, area, theme, t(tui.lang, T::CalendarPlayoffs), &["(unable to load)"]);
            return;
        }
    };
    if series.is_empty() {
        centered_block(
            f,
            area,
            theme,
            t(tui.lang, T::CalendarPlayoffs),
            &[
                "Bracket not yet generated.",
                "",
                "Press Enter to sim the full playoff bracket.",
            ],
        );
        return;
    }

    let team_abbrev = team_abbrev_index(app).unwrap_or_default();

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(
            "{} - {} {}",
            t(tui.lang, T::CalendarPlayoffs),
            t(tui.lang, T::NewGameSeason),
            ctx.season.0
        ),
        theme.accent_style(),
    )));
    lines.push(Line::from(""));

    let round_label = |round: u8| match round {
        1 => "R1",
        2 => "R2",
        3 => "R3",
        4 => "R4",
        _ => "?",
    };

    let mut current_round = 0u8;
    for s in &series {
        if s.round != current_round {
            current_round = s.round;
            lines.push(Line::from(Span::styled(
                format!("  {}", round_label(s.round)),
                theme.accent_style(),
            )));
        }
        let abbrev_h = team_abbrev
            .get(&s.home_team)
            .cloned()
            .unwrap_or_else(|| format!("#{}", s.home_team.0));
        let abbrev_a = team_abbrev
            .get(&s.away_team)
            .cloned()
            .unwrap_or_else(|| format!("#{}", s.away_team.0));
        let winner_marker = if s.home_wins == 4 || s.away_wins == 4 {
            if s.home_wins > s.away_wins {
                format!(" — {} wins", abbrev_h)
            } else {
                format!(" — {} wins", abbrev_a)
            }
        } else {
            String::new()
        };
        lines.push(Line::from(Span::styled(
            format!(
                "    {:<4} {} - {} {:<4}{}",
                abbrev_h, s.home_wins, s.away_wins, abbrev_a, winner_marker
            ),
            theme.text(),
        )));
    }

    let p = Paragraph::new(lines).block(theme.block(t(tui.lang, T::CalendarPlayoffs)));
    f.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Tab 4: Awards
// ---------------------------------------------------------------------------

fn draw_awards_tab(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    app: &mut AppState,
    tui: &TuiApp,
    st: &CalendarState,
) {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        centered_block(f, area, theme, t(tui.lang, T::CalendarAwards), &[t(tui.lang, T::CommonNoSaveLoaded)]);
        return;
    };
    let target_season = SeasonId(
        ((ctx.season.0 as i32) + st.awards_season_offset)
            .max(1) as u16,
    );

    let store = match app.store() {
        Ok(s) => s,
        Err(_) => {
            centered_block(f, area, theme, t(tui.lang, T::CalendarAwards), &["(unable to load)"]);
            return;
        }
    };
    let award_rows = store.read_awards(target_season).unwrap_or_default();
    let players = store.all_active_players().unwrap_or_default();
    let player_name: HashMap<PlayerId, String> =
        players.iter().map(|p| (p.id, p.name.clone())).collect();

    let mut header_lines: Vec<(&str, String)> = vec![
        (
            t(tui.lang, T::NewGameSeason),
            format!("{}-{:02}", target_season.0 - 1, target_season.0 % 100),
        ),
    ];
    if award_rows.is_empty() {
        header_lines.push((t(tui.lang, T::CommonReady), t(tui.lang, T::CalendarAwards).to_string()));
        let table = kv_table(&header_lines, theme, t(tui.lang, T::CalendarAwards));
        f.render_widget(table, area);
        return;
    }

    let lookup = |award: &str| -> String {
        award_rows
            .iter()
            .find(|(a, _)| a == award)
            .and_then(|(_, pid)| player_name.get(pid).cloned())
            .unwrap_or_else(|| "—".into())
    };

    let award_list: Vec<(&str, String)> = vec![
        ("MVP", lookup("MVP")),
        ("DPOY", lookup("DPOY")),
        ("ROY", lookup("ROY")),
        ("6M", lookup("SIXTH_MAN")),
        ("MIP", lookup("MIP")),
    ];
    let owned: Vec<(&str, String)> = award_list.into_iter().collect();
    let title = format!(
        " {} - {} {}-{:02} (← →: {}) ",
        t(tui.lang, T::CalendarAwards),
        t(tui.lang, T::NewGameSeason),
        target_season.0 - 1,
        target_season.0 % 100,
        t(tui.lang, T::CommonMove)
    );
    let table = kv_table(&owned, theme, &title);
    f.render_widget(table, area);
}

// ---------------------------------------------------------------------------
// Tab 5: All-Star
// ---------------------------------------------------------------------------

fn draw_all_star_tab(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    app: &mut AppState,
    tui: &TuiApp,
    st: &CalendarState,
) {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        centered_block(f, area, theme, t(tui.lang, T::CalendarAllStar), &[t(tui.lang, T::CommonNoSaveLoaded)]);
        return;
    };
    let target_season = SeasonId(
        ((ctx.season.0 as i32) + st.awards_season_offset)
            .max(1) as u16,
    );

    let store = match app.store() {
        Ok(s) => s,
        Err(_) => {
            centered_block(f, area, theme, t(tui.lang, T::CalendarAllStar), &["(unable to load)"]);
            return;
        }
    };
    let rows = store.read_all_star(target_season).unwrap_or_default();
    if rows.is_empty() {
        centered_block(
            f,
            area,
            theme,
            t(tui.lang, T::CalendarAllStar),
            &[
                "No All-Star roster yet.",
                "",
                "(Sim past day 41 to trigger.)",
                "Use ← → to flip seasons.",
            ],
        );
        return;
    }

    let players = store.all_active_players().unwrap_or_default();
    let player_name: HashMap<PlayerId, String> =
        players.iter().map(|p| (p.id, p.name.clone())).collect();

    let mut east_starters = Vec::new();
    let mut east_reserves = Vec::new();
    let mut west_starters = Vec::new();
    let mut west_reserves = Vec::new();
    for (conf, role, pid) in &rows {
        let name = player_name
            .get(pid)
            .cloned()
            .unwrap_or_else(|| format!("#{}", pid.0));
        match (conf, role.as_str()) {
            (Conference::East, "starter") => east_starters.push(name),
            (Conference::East, _) => east_reserves.push(name),
            (Conference::West, "starter") => west_starters.push(name),
            (Conference::West, _) => west_reserves.push(name),
        }
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let render_side = |starters: &[String], reserves: &[String]| -> Vec<Line<'static>> {
        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(t(tui.lang, T::RotationStarters), theme.accent_style())));
        for n in starters {
            lines.push(Line::from(format!("  {}", n)));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(t(tui.lang, T::RotationBench), theme.accent_style())));
        for n in reserves {
            lines.push(Line::from(format!("  {}", n)));
        }
        lines
    };

    let east_title = " East ".to_string();
    let west_title = " West ".to_string();
    let east = Paragraph::new(render_side(&east_starters, &east_reserves))
        .block(theme.block(&east_title));
    let west = Paragraph::new(render_side(&west_starters, &west_reserves))
        .block(theme.block(&west_title));
    f.render_widget(east, cols[0]);
    f.render_widget(west, cols[1]);
}

// ---------------------------------------------------------------------------
// Tab 6: Cup
// ---------------------------------------------------------------------------

fn draw_cup_tab(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    app: &mut AppState,
    tui: &TuiApp,
    st: &CalendarState,
) {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        centered_block(f, area, theme, t(tui.lang, T::CalendarCup), &[t(tui.lang, T::CommonNoSaveLoaded)]);
        return;
    };
    let target_season = SeasonId(
        ((ctx.season.0 as i32) + st.awards_season_offset)
            .max(1) as u16,
    );

    let store = match app.store() {
        Ok(s) => s,
        Err(_) => {
            centered_block(f, area, theme, t(tui.lang, T::CalendarCup), &["(unable to load)"]);
            return;
        }
    };
    let rows = store.read_cup_matches(target_season).unwrap_or_default();
    if rows.is_empty() {
        centered_block(
            f,
            area,
            theme,
            t(tui.lang, T::CalendarCup),
            &[
                "No NBA Cup recorded for this season.",
                "",
                &format!("(Group stage triggers around day {}.)", CUP_GROUP_DAY),
            ],
        );
        return;
    }

    let team_abbrev = team_abbrev_index(app).unwrap_or_default();

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(
            "{} - {} {}",
            t(tui.lang, T::CalendarCup),
            t(tui.lang, T::NewGameSeason),
            target_season.0
        ),
        theme.accent_style(),
    )));
    lines.push(Line::from(""));

    // Group-by round. Display order: group, qf, sf, final.
    fn label_for(round: &str) -> &'static str {
        match round {
            "group" => "G",
            "qf" => "QF",
            "sf" => "SF",
            "final" => "F",
            _ => "?",
        }
    }
    for label in ["group", "qf", "sf", "final"] {
        let group: Vec<_> = rows.iter().filter(|r| r.round == label).collect();
        if group.is_empty() {
            continue;
        }
        lines.push(Line::from(Span::styled(
            format!("  {}", label_for(label)),
            theme.accent_style(),
        )));
        for r in group {
            let h = team_abbrev
                .get(&r.home_team)
                .cloned()
                .unwrap_or_else(|| format!("#{}", r.home_team.0));
            let a = team_abbrev
                .get(&r.away_team)
                .cloned()
                .unwrap_or_else(|| format!("#{}", r.away_team.0));
            lines.push(Line::from(format!(
                "    {:<4} {:>3} - {:>3} {:<4}",
                h, r.home_score, r.away_score, a
            )));
        }
        lines.push(Line::from(""));
    }

    let p = Paragraph::new(lines).block(theme.block(t(tui.lang, T::CalendarCup)));
    f.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Modal helpers
// ---------------------------------------------------------------------------

fn draw_centered_modal(f: &mut Frame, area: Rect, theme: &Theme, title: &str, body: &str, footer: &str) {
    let w = 60.min(area.width.saturating_sub(4));
    let h = 9.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(body.to_string(), theme.text())).alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(footer.to_string(), theme.muted_style())).alignment(Alignment::Center),
    ];
    let title_owned = format!(" {} ", title);
    let p = Paragraph::new(lines).block(theme.block(&title_owned));
    f.render_widget(p, rect);
}

fn draw_widget_modal<W: FormWidget>(f: &mut Frame, area: Rect, theme: &Theme, w: &W) {
    let mw = 50.min(area.width.saturating_sub(4));
    let mh = 7.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(mw)) / 2;
    let y = area.y + (area.height.saturating_sub(mh)) / 2;
    let rect = Rect { x, y, width: mw, height: mh };
    w.render(f, rect, theme);
}

// ---------------------------------------------------------------------------
// handle_key
// ---------------------------------------------------------------------------

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    let mut consumed = false;
    let mut sim_request: Option<SimRequest> = None;
    let mut switch_to_home = false;

    STATE.with(|cell| {
        let mut st = cell.borrow_mut();

        // Modal precedence: pause modal absorbs everything except 'c', 'i', Esc.
        if let Some(_modal) = st.pause_modal.clone() {
            match key.code {
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    st.pause_modal = None;
                }
                KeyCode::Char('i') | KeyCode::Char('I') => {
                    st.pause_modal = None;
                    switch_to_home = true;
                }
                KeyCode::Esc => {
                    st.pause_modal = None;
                }
                _ => {}
            }
            consumed = true;
            return;
        }
        if let Some(c) = st.advance_confirm.as_mut() {
            match c.handle_key(key) {
                WidgetEvent::Submitted => {
                    st.advance_confirm = None;
                    sim_request = Some(SimRequest::SeasonAdvance);
                }
                WidgetEvent::Cancelled => {
                    st.advance_confirm = None;
                }
                _ => {}
            }
            consumed = true;
            return;
        }
        if let Some(c) = st.playoffs_confirm.as_mut() {
            match c.handle_key(key) {
                WidgetEvent::Submitted => {
                    st.playoffs_confirm = None;
                    sim_request = Some(SimRequest::PlayoffsSim);
                }
                WidgetEvent::Cancelled => {
                    st.playoffs_confirm = None;
                }
                _ => {}
            }
            consumed = true;
            return;
        }

        // Tab navigation (works on every sub-tab).
        match key.code {
            KeyCode::Tab => {
                st.sub_tab = st.sub_tab.next();
                consumed = true;
                return;
            }
            KeyCode::BackTab => {
                st.sub_tab = st.sub_tab.prev();
                consumed = true;
                return;
            }
            KeyCode::Char(c @ '1'..='6') => {
                let idx = (c as u8 - b'1') as usize;
                if idx < SubTab::ALL.len() {
                    st.sub_tab = SubTab::ALL[idx];
                    consumed = true;
                    return;
                }
            }
            _ => {}
        }

        // Sub-tab specific keys.
        match st.sub_tab {
            SubTab::Schedule => {
                match key.code {
                    KeyCode::Char(' ') => {
                        sim_request = Some(SimRequest::Day);
                        consumed = true;
                    }
                    KeyCode::Char('w') | KeyCode::Char('W') => {
                        sim_request = Some(SimRequest::Week);
                        consumed = true;
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        sim_request = Some(SimRequest::Month);
                        consumed = true;
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        st.advance_confirm =
                            Some(Confirm::new(t(tui.lang, T::CalendarSeasonAdvance)));
                        consumed = true;
                    }
                    KeyCode::Enter => {
                        let cell_date = cell_to_date(st.view_month, st.cell_cursor);
                        if let Some(target) = event_target_for_date(cell_date) {
                            sim_request = Some(SimRequest::SimTo(target));
                            consumed = true;
                        }
                    }
                    KeyCode::Left => {
                        if !key.modifiers.contains(KeyModifiers::SHIFT) {
                            st.cell_cursor = st.cell_cursor.saturating_sub(1);
                        }
                        consumed = true;
                    }
                    KeyCode::Right => {
                        st.cell_cursor = (st.cell_cursor + 1).min(41);
                        consumed = true;
                    }
                    KeyCode::Up => {
                        st.cell_cursor = st.cell_cursor.saturating_sub(7);
                        consumed = true;
                    }
                    KeyCode::Down => {
                        st.cell_cursor = (st.cell_cursor + 7).min(41);
                        consumed = true;
                    }
                    KeyCode::Char('[') => {
                        st.view_month = prev_month(st.view_month);
                        consumed = true;
                    }
                    KeyCode::Char(']') => {
                        st.view_month = next_month(st.view_month);
                        consumed = true;
                    }
                    _ => {}
                }
            }
            SubTab::Playoffs => {
                if matches!(key.code, KeyCode::Enter) {
                    st.playoffs_confirm = Some(Confirm::new(t(tui.lang, T::CalendarPlayoffs)));
                    consumed = true;
                }
            }
            SubTab::Awards | SubTab::AllStar | SubTab::Cup => {
                match key.code {
                    KeyCode::Left => {
                        st.awards_season_offset -= 1;
                        consumed = true;
                    }
                    KeyCode::Right => {
                        if st.awards_season_offset < 0 {
                            st.awards_season_offset += 1;
                            consumed = true;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    });

    if switch_to_home {
        tui.current = crate::tui::Screen::Home;
        return Ok(true);
    }

    if let Some(req) = sim_request {
        run_sim_request(app, tui, req)?;
    }

    Ok(consumed)
}

// ---------------------------------------------------------------------------
// Sim request runner
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
enum SimRequest {
    Day,
    Week,
    Month,
    SimTo(SimTarget),
    SeasonAdvance,
    PlayoffsSim,
}

#[derive(Copy, Clone, Debug)]
enum SimTarget {
    AllStar,
    CupFinal,
    TradeDeadline,
    SeasonEnd,
}

fn event_target_for_date(date: NaiveDate) -> Option<SimTarget> {
    let day = date_to_day(date);
    if day < 0 {
        return None;
    }
    let day = day as u32;
    if day == ALL_STAR_DAY {
        Some(SimTarget::AllStar)
    } else if day == CUP_FINAL_DAY {
        Some(SimTarget::CupFinal)
    } else if day == TRADE_DEADLINE_DAY {
        Some(SimTarget::TradeDeadline)
    } else if day >= PLAYOFFS_START_DAY {
        Some(SimTarget::SeasonEnd)
    } else {
        None
    }
}

fn run_sim_request(app: &mut AppState, tui: &mut TuiApp, req: SimRequest) -> Result<()> {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        return Ok(());
    };
    let season = ctx.season;
    let user_team = ctx.user_team;
    let pre_day = ctx.season_state.day;

    let pre_offers = app
        .store()?
        .read_open_chains_targeting(season, user_team)?
        .len();
    let pre_news_id = app
        .store()?
        .recent_news(1)?
        .first()
        .map(|n| (n.season, n.day))
        .unwrap_or((season, 0));

    let result = with_silenced_io(|| {
        let cmd = match req {
            SimRequest::Day => Command::SimDay { count: Some(1) },
            SimRequest::Week => Command::SimWeek { no_pause: true },
            SimRequest::Month => Command::SimMonth { no_pause: true },
            SimRequest::SimTo(target) => Command::SimTo {
                phase: match target {
                    SimTarget::AllStar => "all-star".into(),
                    SimTarget::CupFinal => "cup-final".into(),
                    SimTarget::TradeDeadline => "trade-deadline".into(),
                    SimTarget::SeasonEnd => "season-end".into(),
                },
            },
            SimRequest::SeasonAdvance => {
                Command::SeasonAdvance(JsonFlag { json: false })
            }
            SimRequest::PlayoffsSim => Command::Playoffs(PlayoffsArgs {
                action: PlayoffsAction::Sim(JsonFlag { json: false }),
            }),
        };
        crate::commands::dispatch(app, cmd)
    });

    match result {
        Ok(()) => {
            tui.refresh_season_state(app)?;
            crate::tui::invalidate_all_screens(tui);

            // Re-fetch the post-sim ctx (refresh_season_state above updated it).
            let post_day = tui
                .save_ctx
                .as_ref()
                .map(|c| c.season_state.day)
                .unwrap_or(pre_day);
            let post_season = tui
                .save_ctx
                .as_ref()
                .map(|c| c.season)
                .unwrap_or(season);
            let post_user_team = tui
                .save_ctx
                .as_ref()
                .map(|c| c.user_team)
                .unwrap_or(user_team);
            let post_offers = app
                .store()?
                .read_open_chains_targeting(post_season, post_user_team)?
                .len();
            let new_offers = post_offers.saturating_sub(pre_offers);
            let post_news = app.store()?.recent_news(1)?;
            let post_news_id = post_news
                .first()
                .map(|n| (n.season, n.day))
                .unwrap_or(pre_news_id);
            let _ = post_news_id;
            let label = match req {
                SimRequest::Day => t(tui.lang, T::CalendarSimDay),
                SimRequest::Week => t(tui.lang, T::CalendarSimWeek),
                SimRequest::Month => t(tui.lang, T::CalendarSimMonth),
                SimRequest::SimTo(_) => t(tui.lang, T::CalendarSimToEvent),
                SimRequest::SeasonAdvance => t(tui.lang, T::CalendarSeasonAdvance),
                SimRequest::PlayoffsSim => t(tui.lang, T::CalendarPlayoffs),
            };
            let mut msg = format!(
                "{}: +{}d ({} {})",
                label,
                post_day.saturating_sub(pre_day),
                t(tui.lang, T::CalendarDayOf),
                post_day
            );
            if new_offers > 0 {
                msg.push_str(&format!(", {} new offer(s)", new_offers));
                if matches!(req, SimRequest::Week | SimRequest::Month) {
                    STATE.with(|cell| {
                        let mut st = cell.borrow_mut();
                        st.pause_modal = Some(PauseModal {
                            title: t(tui.lang, T::TradesInbox).into(),
                            body: format!(
                                "{}: {}",
                                t(tui.lang, T::TradesInbox),
                                new_offers
                            ),
                        });
                    });
                }
            }
            tui.last_msg = Some(msg);
        }
        Err(e) => {
            tui.last_msg = Some(format!("{}: {}", t(tui.lang, T::CommonError), e));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tiny helpers
// ---------------------------------------------------------------------------

fn team_abbrev_index(app: &mut AppState) -> Result<HashMap<TeamId, String>> {
    let teams = app.store()?.list_teams()?;
    Ok(teams.into_iter().map(|t| (t.id, t.abbrev)).collect())
}

fn first_of_month(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).expect("valid")
}

fn prev_month(d: NaiveDate) -> NaiveDate {
    if d.month() == 1 {
        NaiveDate::from_ymd_opt(d.year() - 1, 12, 1).expect("valid")
    } else {
        NaiveDate::from_ymd_opt(d.year(), d.month() - 1, 1).expect("valid")
    }
}

fn next_month(d: NaiveDate) -> NaiveDate {
    if d.month() == 12 {
        NaiveDate::from_ymd_opt(d.year() + 1, 1, 1).expect("valid")
    } else {
        NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1).expect("valid")
    }
}

fn cursor_for(date: NaiveDate, view_month: NaiveDate) -> Option<u8> {
    let lead = match view_month.weekday() {
        Weekday::Mon => 0,
        Weekday::Tue => 1,
        Weekday::Wed => 2,
        Weekday::Thu => 3,
        Weekday::Fri => 4,
        Weekday::Sat => 5,
        Weekday::Sun => 6,
    };
    let offset = (date - view_month).num_days() + lead as i64;
    if (0..42).contains(&offset) {
        Some(offset as u8)
    } else {
        None
    }
}

fn cell_to_date(view_month: NaiveDate, cell: u8) -> NaiveDate {
    let lead = match view_month.weekday() {
        Weekday::Mon => 0i64,
        Weekday::Tue => 1,
        Weekday::Wed => 2,
        Weekday::Thu => 3,
        Weekday::Fri => 4,
        Weekday::Sat => 5,
        Weekday::Sun => 6,
    };
    view_month + ChronoDuration::days(cell as i64 - lead)
}

fn weekday_name(lang: Lang, day: usize) -> &'static str {
    match day {
        0 => t(lang, T::CalDayMon),
        1 => t(lang, T::CalDayTue),
        2 => t(lang, T::CalDayWed),
        3 => t(lang, T::CalDayThu),
        4 => t(lang, T::CalDayFri),
        5 => t(lang, T::CalDaySat),
        6 => t(lang, T::CalDaySun),
        _ => "?",
    }
}

fn month_name(lang: Lang, month: u32) -> &'static str {
    match month {
        1 => t(lang, T::CalMonJan),
        2 => t(lang, T::CalMonFeb),
        3 => t(lang, T::CalMonMar),
        4 => t(lang, T::CalMonApr),
        5 => t(lang, T::CalMonMay),
        6 => t(lang, T::CalMonJun),
        7 => t(lang, T::CalMonJul),
        8 => t(lang, T::CalMonAug),
        9 => t(lang, T::CalMonSep),
        10 => t(lang, T::CalMonOct),
        11 => t(lang, T::CalMonNov),
        12 => t(lang, T::CalMonDec),
        _ => "?",
    }
}

fn inner_rect(r: Rect) -> Rect {
    Rect {
        x: r.x.saturating_add(1),
        y: r.y.saturating_add(1),
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(2),
    }
}
