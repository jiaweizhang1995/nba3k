//! Finance screen (M22). Single-screen cap sheet with sortable contract rows
//! and a modal extension flow.
//!
//! The screen mirrors the M21 cache pattern: row data is fetched lazily into a
//! thread-local cache, mutations route through `commands::dispatch` wrapped in
//! `with_silenced_io`, and `invalidate()` is exposed for other screens to bust
//! stale cap data after roster changes.

use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Clear, Gauge, Paragraph, Row, Table, Wrap},
    Frame,
};
use std::cell::RefCell;

use crate::cli::Command;
use crate::state::AppState;
use crate::tui::widgets::{ActionBar, FormWidget, NumberInput, Theme, WidgetEvent};
use crate::tui::{with_silenced_io, TuiApp};
use nba3k_core::{t, Cents, Lang, LeagueYear, Player, PlayerId, Position, SeasonId, T};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SortKey {
    Total,
    Years,
    Name,
}

impl Default for SortKey {
    fn default() -> Self {
        SortKey::Total
    }
}

#[derive(Clone, Debug)]
struct FinanceSnapshot {
    team: String,
    season: SeasonId,
    roster_size: usize,
    payroll: Cents,
    league_year: LeagueYear,
    rows: Vec<ContractRow>,
}

#[derive(Clone, Debug)]
struct ContractRow {
    player_id: PlayerId,
    name: String,
    position: Position,
    age: u8,
    years_remaining: usize,
    y1: Cents,
    y2: Cents,
    y3: Cents,
    y4: Cents,
    total: Cents,
    notes: String,
}

#[derive(Default)]
struct FinanceCache {
    snapshot: Option<FinanceSnapshot>,
    sort: SortKey,
    cursor: usize,
    modal: Modal,
}

#[derive(Default)]
enum Modal {
    #[default]
    None,
    ExtendSalary {
        input: NumberInput,
        target_id: PlayerId,
        target_name: String,
    },
    ExtendYears {
        input: NumberInput,
        target_id: PlayerId,
        target_name: String,
        salary_m: i64,
    },
}

thread_local! {
    static CACHE: RefCell<FinanceCache> = RefCell::new(FinanceCache::default());
}

pub fn invalidate() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.snapshot = None;
        c.cursor = 0;
    });
}

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    if !tui.has_save() {
        let p = Paragraph::new(t(tui.lang, T::CommonNoSaveLoaded))
            .block(theme.block(t(tui.lang, T::FinanceTitle)));
        f.render_widget(p, area);
        return;
    }

    if let Err(e) = ensure_cache(app, tui) {
        let p =
            Paragraph::new(format!("Finance unavailable: {}", e)).block(theme.block(t(tui.lang, T::FinanceTitle)));
        f.render_widget(p, area);
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);

    draw_summary(f, parts[0], theme, tui.lang);
    draw_contracts(f, parts[1], theme, tui.lang);
    draw_footer(f, parts[2], theme, tui);

    let need_modal = CACHE.with(|c| !matches!(c.borrow().modal, Modal::None));
    if need_modal {
        let rect = modal_rect(area);
        f.render_widget(Clear, rect);
        draw_modal(f, rect, theme, tui.lang);
    }
}

