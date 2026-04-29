//! New-game wizard. Five steps (save path → team → mode → season → confirm).
//! On confirm, dispatches `Command::New` and transitions back to the
//! Menu screen with the new save loaded.
//!
//! State is kept in a thread-local since per-screen state can't live on the
//! Wave-0 `TuiApp`. `reset()` re-initializes it for a fresh wizard run; called
//! from the Saves overlay before pushing the NewGame screen.

use anyhow::{anyhow, Context, Result};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::cell::RefCell;
use std::path::PathBuf;

use crate::cli::{Command, NewArgs};
use crate::state::AppState;
use crate::tui::widgets::{
    centered_block, FormWidget, NumberInput, Picker, TextInput, Theme, WidgetEvent,
};
use crate::tui::{Screen, TuiApp};
use nba3k_core::{t, Lang, Team, T};

const SEED_DEFAULT_PATH: &str = "data/seed_2025_26.sqlite";
const MODES: &[&str] = &["standard", "god", "hardcore", "sandbox"];

// ---------------------------------------------------------------------------
// Wizard state
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, PartialEq, Eq)]
enum Step {
    SavePath,
    Team,
    Start,
    Mode,
    Season,
    Confirm,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum StartMode {
    Fresh,
    Today,
}

struct WizardState {
    step: Step,
    save_path: TextInput,
    team_picker: Picker<Team>,
    mode_picker: Picker<&'static str>,
    season: NumberInput,
    /// M33 — pick fresh October open vs live ESPN today's-state import.
    start_mode: StartMode,
    /// Last error from a failed dispatch (`new` overwrite refuse, invalid input).
    error: Option<String>,
}

impl Default for WizardState {
    fn default() -> Self {
        Self::new(Lang::En)
    }
}

impl WizardState {
    fn new(lang: Lang) -> Self {
        Self {
            step: Step::SavePath,
            save_path: TextInput::new(t(lang, T::NewGameSavePath))
                .with_initial(default_save_path()),
            team_picker: Picker::new(t(lang, T::NewGameTeam), load_teams(), display_team),
            mode_picker: Picker::new(
                t(lang, T::NewGameMode),
                MODES.to_vec(),
                |s: &&'static str| s.to_string(),
            ),
            season: NumberInput::new(t(lang, T::NewGameSeason))
                .with_bounds(2024, 2030)
                .with_initial(2026),
            start_mode: StartMode::Fresh,
            error: None,
        }
    }

    fn localize(&mut self, lang: Lang) {
        self.save_path.set_label(t(lang, T::NewGameSavePath));
        self.team_picker.set_title(t(lang, T::NewGameTeam));
        self.mode_picker.set_title(t(lang, T::NewGameMode));
        self.season.set_label(t(lang, T::NewGameSeason));
    }
}

thread_local! {
    static STATE: RefCell<WizardState> = RefCell::new(WizardState::default());
}

/// Re-initialize wizard state for a fresh run. Called from Saves overlay
/// before pushing `Screen::NewGame`.
pub fn reset() {
    STATE.with(|s| *s.borrow_mut() = WizardState::default());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_save_path() -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push("Desktop");
        p.push("nba3k_save");
        p.push("save.db");
        return p.display().to_string();
    }
    "nba3k_save.db".to_string()
}

/// Normalize the user-typed save path:
/// - trim whitespace
/// - if it ends with `/` or `\` → append `save.db`
/// - if it points to an existing directory → append `save.db`
/// - if it has no extension → append `.db`
/// - mkdir-p the parent directory so SQLite can create the file
fn normalize_save_path(input: &str) -> Result<PathBuf> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("save path is empty"));
    }
    let mut path = PathBuf::from(trimmed);
    let dir_like =
        trimmed.ends_with('/') || trimmed.ends_with('\\') || (path.exists() && path.is_dir());
    if dir_like {
        path.push("save.db");
    } else if path.extension().is_none() {
        path.set_extension("db");
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent directory {}", parent.display()))?;
        }
    }
    Ok(path)
}

fn display_team(t: &Team) -> String {
    format!("{:<5}  {} {}", t.abbrev, t.city, t.name)
}

/// Best-effort load of the 30 teams from the bundled seed DB. If the seed
/// isn't present the picker falls back to an empty list and the wizard will
/// surface a helpful error on confirm.
fn load_teams() -> Vec<Team> {
    let path = PathBuf::from(SEED_DEFAULT_PATH);
    if !path.exists() {
        return Vec::new();
    }
    match nba3k_store::Store::open(&path) {
        Ok(store) => store.list_teams().unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, _app: &mut AppState, tui: &TuiApp) {
    let title = t(tui.lang, T::NewGameTitle);
    let submit = t(tui.lang, T::CommonSubmit);
    let back = t(tui.lang, T::CommonBack);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(0),    // body
            Constraint::Length(4), // status / error
        ])
        .split(area);

    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.localize(tui.lang);
        draw_header(f, parts[0], theme, tui.lang, title, st.step);
        draw_body(f, parts[1], theme, tui.lang, &st);
        draw_status(
            f,
            parts[2],
            theme,
            tui.lang,
            submit,
            back,
            st.error.as_deref(),
        );
    });
}

