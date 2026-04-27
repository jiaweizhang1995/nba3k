//! M20 TUI shell. Renders the 7-item left-side menu mockup; per-screen
//! content is delegated to `screens::*`. Pre-M20 5-tab dashboard is preserved
//! at `tui --legacy` (see `screens::legacy`).
//!
//! Wave 0 owns: menu nav, terminal lifecycle, event-loop dispatch, theme,
//! widget API. Wave 1 fills the Home / Calendar / Saves / NewGame screens —
//! some types (e.g. `Screen::NewGame`) and helpers are unreferenced until
//! then, so `dead_code` is allowed module-wide for the duration of Wave 0.

#![allow(dead_code)]

pub mod widgets;
pub mod screens;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};
use std::io;
use std::path::PathBuf;

use crate::state::AppState;
use crate::tui::widgets::{ActionBar, Confirm, FormWidget, Theme, WidgetEvent};
use nba3k_core::{Cents, SeasonId, SeasonState, TeamId};

// ---------------------------------------------------------------------------
// Menu / Screen enums
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Home,
    Roster,
    Rotation,
    Trades,
    Draft,
    Finance,
    Calendar,
}

impl MenuItem {
    pub const ALL: [MenuItem; 7] = [
        MenuItem::Home,
        MenuItem::Roster,
        MenuItem::Rotation,
        MenuItem::Trades,
        MenuItem::Draft,
        MenuItem::Finance,
        MenuItem::Calendar,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MenuItem::Home => "Home",
            MenuItem::Roster => "Roster",
            MenuItem::Rotation => "Rotation",
            MenuItem::Trades => "Trades",
            MenuItem::Draft => "Draft",
            MenuItem::Finance => "Finance",
            MenuItem::Calendar => "Calendar",
        }
    }

    pub fn screen(self) -> Screen {
        match self {
            MenuItem::Home => Screen::Home,
            MenuItem::Roster => Screen::Roster,
            MenuItem::Rotation => Screen::Rotation,
            MenuItem::Trades => Screen::Trades,
            MenuItem::Draft => Screen::Draft,
            MenuItem::Finance => Screen::Finance,
            MenuItem::Calendar => Screen::Calendar,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Menu,
    Home,
    Roster,
    Rotation,
    Trades,
    Draft,
    Finance,
    Calendar,
    Saves,
    NewGame,
    QuitConfirm,
}

// ---------------------------------------------------------------------------
// TuiApp — top-level state
// ---------------------------------------------------------------------------

/// Snapshot of save-derived state. Built by `TuiApp::refresh_save_ctx` from
/// `app.store()` and mirrored onto `TuiApp` for ergonomic field access from
/// screens. Wave-1 screens may read either `tui.save_ctx.as_ref()` or the
/// mirrored fields directly — both are kept in sync by the shell.
#[derive(Clone, Debug)]
pub struct SaveCtx {
    pub user_team: TeamId,
    pub user_abbrev: String,
    pub season: SeasonId,
    pub season_state: SeasonState,
}

impl SaveCtx {
    /// Read all save-derived state from the store. Returns `Ok(None)` if the
    /// save has no `season_state` row yet (fresh DB pre-`new`).
    fn load(app: &mut AppState) -> Result<Option<Self>> {
        let store = app.store()?;
        let Some(season_state) = store.load_season_state()? else {
            return Ok(None);
        };
        let user_team = season_state.user_team;
        let user_abbrev = store
            .team_abbrev(user_team)?
            .unwrap_or_else(|| format!("T{}", user_team.0));
        Ok(Some(Self {
            user_team,
            user_abbrev,
            season: season_state.season,
            season_state,
        }))
    }
}

/// Top-level TUI state. Renamed from M19's `TuiState`. Held by reference for
/// the duration of `run`. Per-screen modules read fields directly (the shell
/// is single-threaded, no locks needed).
///
/// **No-save mode.** When the shell launches with no `--save` / no
/// `season_state`, `save_ctx` is `None` and the shell forces
/// `Screen::NewGame` so the wizard is the only reachable entry. The mirrored
/// `user_team` / `user_abbrev` / `season` / `season_state` / `payroll` fields
/// hold safe-default values in that mode — Wave-1 screens that read them
/// must guard via `has_save()` first.
pub struct TuiApp {
    /// Current screen (drives draw + key dispatch).
    pub current: Screen,
    /// Cursor in the menu (0..7). Wraps on ↑/↓.
    pub menu_selected: usize,
    /// Theme palette (DEFAULT or TV).
    pub theme: Theme,

    /// Save-derived snapshot. `None` until a save is loaded. Wave-1 screens
    /// can use either this or the mirrored fields below.
    pub save_ctx: Option<SaveCtx>,

    // ---- Mirror of `save_ctx` fields (zero/empty when no save). ----
    /// User team id. `TeamId(0)` when no save loaded.
    pub user_team: TeamId,
    /// User team abbreviation. Empty string when no save loaded.
    pub user_abbrev: String,
    /// Current season. `SeasonId(0)` when no save loaded.
    pub season: SeasonId,
    /// Current season state. Default-zeroed when no save loaded.
    pub season_state: SeasonState,
    /// Lazy cache for header payroll. Cleared via `invalidate_caches`.
    pub payroll: Option<Cents>,

    /// Last action message (sim result, error, role change, etc.). Shown
    /// left-aligned in the action bar.
    pub last_msg: Option<String>,

    /// Confirm widget shown when QuitConfirm is current.
    quit_confirm: Confirm,
}

impl TuiApp {
    fn new(theme: Theme, save_ctx: Option<SaveCtx>) -> Self {
        let mut app = Self {
            current: if save_ctx.is_some() {
                Screen::Menu
            } else {
                Screen::NewGame
            },
            menu_selected: 0,
            theme,
            save_ctx: None,
            user_team: TeamId(0),
            user_abbrev: String::new(),
            season: SeasonId(0),
            season_state: empty_season_state(),
            payroll: None,
            last_msg: None,
            quit_confirm: Confirm::new("Quit nba3k?"),
        };
        app.set_save_ctx(save_ctx);
        app
    }

    /// True iff a save is loaded and has `season_state`. Wave-1 screens use
    /// this to gate "needs save" rendering.
    pub fn has_save(&self) -> bool {
        self.save_ctx.is_some()
    }

    /// Replace `save_ctx` and re-mirror its fields onto self. Internal helper
    /// used by `new`, `refresh_save_ctx`, and `switch_save`.
    fn set_save_ctx(&mut self, ctx: Option<SaveCtx>) {
        match ctx {
            Some(c) => {
                self.user_team = c.user_team;
                self.user_abbrev = c.user_abbrev.clone();
                self.season = c.season;
                self.season_state = c.season_state.clone();
                self.payroll = None;
                self.save_ctx = Some(c);
            }
            None => {
                self.user_team = TeamId(0);
                self.user_abbrev = String::new();
                self.season = SeasonId(0);
                self.season_state = empty_season_state();
                self.payroll = None;
                self.save_ctx = None;
            }
        }
    }

    /// Drop screen-specific caches. Call after any sim / mutation.
    pub fn invalidate_caches(&mut self) {
        self.payroll = None;
    }

    /// Re-read save-derived state from the store. Call after the new-game
    /// wizard finishes, after `switch_save`, or whenever you mutate the save.
    /// If the save has no `season_state` (fresh / corrupt), `save_ctx` is
    /// cleared back to `None` and the mirror fields reset to defaults.
    pub fn refresh_save_ctx(&mut self, app: &mut AppState) -> Result<()> {
        let ctx = SaveCtx::load(app)?;
        self.set_save_ctx(ctx);
        Ok(())
    }

    /// Open a different save file and refresh `save_ctx`. Used by the Saves
    /// overlay when the user picks "Load" on a different file.
    pub fn switch_save(&mut self, app: &mut AppState, new_path: PathBuf) -> Result<()> {
        app.open_path(new_path)?;
        self.refresh_save_ctx(app)
    }

    /// Re-read just the season_state row (no payroll/team flush). Used by sim
    /// paths that already invalidate other caches separately.
    pub fn refresh_season_state(&mut self, app: &mut AppState) -> Result<()> {
        if !self.has_save() {
            return Ok(());
        }
        if let Some(s) = app.store()?.load_season_state()? {
            self.season_state = s.clone();
            self.season = s.season;
            self.user_team = s.user_team;
            if let Some(ctx) = self.save_ctx.as_mut() {
                ctx.season_state = s.clone();
                ctx.season = s.season;
                ctx.user_team = s.user_team;
            }
        }
        Ok(())
    }
}

/// Construct a zeroed `SeasonState` for use when no save is loaded.
/// Mirrored field reads return this; screens must `has_save()`-guard before
/// trusting any field except as a placeholder.
fn empty_season_state() -> SeasonState {
    use nba3k_core::{GameMode, SeasonPhase};
    SeasonState {
        season: SeasonId(0),
        phase: SeasonPhase::PreSeason,
        day: 0,
        user_team: TeamId(0),
        mode: GameMode::Standard,
        rng_seed: 0,
    }
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

/// New M20 TUI entry — 7-menu shell. The shell tolerates a no-save launch:
/// when no `--save` is passed (or the save has no `season_state` yet), we
/// enter `Screen::NewGame` and the wizard becomes the only reachable screen
/// until it creates / loads a save. The `--legacy` flag is wired separately
/// through `run_legacy`.
pub fn run(app: &mut AppState, tv: bool) -> Result<()> {
    let theme = if tv { Theme::TV } else { Theme::DEFAULT };

    // Best-effort load: if the store can't open or the save has no
    // season_state, fall through with `save_ctx = None` and let the wizard
    // take over instead of aborting.
    let save_ctx = match app.store() {
        Ok(_) => SaveCtx::load(app).unwrap_or(None),
        Err(_) => None,
    };

    let mut tui = TuiApp::new(theme, save_ctx);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, app, &mut tui);

    // Always restore terminal even on error.
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    result
}

/// `tui --legacy` entry — preserves the M19 5-tab dashboard.
pub fn run_legacy(app: &mut AppState) -> Result<()> {
    screens::legacy::run(app)
}

// ---------------------------------------------------------------------------
// Event loop
// ---------------------------------------------------------------------------

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    tui: &mut TuiApp,
) -> Result<()> {
    loop {
        ensure_shell_cache(app, tui)?;
        terminal.draw(|f| draw(f, app, tui))?;

        let Event::Key(k) = event::read()? else { continue };
        if k.kind == KeyEventKind::Release {
            continue;
        }

        let exit = handle_key(app, tui, k)?;
        if exit {
            break;
        }
    }
    Ok(())
}