fn draw_summary(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let Some(s) = cache.snapshot.as_ref() else {
            let p = Paragraph::new("No finance data loaded.").block(theme.block(t(lang, T::FinanceTitle)));
            f.render_widget(p, area);
            return;
        };

        let over_under = |line: Cents| -> String {
            if s.payroll > line {
                format!("{} over", s.payroll - line)
            } else if s.payroll < line {
                format!("{} under", line - s.payroll)
            } else {
                "at line".to_string()
            }
        };

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(area);

        let lines = vec![
            Line::from(vec![
                Span::styled(format!("{}  ", s.team), theme.accent_style()),
                Span::styled(
                    format!(
                        "Season {} / roster {} / payroll {}",
                        s.season.0, s.roster_size, s.payroll
                    ),
                    theme.text(),
                ),
            ]),
            Line::from(vec![
                Span::styled(format!("{:<15}", t(lang, T::FinanceCap)), theme.accent_style()),
                Span::styled(
                    format!(
                        "{:<10} {}",
                        s.league_year.cap,
                        over_under(s.league_year.cap)
                    ),
                    theme.text(),
                ),
            ]),
            Line::from(vec![
                Span::styled(format!("{:<15}", t(lang, T::FinanceTax)), theme.accent_style()),
                Span::styled(
                    format!(
                        "{:<10} {}",
                        s.league_year.tax,
                        over_under(s.league_year.tax)
                    ),
                    theme.text(),
                ),
            ]),
            Line::from(vec![
                Span::styled(format!("{:<15}", t(lang, T::FinanceApron)), theme.accent_style()),
                Span::styled(
                    format!(
                        "{:<10} {}",
                        s.league_year.apron_1,
                        over_under(s.league_year.apron_1)
                    ),
                    theme.text(),
                ),
            ]),
            Line::from(vec![
                Span::styled(format!("{:<15}", t(lang, T::FinanceApron)), theme.accent_style()),
                Span::styled(
                    format!(
                        "{:<10} {}",
                        s.league_year.apron_2,
                        over_under(s.league_year.apron_2)
                    ),
                    theme.text(),
                ),
            ]),
            Line::from(vec![
                Span::styled(t(lang, T::FinancePayroll), theme.accent_style()),
                Span::styled(
                    format!(
                        "{:<10} {}",
                        s.league_year.min_team_salary,
                        over_under(s.league_year.min_team_salary)
                    ),
                    theme.text(),
                ),
            ]),
        ];
        let p = Paragraph::new(lines)
            .block(theme.block(t(lang, T::FinanceTitle)))
            .wrap(Wrap { trim: false });
        f.render_widget(p, chunks[0]);

        let ratio = if s.league_year.apron_2.0 <= 0 {
            0.0
        } else {
            (s.payroll.0 as f64 / s.league_year.apron_2.0 as f64).clamp(0.0, 1.0)
        };
        let label = format!(
            "{} / {} {} ({:.0}%)",
            s.payroll,
            s.league_year.apron_2,
            t(lang, T::FinanceApron),
            ratio * 100.0
        );
        let gauge = Gauge::default()
            .block(theme.block(t(lang, T::FinancePayroll)))
            .gauge_style(theme.accent_style())
            .label(label)
            .ratio(ratio);
        f.render_widget(gauge, chunks[1]);
    });
}

fn draw_contracts(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let Some(snapshot) = cache.snapshot.as_ref() else {
            let p = Paragraph::new(t(lang, T::FinanceContracts))
                .block(theme.block(t(lang, T::FinanceContracts)));
            f.render_widget(p, area);
            return;
        };

        if snapshot.rows.is_empty() {
            let p = Paragraph::new(t(lang, T::RosterNoPlayers))
                .block(theme.block(t(lang, T::FinanceContracts)));
            f.render_widget(p, area);
            return;
        }

        let cursor = cache.cursor.min(snapshot.rows.len().saturating_sub(1));
        let header = Row::new(vec![
            Cell::from(Span::styled(t(lang, T::RosterPlayer), theme.accent_style())),
            Cell::from(Span::styled(t(lang, T::RosterPosition), theme.accent_style())),
            Cell::from(Span::styled(t(lang, T::RosterAge), theme.accent_style())),
            Cell::from(Span::styled("Y1", theme.accent_style())),
            Cell::from(Span::styled("Y2", theme.accent_style())),
            Cell::from(Span::styled("Y3", theme.accent_style())),
            Cell::from(Span::styled("Y4", theme.accent_style())),
            Cell::from(Span::styled(t(lang, T::FinanceTotal), theme.accent_style())),
            Cell::from(Span::styled("NOTES", theme.accent_style())),
        ]);

        let body: Vec<Row> = snapshot
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let style = if i == cursor {
                    theme.highlight()
                } else {
                    theme.text()
                };
                Row::new(vec![
                    Cell::from(Span::styled(r.name.clone(), style)),
                    Cell::from(Span::styled(format!("{}", r.position), style)),
                    Cell::from(Span::styled(format!("{}", r.age), style)),
                    Cell::from(Span::styled(money_cell(r.y1), style)),
                    Cell::from(Span::styled(money_cell(r.y2), style)),
                    Cell::from(Span::styled(money_cell(r.y3), style)),
                    Cell::from(Span::styled(money_cell(r.y4), style)),
                    Cell::from(Span::styled(format!("{}", r.total), style)),
                    Cell::from(Span::styled(r.notes.clone(), style)),
                ])
            })
            .collect();

        let title = format!(
            " {} - {}: {} ",
            t(lang, T::FinanceContracts),
            t(lang, T::CommonSort),
            sort_label(lang, cache.sort)
        );
        let table = Table::new(
            body,
            [
                Constraint::Min(18),
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(9),
                Constraint::Min(18),
            ],
        )
        .header(header)
        .block(theme.block(&title));
        f.render_widget(table, area);
    });
}