fn draw_header(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, title: &str, step: Step) {
    let labels = [
        ("1", t(lang, T::NewGameSavePath), Step::SavePath),
        ("2", t(lang, T::NewGameTeam), Step::Team),
        ("3", t(lang, T::NewGameStartTitle), Step::Start),
        ("4", t(lang, T::NewGameMode), Step::Mode),
        ("5", t(lang, T::NewGameSeason), Step::Season),
        ("6", t(lang, T::NewGameConfirm), Step::Confirm),
    ];
    let mut spans: Vec<Span> = Vec::new();
    for (i, (n, label, s)) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ›  ", theme.muted_style()));
        }
        let active = *s == step;
        let style = if active {
            theme.highlight()
        } else {
            theme.muted_style()
        };
        spans.push(Span::styled(format!(" {}.{} ", n, label), style));
    }
    let p = Paragraph::new(Line::from(spans)).block(theme.block(title));
    f.render_widget(p, area);
}

fn draw_body(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, st: &WizardState) {
    match st.step {
        Step::SavePath => {
            let inner = vsplit(area, 4);
            let lines = vec![
                Line::from(Span::styled(
                    t(lang, T::NewGameSavePath),
                    theme.accent_style(),
                )),
                Line::from(Span::styled(default_save_path(), theme.muted_style())),
                Line::from(Span::styled(t(lang, T::SavesLoad), theme.accent_style())),
            ];
            let p = Paragraph::new(lines)
                .block(theme.block(""))
                .wrap(Wrap { trim: false });
            f.render_widget(p, inner.0);
            st.save_path.render(f, inner.1, theme);
        }
        Step::Team => {
            if st.team_picker.items().is_empty() {
                centered_block(
                    f,
                    area,
                    theme,
                    t(lang, T::NewGameTeam),
                    &[t(lang, T::CommonError), "", SEED_DEFAULT_PATH],
                );
                return;
            }
            render_team_picker(f, area, theme, lang, &st.team_picker);
        }
        Step::Start => {
            render_start_picker(f, area, theme, lang, st.start_mode);
        }
        Step::Mode => {
            render_mode_picker(f, area, theme, lang, &st.mode_picker);
        }
        Step::Season => {
            let inner = vsplit(area, 3);
            let lines = vec![
                Line::from(Span::styled(
                    t(lang, T::NewGameSeason),
                    theme.accent_style(),
                )),
                Line::from(Span::styled("2024-2030", theme.muted_style())),
            ];
            let p = Paragraph::new(lines)
                .block(theme.block(""))
                .wrap(Wrap { trim: false });
            f.render_widget(p, inner.0);
            render_text_value(
                f,
                inner.1,
                theme,
                t(lang, T::NewGameSeason),
                st.season.raw(),
            );
        }
        Step::Confirm => {
            let team_label = st
                .team_picker
                .selected()
                .map(display_team)
                .unwrap_or_else(|| "(none)".into());
            let mode_label = st
                .mode_picker
                .selected()
                .map(|s| (*s).to_string())
                .unwrap_or_else(|| "standard".into());
            let season_label = st
                .season
                .value()
                .map(|n| n.to_string())
                .unwrap_or_else(|| st.season.raw().to_string());
            let start_label = match st.start_mode {
                StartMode::Fresh => t(lang, T::NewGameStartFresh),
                StartMode::Today => t(lang, T::NewGameStartToday),
            };
            let lines = vec![
                Line::from(Span::styled(
                    t(lang, T::NewGameConfirm),
                    theme.accent_style(),
                )),
                Line::from(""),
                kv_line(theme, t(lang, T::NewGameSavePath), st.save_path.value()),
                kv_line(theme, t(lang, T::NewGameTeam), &team_label),
                kv_line(theme, t(lang, T::NewGameStartTitle), start_label),
                kv_line(theme, t(lang, T::NewGameMode), &mode_label),
                kv_line(theme, t(lang, T::NewGameSeason), &season_label),
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "Enter {} · Esc {}",
                        t(lang, T::CommonConfirm),
                        t(lang, T::CommonBack)
                    ),
                    theme.muted_style(),
                )),
            ];
            let p = Paragraph::new(lines).block(theme.block(""));
            f.render_widget(p, area);
        }
    }
}