/// Returns Ok(true) when the shell should exit.
fn handle_key(app: &mut AppState, tui: &mut TuiApp, k: KeyEvent) -> Result<bool> {
    // QuitConfirm modal swallows everything until resolved.
    if tui.current == Screen::QuitConfirm {
        match tui.quit_confirm.handle_key(k) {
            WidgetEvent::Submitted => return Ok(true),
            WidgetEvent::Cancelled => {
                tui.current = Screen::Menu;
            }
            _ => {}
        }
        return Ok(false);
    }

    // Global shortcuts (Ctrl+S = saves overlay) work from any screen except
    // QuitConfirm (handled above) and the wizard before a save exists (the
    // saves overlay needs a loaded save to render its list).
    if k.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(k.code, KeyCode::Char('s'))
        && tui.has_save()
    {
        tui.current = Screen::Saves;
        return Ok(false);
    }

    match tui.current {
        Screen::Menu => menu_key(app, tui, k),
        Screen::Home => inner_screen_key(app, tui, k, screens::home::handle_key),
        Screen::Calendar => inner_screen_key(app, tui, k, screens::calendar::handle_key),
        Screen::Saves => inner_screen_key(app, tui, k, screens::saves::handle_key),
        Screen::NewGame => inner_screen_key(app, tui, k, screens::new_game::handle_key),
        Screen::Roster | Screen::Rotation | Screen::Trades | Screen::Draft | Screen::Finance => {
            stub_key(tui, k)
        }
        Screen::QuitConfirm => Ok(false), // unreachable
    }
}

