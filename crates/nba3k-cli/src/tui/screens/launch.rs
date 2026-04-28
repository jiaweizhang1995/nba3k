//! Full-screen launch menu. The shell owns `Screen::Launch`; this module owns
//! the row state, newest-save discovery, and launch-specific key behavior.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::state::AppState;
use crate::tui::widgets::Theme;
use crate::tui::{Screen, TuiApp};
use nba3k_core::{t, T};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum LaunchRow {
    Continue,
    NewGame,
    LoadGame,
    Settings,
    Quit,
}

#[derive(Clone, Debug)]
struct SaveSummary {
    path: PathBuf,
    team: Option<String>,
    season: Option<u16>,
    mtime: Option<SystemTime>,
    loadable: bool,
}

#[derive(Clone, Debug)]
struct LaunchState {
    cursor: usize,
    saves: Vec<SaveSummary>,
    preferred_save: Option<PathBuf>,
}

impl Default for LaunchState {
    fn default() -> Self {
        Self {
            cursor: 0,
            saves: Vec::new(),
            preferred_save: None,
        }
    }
}

/// Key outcome that the shell can use for variants that may not exist in this
/// module's narrow write scope yet.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LaunchAction {
    None,
    Consumed,
}

thread_local! {
    static STATE: RefCell<LaunchState> = RefCell::new(LaunchState::default());
}

pub fn invalidate() {
    STATE.with(|s| *s.borrow_mut() = LaunchState::default());
}

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, _tui: &TuiApp) {
    refresh_scan(app);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(area);

    draw_title(f, parts[0], theme);
    STATE.with(|s| {
        let st = s.borrow();
        draw_rows(f, parts[1], theme, &st, _tui);
        draw_footer(f, parts[2], theme, &st, _tui);
    });
}

fn draw_title(f: &mut Frame, area: Rect, theme: &Theme) {
    let lines = vec![
        Line::from(Span::styled(
            " _   _ ____    _    _____ _  __",
            theme.accent_style(),
        )),
        Line::from(Span::styled(
            "| \\ | | __ )  / \\  |___ /| |/ /",
            theme.accent_style(),
        )),
        Line::from(Span::styled(
            "|  \\| |  _ \\ / _ \\   |_ \\| ' / ",
            theme.accent_style(),
        )),
        Line::from(Span::styled(
            "| |\\  | |_) / ___ \\ ___) | . \\ ",
            theme.accent_style(),
        )),
        Line::from(Span::styled(
            "|_| \\_|____/_/   \\_\\____/|_|\\_\\",
            theme.accent_style(),
        )),
    ];
    let p = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(theme.block(""));
    f.render_widget(p, area);
}

fn draw_rows(f: &mut Frame, area: Rect, theme: &Theme, st: &LaunchState, tui: &TuiApp) {
    let rows = visible_rows(st);
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let selected = idx == st.cursor;
            let style = if selected {
                theme.highlight()
            } else {
                theme.text()
            };
            let marker = if selected { "> " } else { "  " };
            ListItem::new(Line::from(Span::styled(
                format!("{}{}", marker, row_label(tui, *row)),
                style,
            )))
        })
        .collect();

    let list = List::new(items).block(theme.block(t(tui.lang, T::AppName)));
    f.render_widget(list, centered(area, 34, rows.len() as u16 + 2));
}

fn draw_footer(f: &mut Frame, area: Rect, theme: &Theme, st: &LaunchState, tui: &TuiApp) {
    let Some(save) = selected_continue_save(st) else {
        let hint = footer_hint(theme, tui);
        f.render_widget(Paragraph::new(hint).block(theme.block("")), area);
        return;
    };

    let label = if save.loadable {
        format!(
            "{}: {} - Season {} - {}",
            t(tui.lang, T::LaunchLastSave),
            save.team.clone().unwrap_or_else(|| "-".into()),
            season_label(save.season),
            save.path.display()
        )
    } else {
        format!("Last save unavailable: {}", save.path.display())
    };
    let lines = vec![
        Line::from(Span::styled(label, theme.muted_style())),
        footer_hint(theme, tui),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(theme.block("")),
        area,
    );
}

/// Handles launch-screen keys. Existing screens are switched directly.
pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<LaunchAction> {
    refresh_scan(app);

    match key.code {
        KeyCode::Up => {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                if st.cursor > 0 {
                    st.cursor -= 1;
                }
            });
            Ok(LaunchAction::Consumed)
        }
        KeyCode::Down => {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                let max = visible_rows(&st).len().saturating_sub(1);
                if st.cursor < max {
                    st.cursor += 1;
                }
            });
            Ok(LaunchAction::Consumed)
        }
        KeyCode::Enter => activate_selected(app, tui),
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            tui.current = Screen::QuitConfirm;
            Ok(LaunchAction::Consumed)
        }
        _ => Ok(LaunchAction::None),
    }
}