fn render_text_value(f: &mut Frame, area: Rect, theme: &Theme, label: &str, value: &str) {
    let line = Line::from(vec![
        Span::styled(format!(" {} ", label), theme.accent_style()),
        Span::styled(value.to_string(), theme.text()),
        Span::styled("█", theme.text()),
    ]);
    f.render_widget(Paragraph::new(line).block(theme.block("")), area);
}

fn render_start_picker(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    lang: Lang,
    selected: StartMode,
) {
    let items = vec![
        (StartMode::Fresh, t(lang, T::NewGameStartFresh)),
        (StartMode::Today, t(lang, T::NewGameStartToday)),
    ];
    let title = format!(" {} (↑↓) ", t(lang, T::NewGameStartTitle));
    let lines: Vec<ListItem> = items
        .iter()
        .map(|(mode, label)| {
            let style = if *mode == selected {
                theme.highlight()
            } else {
                theme.text()
            };
            ListItem::new(Line::from(Span::styled(label.to_string(), style)))
        })
        .collect();
    let inner = vsplit(area, 5);
    f.render_widget(List::new(lines).block(theme.block(&title)), inner.0);
    let hint = Paragraph::new(Line::from(Span::styled(
        t(lang, T::NewGameStartTodayHint),
        theme.muted_style(),
    )))
    .block(theme.block(""))
    .wrap(Wrap { trim: false });
    f.render_widget(hint, inner.1);
}

fn render_team_picker(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, picker: &Picker<Team>) {
    render_picker_lines(
        f,
        area,
        theme,
        t(lang, T::NewGameTeam),
        picker.items().iter().map(display_team).collect(),
        picker.selected_index(),
        picker.filter(),
        lang,
    );
}

fn render_mode_picker(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    lang: Lang,
    picker: &Picker<&'static str>,
) {
    render_picker_lines(
        f,
        area,
        theme,
        t(lang, T::NewGameMode),
        picker.items().iter().map(|s| (*s).to_string()).collect(),
        picker.selected_index(),
        picker.filter(),
        lang,
    );
}

fn render_picker_lines(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    labels: Vec<String>,
    selected_index: Option<usize>,
    filter: &str,
    lang: Lang,
) {
    let filter_lc = filter.to_lowercase();
    let visible: Vec<(usize, String)> = labels
        .into_iter()
        .enumerate()
        .filter(|(_, label)| filter_lc.is_empty() || label.to_lowercase().contains(&filter_lc))
        .collect();
    let count = visible.len();
    let items: Vec<ListItem> = visible
        .into_iter()
        .map(|(i, label)| {
            let style = if Some(i) == selected_index {
                theme.highlight()
            } else {
                theme.text()
            };
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect();
    let block_title = if filter_lc.is_empty() {
        format!(" {} ({}) ", title, count)
    } else {
        format!(
            " {} ({}) - {}: {} ",
            title,
            count,
            t(lang, T::CommonFilter),
            filter
        )
    };
    f.render_widget(List::new(items).block(theme.block(&block_title)), area);
}

fn kv_line<'a>(theme: &Theme, k: &'a str, v: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {:<8}", k), theme.muted_style()),
        Span::styled(v.to_string(), theme.text()),
    ])
}

fn vsplit(area: Rect, top: u16) -> (Rect, Rect) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(top), Constraint::Min(0)])
        .split(area);
    (parts[0], parts[1])
}

fn draw_status(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    lang: Lang,
    submit: &str,
    back: &str,
    error: Option<&str>,
) {
    let line = match error {
        None => Line::from(Span::styled(
            format!(
                "Enter {} · Esc {} · {}",
                submit,
                back,
                t(lang, T::CommonFilter)
            ),
            theme.muted_style(),
        )),
        Some(e) => Line::from(Span::styled(format!("error: {}", e), theme.accent_style())),
    };
    let p = Paragraph::new(line).block(theme.block(""));
    f.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    let action = STATE.with(|s| {
        let mut st = s.borrow_mut();
        // Confirm step has its own key map (Enter = create, Esc = back).
        if st.step == Step::Confirm {
            match key.code {
                KeyCode::Enter => return WizardAction::Submit,
                KeyCode::Esc => {
                    st.step = Step::Season;
                    return WizardAction::Consumed;
                }
                _ => return WizardAction::Consumed,
            }
        }
        // The Start step is a 2-option toggle, not a Picker — handle it inline.
        if st.step == Step::Start {
            match key.code {
                KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
                    st.start_mode = match st.start_mode {
                        StartMode::Fresh => StartMode::Today,
                        StartMode::Today => StartMode::Fresh,
                    };
                    return WizardAction::Consumed;
                }
                KeyCode::Enter => return advance_step(&mut st),
                KeyCode::Esc => return retreat_step(&mut st),
                _ => return WizardAction::Consumed,
            }
        }
        // Other steps delegate to the active widget; on Submitted move forward.
        let ev = match st.step {
            Step::SavePath => st.save_path.handle_key(key),
            Step::Team => st.team_picker.handle_key(key),
            Step::Start => unreachable!(),
            Step::Mode => st.mode_picker.handle_key(key),
            Step::Season => st.season.handle_key(key),
            Step::Confirm => unreachable!(),
        };
        match ev {
            WidgetEvent::Submitted => advance_step(&mut st),
            WidgetEvent::Cancelled => retreat_step(&mut st),
            _ => WizardAction::Consumed,
        }
    });

    match action {
        WizardAction::Consumed => Ok(true),
        WizardAction::ExitToMenu => {
            tui.current = Screen::Menu;
            Ok(true)
        }
        WizardAction::Submit => {
            let res = submit(app, tui);
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                match &res {
                    Ok(()) => st.error = None,
                    Err(e) => st.error = Some(e.to_string()),
                }
            });
            if res.is_ok() {
                reset();
                tui.invalidate_caches();
                crate::tui::screens::home::invalidate();
                crate::tui::screens::saves::invalidate();
                tui.show_home_preview();
            }
            Ok(true)
        }
    }
}