fn draw_footer(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp) {
    let status = tui.last_msg.as_deref();
    let hints = [
        ("Up/Dn", t(tui.lang, T::CommonMove)),
        ("PgUp/PgDn", t(tui.lang, T::CommonMove)),
        ("e", t(tui.lang, T::FinanceExtensions)),
        ("t/y/n", t(tui.lang, T::CommonSort)),
        ("Esc", t(tui.lang, T::CommonBack)),
    ];
    match status {
        Some(s) => ActionBar::new(&hints).with_status(s).render(f, area, theme),
        None => ActionBar::new(&hints).render(f, area, theme),
    }
}

fn modal_rect(area: Rect) -> Rect {
    let w = area.width.saturating_sub(8).min(76).max(36);
    let h = area.height.saturating_sub(6).min(12).max(8);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

fn render_number_value(f: &mut Frame, area: Rect, theme: &Theme, label: &str, value: &str) {
    let line = Line::from(vec![
        Span::styled(format!(" {} ", label), theme.accent_style()),
        Span::styled(value.to_string(), theme.text()),
        Span::styled("█", theme.text()),
    ]);
    f.render_widget(Paragraph::new(line).block(theme.block("")), area);
}

fn draw_modal(f: &mut Frame, rect: Rect, theme: &Theme, lang: Lang) {
    enum DrawSpec {
        None,
        Salary {
            input: NumberInput,
            target_name: String,
        },
        Years {
            input: NumberInput,
            target_name: String,
            salary_m: i64,
        },
    }

    let spec = CACHE.with(|c| {
        let cache = c.borrow();
        match &cache.modal {
            Modal::None => DrawSpec::None,
            Modal::ExtendSalary {
                input, target_name, ..
            } => DrawSpec::Salary {
                input: input.clone(),
                target_name: target_name.clone(),
            },
            Modal::ExtendYears {
                input,
                target_name,
                salary_m,
                ..
            } => DrawSpec::Years {
                input: input.clone(),
                target_name: target_name.clone(),
                salary_m: *salary_m,
            },
        }
    });

    match spec {
        DrawSpec::None => {}
        DrawSpec::Salary { input, target_name } => {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .split(rect);
            let head = Paragraph::new(Line::from(Span::styled(
                format!(" {}: {} (1/2)", t(lang, T::ModalExtendContractTitle), target_name),
                theme.accent_style(),
            )))
            .block(theme.block(""));
            f.render_widget(head, parts[0]);
            render_number_value(f, parts[1], theme, t(lang, T::RosterSalary), input.raw());
            let hint = Paragraph::new(Line::from(Span::styled(
                format!(
                    "{} 1-300 · Enter {} · Esc {}",
                    t(lang, T::RosterSalary),
                    t(lang, T::CommonConfirm),
                    t(lang, T::CommonCancel)
                ),
                theme.muted_style(),
            )))
            .block(theme.block(""));
            f.render_widget(hint, parts[2]);
        }
        DrawSpec::Years {
            input,
            target_name,
            salary_m,
        } => {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .split(rect);
            let head = Paragraph::new(Line::from(Span::styled(
                format!(" {}: {} (2/2) - ${}M/yr", t(lang, T::ModalExtendContractTitle), target_name, salary_m),
                theme.accent_style(),
            )))
            .block(theme.block(""));
            f.render_widget(head, parts[0]);
            render_number_value(f, parts[1], theme, t(lang, T::FinanceYears), input.raw());
            let hint = Paragraph::new(Line::from(Span::styled(
                format!(
                    "{} 1-5 · Enter {} · Esc {}",
                    t(lang, T::FinanceYears),
                    t(lang, T::CommonSubmit),
                    t(lang, T::CommonCancel)
                ),
                theme.muted_style(),
            )))
            .block(theme.block(""));
            f.render_widget(hint, parts[2]);
        }
    }
}

fn ensure_cache(app: &mut AppState, tui: &TuiApp) -> Result<()> {
    let need = CACHE.with(|c| c.borrow().snapshot.is_none());
    if !need {
        return Ok(());
    }

    let snapshot = build_snapshot(app, tui)?;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.snapshot = Some(snapshot);
        apply_sort(&mut c);
    });
    Ok(())
}

fn build_snapshot(app: &mut AppState, tui: &TuiApp) -> Result<FinanceSnapshot> {
    let store = app.store()?;
    let season = tui.season;
    let league_year = LeagueYear::for_season(season)
        .ok_or_else(|| anyhow!("no LeagueYear constants for season {}", season.0))?;
    let roster = store.roster_for_team(tui.user_team)?;
    let payroll = store.team_salary(tui.user_team, season)?;
    let team = store
        .team_abbrev(tui.user_team)?
        .unwrap_or_else(|| tui.user_abbrev.clone());

    let rows: Vec<ContractRow> = roster
        .into_iter()
        .map(|p| contract_row(p, season))
        .collect();

    Ok(FinanceSnapshot {
        team,
        season,
        roster_size: roster_len_from_rows(&rows),
        payroll,
        league_year,
        rows,
    })
}