/// Common inner-screen wrapper: route key into screen handler; if the screen
/// didn't consume it, apply the shell-wide bindings (Esc → Menu, but only
/// when a save is loaded — otherwise Esc on NewGame opens the quit confirm).
fn inner_screen_key<F>(
    app: &mut AppState,
    tui: &mut TuiApp,
    k: KeyEvent,
    handler: F,
) -> Result<bool>
where
    F: FnOnce(&mut AppState, &mut TuiApp, KeyEvent) -> Result<bool>,
{
    let consumed = handler(app, tui, k)?;
    if !consumed && matches!(k.code, KeyCode::Esc) {
        if tui.has_save() {
            tui.current = Screen::Menu;
        } else {
            // No save → wizard is the only way out; Esc opens quit confirm.
            tui.current = Screen::QuitConfirm;
            tui.quit_confirm = Confirm::new("Quit nba3k?");
        }
    }
    Ok(false)
}

fn stub_key(tui: &mut TuiApp, k: KeyEvent) -> Result<bool> {
    if matches!(k.code, KeyCode::Esc) {
        tui.current = Screen::Menu;
    }
    Ok(false)
}

fn menu_key(_app: &mut AppState, tui: &mut TuiApp, k: KeyEvent) -> Result<bool> {
    // No save loaded: only quit / open wizard are valid from the (empty) menu.
    // The shell forces `Screen::NewGame` on entry, so this branch is mostly
    // defensive — if anything ever routes back to `Screen::Menu` without a
    // save, nav keys are no-ops.
    if !tui.has_save() {
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                tui.current = Screen::QuitConfirm;
                tui.quit_confirm = Confirm::new("Quit nba3k?");
            }
            KeyCode::Enter => {
                tui.current = Screen::NewGame;
            }
            _ => {}
        }
        return Ok(false);
    }

    match k.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            tui.current = Screen::QuitConfirm;
            tui.quit_confirm = Confirm::new("Quit nba3k?");
        }
        KeyCode::Up => {
            if tui.menu_selected == 0 {
                tui.menu_selected = MenuItem::ALL.len() - 1;
            } else {
                tui.menu_selected -= 1;
            }
        }
        KeyCode::Down => {
            tui.menu_selected = (tui.menu_selected + 1) % MenuItem::ALL.len();
        }
        KeyCode::Char(c @ '1'..='7') => {
            let idx = (c as u8 - b'1') as usize;
            if idx < MenuItem::ALL.len() {
                tui.menu_selected = idx;
                tui.current = MenuItem::ALL[idx].screen();
            }
        }
        KeyCode::Enter => {
            tui.current = MenuItem::ALL[tui.menu_selected].screen();
        }
        _ => {}
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Shell-level cache (header payroll only — per-screen caches live in screens)
// ---------------------------------------------------------------------------