fn activate_selected(app: &mut AppState, tui: &mut TuiApp) -> Result<LaunchAction> {
    let row = STATE.with(|s| {
        let st = s.borrow();
        visible_rows(&st).get(st.cursor).copied()
    });

    match row {
        Some(LaunchRow::Continue) => {
            let path = STATE.with(|s| selected_continue_save(&s.borrow()).map(|save| save.path));
            let Some(path) = path else {
                tui.last_msg = Some("nothing to continue".into());
                return Ok(LaunchAction::Consumed);
            };
            let res = crate::tui::with_silenced_io(|| tui.switch_save(app, path.clone()));
            match res {
                Ok(()) => {
                    tui.last_msg = Some(format!("loaded {}", path.display()));
                    crate::tui::screens::home::invalidate();
                    tui.invalidate_caches();
                    tui.current = Screen::Menu;
                }
                Err(e) => {
                    tui.last_msg = Some(format!("load failed: {}", e));
                }
            }
            Ok(LaunchAction::Consumed)
        }
        Some(LaunchRow::NewGame) => {
            tui.current = Screen::NewGame;
            crate::tui::screens::new_game::reset();
            Ok(LaunchAction::Consumed)
        }
        Some(LaunchRow::LoadGame) => {
            tui.current = Screen::Saves;
            Ok(LaunchAction::Consumed)
        }
        Some(LaunchRow::Settings) => {
            tui.current = Screen::Settings;
            crate::tui::screens::settings::reset(crate::tui::settings_choice(tui.lang));
            Ok(LaunchAction::Consumed)
        }
        Some(LaunchRow::Quit) => {
            tui.current = Screen::QuitConfirm;
            Ok(LaunchAction::Consumed)
        }
        None => Ok(LaunchAction::None),
    }
}

fn refresh_scan(app: &AppState) {
    let preferred = app.save_path.as_ref().filter(|p| p.is_file()).cloned();

    let should_scan = STATE.with(|s| {
        let st = s.borrow();
        st.saves.is_empty() || st.preferred_save != preferred
    });
    if !should_scan {
        return;
    }

    let mut saves = scan_saves();
    if let Some(path) = preferred.as_ref() {
        if !saves.iter().any(|s| s.path == *path) {
            saves.push(read_save_summary(path.clone()));
        }
    }
    sort_saves(&mut saves);

    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.saves = saves;
        st.preferred_save = preferred;
        st.cursor = st.cursor.min(visible_rows(&st).len().saturating_sub(1));
    });
}

fn scan_saves() -> Vec<SaveSummary> {
    let mut out = Vec::new();
    scan_db_files(Path::new("data").join("saves").as_path(), &mut out);
    out.sort();
    out.dedup();
    out.into_iter().map(read_save_summary).collect()
}

fn scan_db_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("db") {
            out.push(path);
        }
    }
}

fn read_save_summary(path: PathBuf) -> SaveSummary {
    let mtime = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok());
    match nba3k_store::Store::open(&path) {
        Ok(store) => {
            let state = store.load_season_state().ok().flatten();
            let team = match &state {
                Some(s) => store.team_abbrev(s.user_team).ok().flatten(),
                None => store.get_meta("user_team").ok().flatten(),
            };
            let season = state.as_ref().map(|s| s.season.0).or_else(|| {
                store
                    .get_meta("season")
                    .ok()
                    .flatten()
                    .and_then(|s| s.parse::<u16>().ok())
            });
            SaveSummary {
                path,
                team,
                season,
                mtime,
                loadable: state.is_some(),
            }
        }
        Err(_) => SaveSummary {
            path,
            team: None,
            season: None,
            mtime,
            loadable: false,
        },
    }
}

fn sort_saves(saves: &mut [SaveSummary]) {
    saves.sort_by(|a, b| match (a.mtime, b.mtime) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.path.cmp(&b.path),
    });
}

fn selected_continue_save(st: &LaunchState) -> Option<SaveSummary> {
    if let Some(preferred) = st.preferred_save.as_ref() {
        if let Some(save) = st
            .saves
            .iter()
            .find(|s| s.path == *preferred && s.loadable)
            .cloned()
        {
            return Some(save);
        }
    }
    st.saves.iter().find(|s| s.loadable).cloned()
}

fn visible_rows(st: &LaunchState) -> Vec<LaunchRow> {
    let mut rows = Vec::new();
    if selected_continue_save(st).is_some() {
        rows.push(LaunchRow::Continue);
    }
    rows.extend([
        LaunchRow::NewGame,
        LaunchRow::LoadGame,
        LaunchRow::Settings,
        LaunchRow::Quit,
    ]);
    rows
}

fn row_label(tui: &TuiApp, row: LaunchRow) -> &'static str {
    match row {
        LaunchRow::Continue => t(tui.lang, T::LaunchContinue),
        LaunchRow::NewGame => t(tui.lang, T::LaunchNewGame),
        LaunchRow::LoadGame => t(tui.lang, T::LaunchLoadGame),
        LaunchRow::Settings => t(tui.lang, T::LaunchSettings),
        LaunchRow::Quit => t(tui.lang, T::LaunchQuit),
    }
}

fn footer_hint<'a>(theme: &Theme, tui: &TuiApp) -> Line<'a> {
    Line::from(vec![
        Span::styled(" Up/Down ", theme.accent_style()),
        Span::styled(format!("{}   ", t(tui.lang, T::CommonMove)), theme.text()),
        Span::styled(" Enter ", theme.accent_style()),
        Span::styled(format!("{}   ", t(tui.lang, T::CommonOpen)), theme.text()),
        Span::styled(" q/Esc ", theme.accent_style()),
        Span::styled(t(tui.lang, T::CommonQuit), theme.text()),
    ])
}

fn season_label(season: Option<u16>) -> String {
    match season {
        Some(year) if year > 0 => format!("{}-{:02}", year - 1, year % 100),
        Some(year) => year.to_string(),
        None => "-".into(),
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}