fn roster_len_from_rows(rows: &[ContractRow]) -> usize {
    rows.len()
}

fn contract_row(player: Player, season: SeasonId) -> ContractRow {
    let mut year_salaries = [Cents::ZERO; 4];
    let mut years_remaining = 0usize;
    let mut total = Cents::ZERO;
    let mut option_notes: Vec<String> = Vec::new();

    if let Some(contract) = player.contract.as_ref() {
        for year in contract.years.iter().filter(|y| y.season.0 >= season.0) {
            years_remaining += 1;
            total += year.salary;
            if let Some(offset) = year.season.0.checked_sub(season.0) {
                if (offset as usize) < year_salaries.len() {
                    year_salaries[offset as usize] = year.salary;
                }
            }
            if year.team_option {
                option_notes.push(format!("TO{}", year_label(season, year.season)));
            }
            if year.player_option {
                option_notes.push(format!("PO{}", year_label(season, year.season)));
            }
        }
    }

    let mut notes: Vec<String> = Vec::new();
    if player.no_trade_clause {
        notes.push("NTC".to_string());
    }
    if let Some(kicker) = player.trade_kicker_pct {
        if kicker > 0 {
            notes.push(format!("{}% kicker", kicker));
        }
    }
    notes.extend(option_notes);

    ContractRow {
        player_id: player.id,
        name: clean_name(&player.name),
        position: player.primary_position,
        age: player.age,
        years_remaining,
        y1: year_salaries[0],
        y2: year_salaries[1],
        y3: year_salaries[2],
        y4: year_salaries[3],
        total,
        notes: if notes.is_empty() {
            "-".to_string()
        } else {
            notes.join(", ")
        },
    }
}

fn apply_sort(cache: &mut FinanceCache) {
    let Some(snapshot) = cache.snapshot.as_mut() else {
        return;
    };
    match cache.sort {
        SortKey::Total => snapshot.rows.sort_by(|a, b| {
            b.total
                .cmp(&a.total)
                .then_with(|| b.y1.cmp(&a.y1))
                .then_with(|| a.name.cmp(&b.name))
        }),
        SortKey::Years => snapshot.rows.sort_by(|a, b| {
            b.years_remaining
                .cmp(&a.years_remaining)
                .then_with(|| b.total.cmp(&a.total))
                .then_with(|| a.name.cmp(&b.name))
        }),
        SortKey::Name => snapshot.rows.sort_by(|a, b| a.name.cmp(&b.name)),
    }
    cache.cursor = cache.cursor.min(snapshot.rows.len().saturating_sub(1));
}

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    if handle_modal_key(app, tui, key)? {
        return Ok(true);
    }

    if !tui.has_save() {
        return Ok(false);
    }

    ensure_cache(app, tui)?;

    match key.code {
        KeyCode::Up => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.cursor = c.cursor.saturating_sub(1);
            });
            Ok(true)
        }
        KeyCode::Down => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let max = c
                    .snapshot
                    .as_ref()
                    .map(|s| s.rows.len().saturating_sub(1))
                    .unwrap_or(0);
                c.cursor = (c.cursor + 1).min(max);
            });
            Ok(true)
        }
        KeyCode::PageUp => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.cursor = c.cursor.saturating_sub(10);
            });
            Ok(true)
        }
        KeyCode::PageDown => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let max = c
                    .snapshot
                    .as_ref()
                    .map(|s| s.rows.len().saturating_sub(1))
                    .unwrap_or(0);
                c.cursor = (c.cursor + 10).min(max);
            });
            Ok(true)
        }
        KeyCode::Char(c) if sort_key_from_char(c).is_some() => {
            set_sort(sort_key_from_char(c).expect("guarded by is_some"));
            Ok(true)
        }
        KeyCode::Char('e') => {
            if let Some((pid, name)) = current_row() {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::ExtendSalary {
                        input: NumberInput::new("Salary $M")
                            .with_bounds(1, 300)
                            .with_initial(20),
                        target_id: pid,
                        target_name: name,
                    };
                });
                Ok(true)
            } else {
                tui.last_msg = Some("no contract row selected".to_string());
                Ok(true)
            }
        }
        _ => Ok(false),
    }
}