fn ensure_shell_cache(app: &mut AppState, tui: &mut TuiApp) -> Result<()> {
    if !tui.has_save() {
        return Ok(());
    }
    if tui.payroll.is_none() {
        tui.payroll = Some(app.store()?.team_salary(tui.user_team, tui.season)?);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

const SIDEBAR_WIDTH: u16 = 30;

fn draw(f: &mut Frame, app: &mut AppState, tui: &TuiApp) {
    let area = f.area();
    if area.width < 80 {
        let p = Paragraph::new("Resize terminal to ≥ 80 columns")
            .alignment(Alignment::Center)
            .block(tui.theme.block(""));
        f.render_widget(p, area);
        return;
    }

    // Vertical: body (sidebar | content) over action bar.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    // Horizontal: sidebar (30) | content (rest).
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
        .split(outer[0]);

    draw_sidebar(f, body[0], tui);
    draw_content(f, body[1], app, tui);
    draw_action_bar(f, outer[1], tui);

    if tui.current == Screen::QuitConfirm {
        draw_quit_modal(f, area, tui);
    }
}

fn draw_sidebar(f: &mut Frame, area: Rect, tui: &TuiApp) {
    // Two stacked blocks: season banner (3) + menu (rest).
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    if tui.has_save() {
        draw_sidebar_loaded(f, parts[0], parts[1], tui);
    } else {
        draw_sidebar_empty(f, parts[0], parts[1], tui);
    }
}

fn draw_sidebar_loaded(f: &mut Frame, banner_area: Rect, menu_area: Rect, tui: &TuiApp) {
    // Season banner.
    let payroll = tui
        .payroll
        .map(|c| format!("${:.1}M", c.as_millions_f32()))
        .unwrap_or_else(|| "-".to_string());
    let season_label = format!(" Season {}-{:02} ", tui.season.0 - 1, tui.season.0 % 100);
    let banner_lines = vec![Line::from(vec![
        Span::styled(tui.user_abbrev.clone(), tui.theme.accent_style()),
        Span::styled(format!("  {}", payroll), tui.theme.muted_style()),
    ])];
    let banner = Paragraph::new(banner_lines).block(tui.theme.block(&season_label));
    f.render_widget(banner, banner_area);

    // Menu.
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled("MENU", tui.theme.accent_style())));
    lines.push(Line::from(""));
    for (i, item) in MenuItem::ALL.iter().enumerate() {
        let prefix = if i == tui.menu_selected { "> " } else { "  " };
        let style = if i == tui.menu_selected {
            tui.theme.highlight()
        } else {
            tui.theme.text()
        };
        lines.push(Line::from(Span::styled(
            format!("{}{}. {}", prefix, i + 1, item.label()),
            style,
        )));
    }
    let menu = Paragraph::new(lines).block(tui.theme.block(""));
    f.render_widget(menu, menu_area);
}