enum WizardAction {
    Consumed,
    ExitToMenu,
    Submit,
}

fn advance_step(st: &mut WizardState) -> WizardAction {
    // Per-step validation before moving forward.
    match st.step {
        Step::SavePath => {
            let v = st.save_path.value().trim();
            if v.is_empty() {
                st.error = Some("save path cannot be empty".into());
                return WizardAction::Consumed;
            }
            st.error = None;
            st.step = Step::Team;
        }
        Step::Team => {
            if st.team_picker.selected().is_none() {
                st.error = Some("pick a team (type to filter, ↑↓ to move)".into());
                return WizardAction::Consumed;
            }
            st.error = None;
            st.step = Step::Start;
        }
        Step::Start => {
            st.error = None;
            st.step = Step::Mode;
        }
        Step::Mode => {
            if st.mode_picker.selected().is_none() {
                st.error = Some("pick a mode".into());
                return WizardAction::Consumed;
            }
            st.error = None;
            st.step = Step::Season;
        }
        Step::Season => {
            if st.season.value().is_none() {
                st.error = Some("season must be 2024-2030".into());
                return WizardAction::Consumed;
            }
            st.error = None;
            st.step = Step::Confirm;
        }
        Step::Confirm => unreachable!(),
    }
    WizardAction::Consumed
}

fn retreat_step(st: &mut WizardState) -> WizardAction {
    st.error = None;
    match st.step {
        Step::SavePath => return WizardAction::ExitToMenu,
        Step::Team => st.step = Step::SavePath,
        Step::Start => st.step = Step::Team,
        Step::Mode => st.step = Step::Start,
        Step::Season => st.step = Step::Mode,
        Step::Confirm => st.step = Step::Season,
    }
    WizardAction::Consumed
}

/// Build `NewArgs` from the wizard state and dispatch `Command::New`. On
/// success, refresh the shell context so the rest of the TUI sees the new
/// save's `season_state` immediately.
fn submit(app: &mut AppState, tui: &mut TuiApp) -> Result<()> {
    let (save_path, args) = STATE.with(|s| -> Result<(PathBuf, NewArgs)> {
        let st = s.borrow();
        let save_path = normalize_save_path(st.save_path.value())?;
        let team = st
            .team_picker
            .selected()
            .ok_or_else(|| anyhow!("no team selected"))?
            .abbrev
            .clone();
        let mode = st
            .mode_picker
            .selected()
            .copied()
            .unwrap_or("standard")
            .to_string();
        let season = st.season.value().ok_or_else(|| anyhow!("season not set"))? as u16;
        let from_today = matches!(st.start_mode, StartMode::Today);
        Ok((
            save_path,
            NewArgs {
                team,
                mode,
                season,
                seed: rand::random::<u64>(),
                from_today,
            },
        ))
    })?;

    if save_path.exists() {
        return Err(anyhow!(
            "{} already exists — pick a different path",
            save_path.display()
        ));
    }

    // The CLI `new` command reads `app.save_path` for the destination, so we
    // populate it before dispatching. `cmd_new` then opens the freshly created
    // file via `app.open_path`, leaving the new save loaded for us.
    app.save_path = Some(save_path.clone());

    crate::tui::with_silenced_io(|| crate::commands::dispatch(app, Command::New(args)))
        .with_context(|| format!("creating save {}", save_path.display()))?;

    // Refresh shell context against the newly created save. `refresh_save_ctx`
    // re-reads season_state, user team, and user_abbrev in one shot.
    tui.refresh_save_ctx(app)?;
    tui.last_msg = Some(format!("created {}", save_path.display()));
    tui.show_home_preview();
    Ok(())
}