fn handle_modal_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    enum ModalAction {
        None,
        Pending,
        Close,
        SalaryNext {
            target_id: PlayerId,
            target_name: String,
            salary_m: i64,
        },
        Submit {
            target_id: PlayerId,
            target_name: String,
            salary_m: i64,
            years: i64,
        },
    }

    let action = CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        match &mut cache.modal {
            Modal::None => ModalAction::None,
            Modal::ExtendSalary {
                input,
                target_id,
                target_name,
            } => match input.handle_key(key) {
                WidgetEvent::Submitted => match input.value() {
                    Some(salary_m) => ModalAction::SalaryNext {
                        target_id: *target_id,
                        target_name: target_name.clone(),
                        salary_m,
                    },
                    None => ModalAction::Pending,
                },
                WidgetEvent::Cancelled => ModalAction::Close,
                _ => ModalAction::Pending,
            },
            Modal::ExtendYears {
                input,
                target_id,
                target_name,
                salary_m,
            } => match input.handle_key(key) {
                WidgetEvent::Submitted => match input.value() {
                    Some(years) => ModalAction::Submit {
                        target_id: *target_id,
                        target_name: target_name.clone(),
                        salary_m: *salary_m,
                        years,
                    },
                    None => ModalAction::Pending,
                },
                WidgetEvent::Cancelled => ModalAction::Close,
                _ => ModalAction::Pending,
            },
        }
    });

    match action {
        ModalAction::None => Ok(false),
        ModalAction::Pending => Ok(true),
        ModalAction::Close => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            Ok(true)
        }
        ModalAction::SalaryNext {
            target_id,
            target_name,
            salary_m,
        } => {
            CACHE.with(|c| {
                c.borrow_mut().modal = Modal::ExtendYears {
                    input: NumberInput::new("Years (1-5)")
                        .with_bounds(1, 5)
                        .with_initial(3),
                    target_id,
                    target_name,
                    salary_m,
                };
            });
            Ok(true)
        }
        ModalAction::Submit {
            target_id: _,
            target_name,
            salary_m,
            years,
        } => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let res = with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Extend {
                        player: target_name.clone(),
                        salary_m: salary_m as f64,
                        years: years as u8,
                    },
                )
            });
            match res {
                Ok(()) => {
                    tui.last_msg = Some(format!(
                        "extension submitted for {} (${}M x {}yr)",
                        target_name, salary_m, years
                    ));
                    invalidate();
                    crate::tui::screens::roster::invalidate();
                    crate::tui::screens::home::invalidate();
                }
                Err(e) => {
                    tui.last_msg = Some(format!("{}: {}", t(tui.lang, T::CommonError), e));
                }
            }
            Ok(true)
        }
    }
}

fn current_row() -> Option<(PlayerId, String)> {
    CACHE.with(|c| {
        let cache = c.borrow();
        cache
            .snapshot
            .as_ref()
            .and_then(|s| s.rows.get(cache.cursor))
            .map(|r| (r.player_id, r.name.clone()))
    })
}

fn set_sort(sort: SortKey) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.sort = sort;
        apply_sort(&mut c);
    });
}

fn sort_key_from_char(c: char) -> Option<SortKey> {
    match c {
        't' => Some(SortKey::Total),
        'y' => Some(SortKey::Years),
        'n' => Some(SortKey::Name),
        _ => None,
    }
}

fn sort_label(lang: Lang, sort: SortKey) -> &'static str {
    match sort {
        SortKey::Total => t(lang, T::FinanceTotal),
        SortKey::Years => t(lang, T::FinanceYears),
        SortKey::Name => t(lang, T::RosterPlayer),
    }
}

fn money_cell(c: Cents) -> String {
    if c == Cents::ZERO {
        "-".to_string()
    } else {
        format!("{}", c)
    }
}

fn year_label(current: SeasonId, year: SeasonId) -> String {
    match year.0.checked_sub(current.0).unwrap_or(0) {
        0 => "Y1".to_string(),
        1 => "Y2".to_string(),
        2 => "Y3".to_string(),
        3 => "Y4".to_string(),
        _ => format!("{}", year.0),
    }
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_key_from_char_maps_footer_shortcuts() {
        assert_eq!(sort_key_from_char('t'), Some(SortKey::Total));
        assert_eq!(sort_key_from_char('y'), Some(SortKey::Years));
        assert_eq!(sort_key_from_char('n'), Some(SortKey::Name));
        assert_eq!(sort_key_from_char('x'), None);
        assert_eq!(sort_key_from_char('T'), None);
    }
}