fn draw_sidebar_empty(f: &mut Frame, banner_area: Rect, menu_area: Rect, tui: &TuiApp) {
    let banner = Paragraph::new(Line::from(Span::styled(
        "No save loaded",
        tui.theme.muted_style(),
    )))
    .block(tui.theme.block(" nba3k "));
    f.render_widget(banner, banner_area);

    // Hide the 7-item menu entirely — wizard is the only reachable screen.
    let lines = vec![
        Line::from(Span::styled("NEW GAME", tui.theme.accent_style())),
        Line::from(""),
        Line::from(Span::styled(
            "Run the wizard to",
            tui.theme.text(),
        )),
        Line::from(Span::styled("create or load", tui.theme.text())),
        Line::from(Span::styled("a save.", tui.theme.text())),
    ];
    let p = Paragraph::new(lines).block(tui.theme.block(""));
    f.render_widget(p, menu_area);
}

fn draw_content(f: &mut Frame, area: Rect, app: &mut AppState, tui: &TuiApp) {
    match tui.current {
        Screen::Menu => draw_menu_preview(f, area, tui),
        Screen::Home => screens::home::render(f, area, &tui.theme, app, tui),
        Screen::Calendar => screens::calendar::render(f, area, &tui.theme, app, tui),
        Screen::Saves => screens::saves::render(f, area, &tui.theme, app, tui),
        Screen::NewGame => screens::new_game::render(f, area, &tui.theme, app, tui),
        Screen::Roster => screens::render_stub(f, area, &tui.theme, "Roster", "M21"),
        Screen::Rotation => screens::render_stub(f, area, &tui.theme, "Rotation", "M21"),
        Screen::Trades => screens::render_stub(f, area, &tui.theme, "Trades", "M22"),
        Screen::Draft => screens::render_stub(f, area, &tui.theme, "Draft", "M22"),
        Screen::Finance => screens::render_stub(f, area, &tui.theme, "Finance", "M22"),
        Screen::QuitConfirm => {
            // Body still shows the menu preview; the modal is drawn by `draw`.
            draw_menu_preview(f, area, tui);
        }
    }
}

fn draw_menu_preview(f: &mut Frame, area: Rect, tui: &TuiApp) {
    let item = MenuItem::ALL[tui.menu_selected];
    let blurb: &str = match item {
        MenuItem::Home => {
            "Owner mandate · next-game banner · recent results · GM inbox.\nWave-1 worker B fills this."
        }
        MenuItem::Roster => {
            "Player table with stats, traits, contract, role tags. Coming in M21."
        }
        MenuItem::Rotation => "Lineup builder + minutes distribution. Coming in M21.",
        MenuItem::Trades => "Inbox + offer detail + analysis sidebar. Coming in M22.",
        MenuItem::Draft => "Big board + workouts + pick clock. Coming in M22.",
        MenuItem::Finance => "Cap sheet + apron lines + tax projections. Coming in M22.",
        MenuItem::Calendar => {
            "Month grid + sim controls (day/week/month/sim-to-event).\nWave-1 worker C fills this."
        }
    };
    let title = format!(" {} ", item.label());
    let lines = vec![
        Line::from(Span::styled(item.label(), tui.theme.accent_style())),
        Line::from(""),
        Line::from(Span::styled(blurb, tui.theme.text())),
        Line::from(""),
        Line::from(Span::styled(
            "Press Enter to open.",
            tui.theme.muted_style(),
        )),
    ];
    let p = Paragraph::new(lines).block(tui.theme.block(&title));
    f.render_widget(p, area);
}

fn draw_action_bar(f: &mut Frame, area: Rect, tui: &TuiApp) {
    // No-save mode: only the wizard / quit confirm are reachable, so the bar
    // must not advertise Ctrl+S / Esc-back.
    if !tui.has_save() && tui.current != Screen::QuitConfirm {
        let hints: &[(&str, &str)] = &[("Esc", "Quit")];
        let bar = match tui.last_msg.as_deref() {
            Some(s) => ActionBar::new(hints).with_status(s),
            None => ActionBar::new(hints),
        };
        bar.render(f, area, &tui.theme);
        return;
    }

    let hints: &[(&str, &str)] = match tui.current {
        Screen::Menu => &[
            ("↑↓", "Navigate"),
            ("Enter", "Open"),
            ("Ctrl+S", "Saves"),
            ("q", "Quit"),
        ],
        Screen::QuitConfirm => &[("Y", "Yes"), ("N/Esc", "No")],
        Screen::Calendar => &[
            ("Space", "Sim Day"),
            ("W", "Week"),
            ("M", "Month"),
            ("Enter", "Sim to Event"),
            ("A", "Season Advance"),
            ("Esc", "Back"),
        ],
        _ => &[("Esc", "Back"), ("Ctrl+S", "Saves")],
    };

    let bar = match tui.last_msg.as_deref() {
        Some(s) => ActionBar::new(hints).with_status(s),
        None => ActionBar::new(hints),
    };
    bar.render(f, area, &tui.theme);
}

fn draw_quit_modal(f: &mut Frame, area: Rect, tui: &TuiApp) {
    // Center a small (50w × 7h) modal over the body.
    let w = 50.min(area.width.saturating_sub(4));
    let h = 7.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };
    // Wipe background under the modal.
    f.render_widget(Paragraph::new("").style(Style::default()), rect);
    tui.quit_confirm.render(f, rect, &tui.theme);
}

// ---------------------------------------------------------------------------
// IO silencer (used by sim_action paths in screens)
// ---------------------------------------------------------------------------

/// Run a closure with stdout/stderr redirected to /dev/null so prints from
/// inner sim functions don't corrupt the ratatui alt-screen.
pub fn with_silenced_io<F: FnOnce() -> Result<()>>(f: F) -> Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    unsafe {
        let stdout_fd = libc::dup(1);
        let stderr_fd = libc::dup(2);
        if stdout_fd < 0 || stderr_fd < 0 {
            return f();
        }
        let null = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .ok();
        let null_fd = null.as_ref().map(|f| f.as_raw_fd()).unwrap_or(-1);
        if null_fd >= 0 {
            libc::dup2(null_fd, 1);
            libc::dup2(null_fd, 2);
        }
        let result = f();
        libc::dup2(stdout_fd, 1);
        libc::dup2(stderr_fd, 2);
        let _ = OwnedFd::from_raw_fd(stdout_fd);
        let _ = OwnedFd::from_raw_fd(stderr_fd);
        drop(null);
        result
    }
}
