//! M20 TUI shell. Renders the 8-item left-side menu mockup; per-screen
//! content is delegated to `screens::*`. Pre-M20 5-tab dashboard is preserved
//! at `tui --legacy` (see `screens::legacy`).
//!
//! Wave 0 owns: menu nav, terminal lifecycle, event-loop dispatch, theme,
//! widget API. Wave 1 fills the Home / Calendar / Saves / NewGame screens —
//! some types (e.g. `Screen::NewGame`) and helpers are unreferenced until
//! then, so `dead_code` is allowed module-wide for the duration of Wave 0.

#![allow(dead_code)]

pub mod screens;
pub mod widgets;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::path::PathBuf;

use crate::cli::{Command, JsonFlag};
use crate::state::AppState;
use crate::tui::widgets::{ActionBar, Confirm, FormWidget, Theme, WidgetEvent};
use nba3k_core::{t, Cents, Lang, SeasonId, SeasonState, TeamId, T};

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
    Inbox,
    Calendar,
    Settings,
}

impl MenuItem {
    pub const ALL: [MenuItem; 9] = [
        MenuItem::Home,
        MenuItem::Roster,
        MenuItem::Rotation,
        MenuItem::Trades,
        MenuItem::Draft,
        MenuItem::Finance,
        MenuItem::Inbox,
        MenuItem::Calendar,
        MenuItem::Settings,
    ];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            MenuItem::Home => t(lang, T::MenuHome),
            MenuItem::Roster => t(lang, T::MenuRoster),
            MenuItem::Rotation => t(lang, T::MenuRotation),
            MenuItem::Trades => t(lang, T::MenuTrades),
            MenuItem::Draft => t(lang, T::MenuDraft),
            MenuItem::Finance => t(lang, T::MenuFinance),
            MenuItem::Inbox => t(lang, T::MenuInbox),
            MenuItem::Calendar => t(lang, T::MenuCalendar),
            MenuItem::Settings => t(lang, T::LaunchSettings),
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
            MenuItem::Inbox => Screen::Inbox,
            MenuItem::Calendar => Screen::Calendar,
            MenuItem::Settings => Screen::Settings,
        }
    }

    pub fn from_screen(screen: Screen) -> Option<Self> {
        match screen {
            Screen::Home => Some(MenuItem::Home),
            Screen::Roster => Some(MenuItem::Roster),
            Screen::Rotation => Some(MenuItem::Rotation),
            Screen::Trades => Some(MenuItem::Trades),
            Screen::Draft => Some(MenuItem::Draft),
            Screen::Finance => Some(MenuItem::Finance),
            Screen::Inbox => Some(MenuItem::Inbox),
            Screen::Calendar => Some(MenuItem::Calendar),
            Screen::Settings => Some(MenuItem::Settings),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Launch,
    Menu,
    Home,
    Roster,
    Rotation,
    Trades,
    Draft,
    Finance,
    Inbox,
    Calendar,
    Saves,
    Settings,
    NewGame,
    QuitConfirm,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusZone {
    Sidebar,
    Content,
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
    pub user_team_name: String,
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
        let user_team_name = store
            .team_name(user_team)?
            .unwrap_or_else(|| user_abbrev.clone());
        crate::commands::populate_default_starters(store, user_team)?;
        Ok(Some(Self {
            user_team,
            user_abbrev,
            user_team_name,
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
/// `user_team` / `user_abbrev` / `user_team_name` / `season` / `season_state` / `payroll` fields
/// hold safe-default values in that mode — Wave-1 screens that read them
/// must guard via `has_save()` first.
pub struct TuiApp {
    /// Current screen (drives draw + key dispatch).
    pub current: Screen,
    /// Cursor in the menu (0..7). Wraps on ↑/↓.
    pub menu_selected: usize,
    /// True when sidebar navigation is previewing a screen without focusing it.
    pub preview_mode: bool,
    /// Active shell region. Synced from `current` + `preview_mode` before draw.
    pub focus: FocusZone,
    /// Theme palette (DEFAULT or TV).
    pub theme: Theme,
    /// TUI chrome language. Player names and team abbreviations remain data.
    pub lang: Lang,
    /// Screen to return to when QuitConfirm is cancelled.
    quit_return: Screen,

    /// Save-derived snapshot. `None` until a save is loaded. Wave-1 screens
    /// can use either this or the mirrored fields below.
    pub save_ctx: Option<SaveCtx>,

    // ---- Mirror of `save_ctx` fields (zero/empty when no save). ----
    /// User team id. `TeamId(0)` when no save loaded.
    pub user_team: TeamId,
    /// User team abbreviation. Empty string when no save loaded.
    pub user_abbrev: String,
    /// User team full name. Empty string when no save loaded.
    pub user_team_name: String,
    /// Current season. `SeasonId(0)` when no save loaded.
    pub season: SeasonId,
    /// Current season state. Default-zeroed when no save loaded.
    pub season_state: SeasonState,
    /// Lazy cache for header payroll. Cleared via `invalidate_caches`.
    pub payroll: Option<Cents>,

    /// Last action message (sim result, error, role change, etc.). Shown
    /// left-aligned in the action bar.
    pub last_msg: Option<String>,

    /// Global context help overlay, toggled by `?`.
    pub help_open: bool,

    /// Confirm widget shown when QuitConfirm is current.
    quit_confirm: Confirm,
}

impl TuiApp {
    fn new(theme: Theme, save_ctx: Option<SaveCtx>, lang: Lang) -> Self {
        let mut app = Self {
            current: Screen::Launch,
            menu_selected: 0,
            preview_mode: false,
            focus: FocusZone::Content,
            theme,
            lang,
            quit_return: Screen::Launch,
            save_ctx: None,
            user_team: TeamId(0),
            user_abbrev: String::new(),
            user_team_name: String::new(),
            season: SeasonId(0),
            season_state: empty_season_state(),
            payroll: None,
            last_msg: None,
            help_open: false,
            quit_confirm: Confirm::new(t(lang, T::ModalQuitTitle)),
        };
        app.set_save_ctx(save_ctx);
        app
    }

    /// True iff a save is loaded and has `season_state`. Wave-1 screens use
    /// this to gate "needs save" rendering.
    pub fn has_save(&self) -> bool {
        self.save_ctx.is_some()
    }

    fn sync_focus(&mut self) {
        self.focus = self.derived_focus();
    }

    fn derived_focus(&self) -> FocusZone {
        if self.current == Screen::Menu || (self.preview_mode && self.has_save()) {
            FocusZone::Sidebar
        } else {
            FocusZone::Content
        }
    }

    /// Replace `save_ctx` and re-mirror its fields onto self. Internal helper
    /// used by `new`, `refresh_save_ctx`, and `switch_save`.
    fn set_save_ctx(&mut self, ctx: Option<SaveCtx>) {
        self.help_open = false;
        self.preview_mode = false;
        match ctx {
            Some(c) => {
                self.user_team = c.user_team;
                self.user_abbrev = c.user_abbrev.clone();
                self.user_team_name = c.user_team_name.clone();
                self.season = c.season;
                self.season_state = c.season_state.clone();
                self.payroll = None;
                self.save_ctx = Some(c);
            }
            None => {
                self.user_team = TeamId(0);
                self.user_abbrev = String::new();
                self.user_team_name = String::new();
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
        self.help_open = false;
        app.open_path(new_path)?;
        self.refresh_save_ctx(app)?;
        self.load_lang(app);
        Ok(())
    }

    fn open_quit_confirm(&mut self, return_to: Screen) {
        self.help_open = false;
        self.preview_mode = false;
        self.quit_return = return_to;
        self.current = Screen::QuitConfirm;
        self.quit_confirm = Confirm::new(t(self.lang, T::ModalQuitTitle));
    }

    pub fn apply_language(&mut self, lang: Lang) {
        self.lang = lang;
        self.quit_confirm = Confirm::new(t(lang, T::ModalQuitTitle));
    }

    fn load_lang(&mut self, app: &mut AppState) {
        self.apply_language(read_lang(app));
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

/// New M20 TUI entry — 8-menu shell. The shell tolerates a no-save launch:
/// when no `--save` is passed (or the save has no `season_state` yet), we
/// enter `Screen::NewGame` and the wizard becomes the only reachable screen
/// until it creates / loads a save. The `--legacy` flag is wired separately
/// through `run_legacy`.
pub fn run(app: &mut AppState, tv: bool) -> Result<()> {
    let theme = if tv { Theme::TV } else { Theme::DEFAULT };

    // Best-effort load: if the store can't open or the save has no
    // season_state, fall through with `save_ctx = None` and let the wizard
    // take over instead of aborting.
    let (save_ctx, lang) = match app.store() {
        Ok(_) => {
            let lang = read_lang(app);
            let save_ctx = SaveCtx::load(app).unwrap_or(None);
            (save_ctx, lang)
        }
        Err(_) => (None, read_config_lang()),
    };

    let mut tui = TuiApp::new(theme, save_ctx, lang);

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

fn read_lang(app: &mut AppState) -> Lang {
    match app.store() {
        Ok(store) => store
            .read_setting("language")
            .ok()
            .flatten()
            .or_else(crate::config::read_lang)
            .and_then(|value| Lang::from_setting(&value))
            .unwrap_or(Lang::En),
        Err(_) => read_config_lang(),
    }
}

fn read_config_lang() -> Lang {
    crate::config::read_lang()
        .and_then(|value| Lang::from_setting(&value))
        .unwrap_or(Lang::En)
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
        tui.sync_focus();
        terminal.draw(|f| draw(f, app, tui))?;

        let Event::Key(k) = event::read()? else {
            continue;
        };
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
    if tui.help_open {
        match k.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                tui.help_open = false;
            }
            _ => {}
        }
        return Ok(false);
    }

    // QuitConfirm modal swallows everything until resolved.
    if tui.current == Screen::QuitConfirm {
        match tui.quit_confirm.handle_key(k) {
            WidgetEvent::Submitted => return Ok(true),
            WidgetEvent::Cancelled => {
                tui.current = tui.quit_return;
                tui.preview_mode = false;
            }
            _ => {}
        }
        return Ok(false);
    }

    // Global shortcut: Ctrl+S = saves overlay. Works even with no save loaded
    // so the user can pick an existing save from the wizard. Saves overlay's
    // own logic handles the no-save case (Esc bounces to NewGame).
    if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('s')) {
        tui.help_open = false;
        tui.preview_mode = false;
        tui.current = Screen::Saves;
        return Ok(false);
    }

    if tui.preview_mode && tui.has_save() {
        return preview_key(app, tui, k);
    }

    match tui.current {
        Screen::Launch => launch_key(app, tui, k),
        Screen::Menu => menu_key(app, tui, k),
        Screen::Home => inner_screen_key(app, tui, k, screens::home::handle_key),
        Screen::Calendar => inner_screen_key(app, tui, k, screens::calendar::handle_key),
        Screen::Saves => inner_screen_key(app, tui, k, screens::saves::handle_key),
        Screen::Settings => inner_screen_key(app, tui, k, screens::settings::handle_key),
        Screen::NewGame => inner_screen_key(app, tui, k, screens::new_game::handle_key),
        Screen::Roster => inner_screen_key(app, tui, k, screens::roster::handle_key),
        Screen::Rotation => inner_screen_key(app, tui, k, screens::rotation::handle_key),
        Screen::Trades => inner_screen_key(app, tui, k, screens::trades::handle_key),
        Screen::Draft => inner_screen_key(app, tui, k, screens::draft::handle_key),
        Screen::Finance => inner_screen_key(app, tui, k, screens::finance::handle_key),
        Screen::Inbox => inner_screen_key(app, tui, k, screens::inbox::handle_key),
        Screen::QuitConfirm => Ok(false), // unreachable
    }
}

fn preview_key(app: &mut AppState, tui: &mut TuiApp, k: KeyEvent) -> Result<bool> {
    if handle_global_sim_key(app, tui, k)? {
        return Ok(false);
    }
    match k.code {
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Char('1'..='9')
        | KeyCode::Char('?')
        | KeyCode::Char('q')
        | KeyCode::Esc => menu_key(app, tui, k),
        KeyCode::Enter | KeyCode::Tab => {
            tui.help_open = false;
            tui.preview_mode = false;
            tui.current = selected_menu_screen(tui);
            sync_settings_cursor(tui);
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn launch_key(app: &mut AppState, tui: &mut TuiApp, k: KeyEvent) -> Result<bool> {
    match screens::launch::handle_key(app, tui, k)? {
        screens::launch::LaunchAction::Consumed | screens::launch::LaunchAction::None => {}
    }
    if tui.current == Screen::QuitConfirm {
        tui.quit_return = Screen::Launch;
        tui.quit_confirm = Confirm::new(t(tui.lang, T::ModalQuitTitle));
    }
    Ok(false)
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
    if consumed {
        return Ok(false);
    }
    if handle_global_sim_key(app, tui, k)? {
        return Ok(false);
    }
    if matches!(k.code, KeyCode::Char('?')) {
        tui.help_open = true;
    } else if matches!(k.code, KeyCode::Esc) {
        tui.help_open = false;
        if tui.has_save() {
            if let Some(item) = MenuItem::from_screen(tui.current) {
                tui.menu_selected = MenuItem::ALL
                    .iter()
                    .position(|candidate| *candidate == item)
                    .unwrap_or(tui.menu_selected);
            }
            tui.current = selected_menu_screen(tui);
            sync_settings_cursor(tui);
            tui.preview_mode = true;
        } else if matches!(tui.current, Screen::Saves) {
            // No save AND user opened saves overlay from launch/new-game:
            // bounce back to the explicit launch screen rather than quitting.
            tui.current = Screen::Launch;
            tui.preview_mode = false;
        } else if matches!(tui.current, Screen::Settings) {
            // No save AND user opened settings from launch: return to launch.
            tui.current = Screen::Launch;
            tui.preview_mode = false;
        } else {
            // No save → wizard is the only way out; Esc opens quit confirm.
            tui.open_quit_confirm(tui.current);
        }
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
            KeyCode::Char('?') => {
                tui.help_open = true;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                tui.open_quit_confirm(tui.current);
            }
            KeyCode::Enter => {
                tui.help_open = false;
                tui.preview_mode = false;
                tui.current = Screen::NewGame;
            }
            _ => {}
        }
        return Ok(false);
    }

    if handle_global_sim_key(_app, tui, k)? {
        return Ok(false);
    }

    match k.code {
        KeyCode::Char('?') => {
            tui.help_open = true;
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            tui.open_quit_confirm(tui.current);
        }
        KeyCode::Up => {
            if tui.menu_selected == 0 {
                tui.menu_selected = MenuItem::ALL.len() - 1;
            } else {
                tui.menu_selected -= 1;
            }
            tui.current = selected_menu_screen(tui);
            sync_settings_cursor(tui);
            tui.preview_mode = true;
        }
        KeyCode::Down => {
            tui.menu_selected = (tui.menu_selected + 1) % MenuItem::ALL.len();
            tui.current = selected_menu_screen(tui);
            sync_settings_cursor(tui);
            tui.preview_mode = true;
        }
        KeyCode::Char(c @ '1'..='9') => {
            let idx = (c as u8 - b'1') as usize;
            if idx < MenuItem::ALL.len() {
                tui.help_open = false;
                tui.menu_selected = idx;
                tui.current = selected_menu_screen(tui);
                sync_settings_cursor(tui);
                tui.preview_mode = true;
            }
        }
        KeyCode::Enter | KeyCode::Tab => {
            tui.help_open = false;
            tui.preview_mode = false;
            tui.current = selected_menu_screen(tui);
            sync_settings_cursor(tui);
        }
        _ => {}
    }
    Ok(false)
}

fn handle_global_sim_key(app: &mut AppState, tui: &mut TuiApp, k: KeyEvent) -> Result<bool> {
    if !tui.has_save() || !k.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(false);
    }
    let KeyCode::Char(c) = k.code else {
        return Ok(false);
    };
    let (cmd, label) = match c.to_ascii_lowercase() {
        'd' => (
            Command::SimDay { count: Some(1) },
            t(tui.lang, T::SimDay).to_string(),
        ),
        'w' => (
            Command::SimWeek { no_pause: true },
            t(tui.lang, T::SimWeek).to_string(),
        ),
        'n' => (
            Command::SimMonth { no_pause: true },
            t(tui.lang, T::SimMonth).to_string(),
        ),
        't' => (
            Command::SimTo {
                phase: "trade-deadline".to_string(),
            },
            t(tui.lang, T::SimTradeDeadline).to_string(),
        ),
        'a' => (
            Command::SeasonAdvance(JsonFlag { json: false }),
            t(tui.lang, T::SimSeasonAdvance).to_string(),
        ),
        _ => return Ok(false),
    };
    run_global_sim(app, tui, cmd, &label)?;
    Ok(true)
}

fn run_global_sim(app: &mut AppState, tui: &mut TuiApp, cmd: Command, label: &str) -> Result<()> {
    let pre_day = tui.season_state.day;
    let result = with_silenced_io(|| crate::commands::dispatch(app, cmd));
    match result {
        Ok(()) => {
            tui.refresh_season_state(app)?;
            invalidate_all_screens(tui);
            let post_day = tui.season_state.day;
            let delta = post_day.saturating_sub(pre_day);
            tui.last_msg = Some(format!(
                "{}: +{}d ({} {})",
                label,
                delta,
                t(tui.lang, T::CalendarDayOf),
                post_day
            ));
        }
        Err(e) => {
            tui.last_msg = Some(format!("{}: {}", t(tui.lang, T::CommonError), e));
        }
    }
    Ok(())
}

pub(crate) fn invalidate_all_screens(tui: &mut TuiApp) {
    tui.invalidate_caches();
    screens::home::invalidate();
    screens::roster::invalidate();
    screens::rotation::invalidate();
    screens::trades::invalidate();
    screens::draft::invalidate();
    screens::finance::invalidate();
    screens::calendar::invalidate();
    screens::inbox::invalidate();
}

fn selected_menu_screen(tui: &TuiApp) -> Screen {
    MenuItem::ALL[tui.menu_selected].screen()
}

fn sync_settings_cursor(tui: &TuiApp) {
    if tui.current == Screen::Settings {
        screens::settings::reset(settings_choice(tui.lang));
    }
}

pub(crate) fn settings_choice(lang: Lang) -> screens::settings::LanguageChoice {
    match lang {
        Lang::En => screens::settings::LanguageChoice::En,
        Lang::Zh => screens::settings::LanguageChoice::Zh,
    }
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

    let show_banner = sim_banner_visible(tui);
    let outer = if show_banner {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(area)
    };
    let (banner_area, body_area, action_area) = if show_banner {
        (Some(outer[0]), outer[1], outer[2])
    } else {
        (None, outer[0], outer[1])
    };

    // Horizontal: sidebar (30) | content (rest).
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
        .split(body_area);

    if let Some(banner_area) = banner_area {
        draw_sim_banner(f, banner_area, tui);
    }
    draw_sidebar(f, body[0], tui);
    draw_content(f, body[1], app, tui);
    draw_action_bar(f, action_area, tui);

    if tui.current == Screen::QuitConfirm {
        draw_quit_modal(f, area, tui);
    }
    if tui.help_open {
        draw_help_modal(f, area, tui);
    }
}

fn sim_banner_visible(tui: &TuiApp) -> bool {
    tui.has_save()
        && !matches!(
            tui.current,
            Screen::Launch
                | Screen::NewGame
                | Screen::Saves
                | Screen::Settings
                | Screen::QuitConfirm
        )
}

fn draw_sim_banner(f: &mut Frame, area: Rect, tui: &TuiApp) {
    let season_label = format!("{}-{:02}", tui.season.0 - 1, tui.season.0 % 100);
    let status = format!(
        "Season {}  ·  Day {}  ·  {:?}",
        season_label, tui.season_state.day, tui.season_state.phase
    );
    let buttons = Line::from(vec![
        Span::styled("[D] ", tui.theme.accent_style()),
        Span::styled(t(tui.lang, T::SimDay), tui.theme.text()),
        Span::raw("   "),
        Span::styled("[W] ", tui.theme.accent_style()),
        Span::styled(t(tui.lang, T::SimWeek), tui.theme.text()),
        Span::raw("   "),
        Span::styled("[N] ", tui.theme.accent_style()),
        Span::styled(t(tui.lang, T::SimMonth), tui.theme.text()),
        Span::raw("   "),
        Span::styled("[T] ", tui.theme.accent_style()),
        Span::styled(t(tui.lang, T::SimTradeDeadline), tui.theme.text()),
        Span::raw("   "),
        Span::styled("[A] ", tui.theme.accent_style()),
        Span::styled(t(tui.lang, T::SimSeasonAdvance), tui.theme.text()),
    ]);
    let lines = vec![
        Line::from(Span::styled(status, tui.theme.accent_style())),
        buttons,
        Line::from(Span::styled("Ctrl+D/W/N/T/A", tui.theme.muted_style())),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_sidebar(f: &mut Frame, area: Rect, tui: &TuiApp) {
    let block = tui.theme.focus_block("", tui.focus == FocusZone::Sidebar);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Two stacked blocks: season banner (3) + menu (rest).
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);

    if tui.current == Screen::Launch {
        draw_sidebar_launch(f, parts[0], parts[1], tui);
    } else if tui.has_save() {
        draw_sidebar_loaded(f, parts[0], parts[1], tui);
    } else {
        draw_sidebar_empty(f, parts[0], parts[1], tui);
    }
}

fn draw_sidebar_launch(f: &mut Frame, banner_area: Rect, menu_area: Rect, tui: &TuiApp) {
    let banner = Paragraph::new(Line::from(Span::styled(
        t(tui.lang, T::AppName),
        tui.theme.accent_style(),
    )))
    .block(tui.theme.block(" nba3k "));
    f.render_widget(banner, banner_area);

    let lines = vec![
        Line::from(Span::styled(
            t(tui.lang, T::AppName),
            tui.theme.accent_style(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            t(tui.lang, T::LaunchNewGame),
            tui.theme.text(),
        )),
        Line::from(Span::styled(
            t(tui.lang, T::LaunchLoadGame),
            tui.theme.text(),
        )),
        Line::from(Span::styled(
            t(tui.lang, T::LaunchSettings),
            tui.theme.text(),
        )),
    ];
    let p = Paragraph::new(lines).block(tui.theme.block(""));
    f.render_widget(p, menu_area);
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
            format!("{}{}. {}", prefix, i + 1, item.label(tui.lang)),
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

    // Hide the 8-item menu entirely — wizard is the only reachable screen.
    let lines = vec![
        Line::from(Span::styled("NEW GAME", tui.theme.accent_style())),
        Line::from(""),
        Line::from(Span::styled("Run the wizard to", tui.theme.text())),
        Line::from(Span::styled("create or load", tui.theme.text())),
        Line::from(Span::styled("a save.", tui.theme.text())),
    ];
    let p = Paragraph::new(lines).block(tui.theme.block(""));
    f.render_widget(p, menu_area);
}

fn draw_content(f: &mut Frame, area: Rect, app: &mut AppState, tui: &TuiApp) {
    let title = content_title(tui);
    let block = tui
        .theme
        .focus_block(&title, tui.focus == FocusZone::Content);
    let inner = block.inner(area);
    f.render_widget(block, area);

    match tui.current {
        Screen::Launch => screens::launch::render(f, inner, &tui.theme, app, tui),
        Screen::Menu => draw_menu_preview(f, inner, tui),
        Screen::Home => screens::home::render(f, inner, &tui.theme, app, tui),
        Screen::Calendar => screens::calendar::render(f, inner, &tui.theme, app, tui),
        Screen::Saves => screens::saves::render(f, inner, &tui.theme, app, tui),
        Screen::Settings => {
            screens::settings::render(f, inner, &tui.theme, tui.lang, settings_choice(tui.lang))
        }
        Screen::NewGame => screens::new_game::render(f, inner, &tui.theme, app, tui),
        Screen::Roster => screens::roster::render(f, inner, &tui.theme, app, tui),
        Screen::Rotation => screens::rotation::render(f, inner, &tui.theme, app, tui),
        Screen::Trades => screens::trades::render(f, inner, &tui.theme, app, tui),
        Screen::Draft => screens::draft::render(f, inner, &tui.theme, app, tui),
        Screen::Finance => screens::finance::render(f, inner, &tui.theme, app, tui),
        Screen::Inbox => screens::inbox::render(f, inner, &tui.theme, app, tui),
        Screen::QuitConfirm => {
            // Body still shows the menu preview; the modal is drawn by `draw`.
            draw_menu_preview(f, inner, tui);
        }
    }
}

fn content_title(tui: &TuiApp) -> String {
    match tui.current {
        Screen::Launch => t(tui.lang, T::AppName).to_string(),
        Screen::Menu => MenuItem::ALL[tui.menu_selected].label(tui.lang).to_string(),
        Screen::Home => t(tui.lang, T::HomeTitle).to_string(),
        Screen::Roster => t(tui.lang, T::RosterTitle).to_string(),
        Screen::Rotation => t(tui.lang, T::RotationTitle).to_string(),
        Screen::Trades => t(tui.lang, T::TradesTitle).to_string(),
        Screen::Draft => t(tui.lang, T::DraftTitle).to_string(),
        Screen::Finance => t(tui.lang, T::FinanceTitle).to_string(),
        Screen::Inbox => t(tui.lang, T::InboxTitle).to_string(),
        Screen::Calendar => t(tui.lang, T::CalendarTitle).to_string(),
        Screen::Saves => t(tui.lang, T::SavesTitle).to_string(),
        Screen::Settings => t(tui.lang, T::SettingsTitle).to_string(),
        Screen::NewGame => t(tui.lang, T::NewGameTitle).to_string(),
        Screen::QuitConfirm => t(tui.lang, T::ModalQuitTitle).to_string(),
    }
}

fn draw_menu_preview(f: &mut Frame, area: Rect, tui: &TuiApp) {
    let item = MenuItem::ALL[tui.menu_selected];
    let blurb: &str = match item {
        MenuItem::Home => "Record, standings, leaders, team stats, finances, and lineup.",
        MenuItem::Roster => {
            "Roster table, player details, training, extensions, cuts, roles, and free agents."
        }
        MenuItem::Rotation => "Starting five assignment with auto bench and minutes.",
        MenuItem::Trades => "Incoming offers, proposal chains, trade builder, and rumors.",
        MenuItem::Draft => "Prospect board, scouting, draft order, and pick controls.",
        MenuItem::Finance => "Payroll, cap/tax/apron lines, contracts, and extensions.",
        MenuItem::Inbox => "GM messages, trade demands, and league news.",
        MenuItem::Calendar => "Month grid, standings, playoffs, awards, All-Star, and Cup.",
        MenuItem::Settings => "Language and shell preferences.",
    };
    let title = format!(" {} ", item.label(tui.lang));
    let lines = vec![
        Line::from(Span::styled(item.label(tui.lang), tui.theme.accent_style())),
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
    // No-save mode: wizard + saves overlay are both reachable. Show Ctrl+S
    // so user knows they can load an existing save instead of new-game.
    if !tui.has_save() && tui.current != Screen::QuitConfirm {
        let hints: &[(&str, &str)] = match tui.current {
            Screen::Launch => &[("↑↓", "Navigate"), ("Enter", "Select"), ("Esc", "Quit")],
            Screen::Saves => &[("↑↓", "Navigate"), ("l", "Load"), ("Esc", "Back")],
            Screen::Settings => &[("↑↓", "Move"), ("Enter", "Apply"), ("Esc", "Back")],
            _ => &[("Ctrl+S", "Load Save"), ("?", "Help"), ("Esc", "Quit")],
        };
        let bar = match tui.last_msg.as_deref() {
            Some(s) => ActionBar::new(hints).with_status(s),
            None => ActionBar::new(hints),
        };
        bar.render(f, area, &tui.theme);
        return;
    }

    let hints: &[(&str, &str)] = if tui.preview_mode {
        &[
            ("↑↓", "Navigate"),
            ("1-9", "Jump"),
            ("Enter/Tab", "Focus"),
            ("Ctrl+S", "Saves"),
            ("?", "Help"),
            ("q", "Quit"),
        ]
    } else {
        match tui.current {
            Screen::Launch => &[("↑↓", "Navigate"), ("Enter", "Select"), ("Esc", "Quit")],
            Screen::Menu => &[
                ("↑↓", "Navigate"),
                ("Enter", "Open"),
                ("Ctrl+S", "Saves"),
                ("?", "Help"),
                ("q", "Quit"),
            ],
            Screen::QuitConfirm => &[("Y", "Yes"), ("N/Esc", "No")],
            Screen::Settings => &[("↑↓", "Move"), ("Enter", "Apply"), ("Esc", "Menu")],
            Screen::Calendar => &[
                ("←↑↓→", "Move date"),
                ("[ / ]", "Month"),
                ("Tab", "Sub-page"),
                ("?", "Help"),
                ("Esc", "Back"),
            ],
            Screen::Roster => &[
                ("Tab", "Roster/FA"),
                ("Enter", "Detail"),
                ("t/e/x/R", "Actions"),
                ("?", "Help"),
                ("Esc", "Back"),
            ],
            Screen::Rotation => &[
                ("Enter", "Pick"),
                ("c/C", "Clear"),
                ("?", "Help"),
                ("Esc", "Back"),
            ],
            Screen::Trades => &[
                ("Tab", "Tabs"),
                ("Enter", "Detail/Submit"),
                ("a/r/c", "Respond"),
                ("?", "Help"),
                ("Esc", "Back"),
            ],
            Screen::Draft => &[
                ("Tab", "Board/Order"),
                ("s", "Scout"),
                ("Enter", "Pick"),
                ("A", "Auto"),
                ("?", "Help"),
                ("Esc", "Back"),
            ],
            Screen::Finance => &[
                ("↑↓", "Navigate"),
                ("t/y/n", "Sort"),
                ("e", "Extend"),
                ("?", "Help"),
                ("Esc", "Back"),
            ],
            _ => &[("Esc", "Back"), ("Ctrl+S", "Saves"), ("?", "Help")],
        }
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
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    // Wipe background under the modal.
    f.render_widget(Clear, rect);
    tui.quit_confirm.render(f, rect, &tui.theme);
}

fn draw_help_modal(f: &mut Frame, area: Rect, tui: &TuiApp) {
    let w = 68.min(area.width.saturating_sub(4));
    let h = 20.min(area.height.saturating_sub(4));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("{} keys", screen_label(help_screen(tui))),
        tui.theme.accent_style(),
    )));
    lines.push(Line::from(""));
    for (key, label) in help_key_rows(help_screen(tui)) {
        lines.push(Line::from(vec![
            Span::styled(format!("{:<14}", key), tui.theme.accent_style()),
            Span::styled(*label, tui.theme.text()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Esc, q, or ? closes help.",
        tui.theme.muted_style(),
    )));

    let p = Paragraph::new(lines).block(tui.theme.block(" Help "));
    f.render_widget(p, rect);
}

fn help_screen(tui: &TuiApp) -> Screen {
    if tui.preview_mode {
        Screen::Menu
    } else {
        tui.current
    }
}

fn screen_label(screen: Screen) -> &'static str {
    match screen {
        Screen::Launch => "Launch",
        Screen::Menu => "Menu",
        Screen::Home => "Home",
        Screen::Roster => "Roster",
        Screen::Rotation => "Rotation",
        Screen::Trades => "Trades",
        Screen::Draft => "Draft",
        Screen::Finance => "Finance",
        Screen::Inbox => "Inbox",
        Screen::Calendar => "Calendar",
        Screen::Saves => "Saves",
        Screen::Settings => "Settings",
        Screen::NewGame => "New Game",
        Screen::QuitConfirm => "Quit",
    }
}

fn help_key_rows(screen: Screen) -> &'static [(&'static str, &'static str)] {
    match screen {
        Screen::Launch => &[
            ("↑ / ↓", "Move through launch rows"),
            ("Enter", "Select launch row"),
            ("q / Esc", "Quit"),
        ],
        Screen::Menu => &[
            ("↑ / ↓", "Move through the 9 menu items"),
            ("1 - 9", "Jump directly to a menu item"),
            ("Enter", "Open selected screen"),
            ("Ctrl+S", "Open save manager"),
            ("q / Esc", "Quit"),
        ],
        Screen::Home => &[("Esc", "Back to menu"), ("Ctrl+S", "Open save manager")],
        Screen::Roster => &[
            ("Tab / 1 / 2", "Switch My Roster and Free Agents"),
            ("↑ / ↓", "Move selected row"),
            ("o / p / a / s", "Sort roster"),
            ("Enter", "Open player detail"),
            ("t / e / x / R", "Train, extend, cut, or set role"),
        ],
        Screen::Rotation => &[
            ("↑ / ↓", "Move starter slot"),
            ("Enter", "Pick player for selected slot"),
            ("c", "Clear selected slot"),
            ("C", "Clear all starters"),
        ],
        Screen::Trades => &[
            ("Tab / 1-4", "Switch Inbox, Proposals, Builder, Rumors"),
            ("↑ / ↓", "Move selected row"),
            ("Enter", "Open detail or submit builder"),
            ("a / r / c", "Accept, reject, or counter an open offer"),
            ("Space", "Toggle selected player in builder"),
        ],
        Screen::Draft => &[
            ("Tab / 1 / 2", "Switch Board and Order"),
            ("↑ / ↓", "Move selected row"),
            ("s", "Scout selected prospect"),
            ("Enter", "Pick selected prospect when active"),
            ("A", "Auto-pick draft"),
        ],
        Screen::Finance => &[
            ("↑ / ↓", "Move selected contract"),
            ("t / y / n", "Sort by total, years, or name"),
            ("e", "Offer extension"),
        ],
        Screen::Inbox => &[
            ("Tab / 1-3", "Switch Messages, Trade Demands, News"),
            ("↑ / ↓", "Move selected row"),
            ("Enter", "Open message detail"),
            ("Esc", "Back to menu"),
        ],
        Screen::Calendar => &[
            ("← / ↑ / ↓ / →", "Move selected date"),
            ("[ / ]", "Previous or next month"),
            ("Tab", "Switch Calendar sub-page"),
            ("1-6", "Jump to Calendar sub-page"),
            ("Esc", "Back to menu"),
        ],
        Screen::Saves => &[
            ("↑ / ↓", "Move selected save"),
            ("l / n / d / e", "Load, new, delete, or export"),
            ("Esc", "Back"),
        ],
        Screen::Settings => &[
            ("↑ / ↓", "Move selected language"),
            ("Enter", "Apply language"),
            ("Esc", "Back to menu"),
        ],
        Screen::NewGame => &[
            ("↑ / ↓", "Move selection"),
            ("Enter", "Continue or confirm"),
            ("Esc", "Quit when no save is loaded"),
        ],
        Screen::QuitConfirm => &[("Y", "Quit"), ("N / Esc", "Cancel")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_key_opens_only_after_screen_declines_it() {
        let key = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE);
        let mut app = AppState::new(None, false);
        let mut tui = TuiApp::new(Theme::DEFAULT, None, Lang::En);

        inner_screen_key(&mut app, &mut tui, key, |_app, _tui, _key| Ok(true)).unwrap();
        assert!(!tui.help_open);

        inner_screen_key(&mut app, &mut tui, key, |_app, _tui, _key| Ok(false)).unwrap();
        assert!(tui.help_open);
    }

    #[test]
    fn menu_nav_enters_preview_and_enter_or_tab_focuses() {
        let mut app = AppState::new(None, false);
        let mut tui = TuiApp::new(Theme::DEFAULT, Some(test_save_ctx()), Lang::En);

        menu_key(
            &mut app,
            &mut tui,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(tui.menu_selected, 1);
        assert_eq!(tui.current, Screen::Roster);
        assert!(tui.preview_mode);

        preview_key(
            &mut app,
            &mut tui,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(tui.current, Screen::Roster);
        assert!(!tui.preview_mode);

        menu_key(
            &mut app,
            &mut tui,
            KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(tui.menu_selected, 3);
        assert_eq!(tui.current, Screen::Trades);
        assert!(tui.preview_mode);

        preview_key(
            &mut app,
            &mut tui,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(tui.current, Screen::Trades);
        assert!(!tui.preview_mode);
    }

    #[test]
    fn focused_screen_escape_returns_to_preview_mode() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let mut app = AppState::new(None, false);
        let mut tui = TuiApp::new(Theme::DEFAULT, Some(test_save_ctx()), Lang::En);
        tui.current = Screen::Finance;
        tui.menu_selected = 0;

        inner_screen_key(&mut app, &mut tui, key, |_app, _tui, _key| Ok(false)).unwrap();

        assert_eq!(tui.current, Screen::Finance);
        assert_eq!(tui.menu_selected, 5);
        assert!(tui.preview_mode);
    }

    #[test]
    fn focus_zone_matches_sidebar_preview_and_content_rules() {
        let mut tui = TuiApp::new(Theme::DEFAULT, Some(test_save_ctx()), Lang::En);

        tui.current = Screen::Menu;
        tui.preview_mode = false;
        assert_eq!(tui.derived_focus(), FocusZone::Sidebar);

        tui.current = Screen::Calendar;
        tui.preview_mode = true;
        assert_eq!(tui.derived_focus(), FocusZone::Sidebar);

        tui.preview_mode = false;
        assert_eq!(tui.derived_focus(), FocusZone::Content);

        for screen in [
            Screen::Launch,
            Screen::NewGame,
            Screen::Saves,
            Screen::Settings,
            Screen::QuitConfirm,
        ] {
            tui.current = screen;
            tui.preview_mode = false;
            assert_eq!(tui.derived_focus(), FocusZone::Content);
        }
    }

    #[test]
    fn sync_focus_updates_public_focus_field() {
        let mut tui = TuiApp::new(Theme::DEFAULT, Some(test_save_ctx()), Lang::En);
        tui.current = Screen::Roster;
        tui.preview_mode = true;
        tui.sync_focus();
        assert_eq!(tui.focus, FocusZone::Sidebar);

        tui.preview_mode = false;
        tui.sync_focus();
        assert_eq!(tui.focus, FocusZone::Content);
    }

    fn test_save_ctx() -> SaveCtx {
        SaveCtx {
            user_team: TeamId(1),
            user_abbrev: "BOS".to_string(),
            user_team_name: "Boston Celtics".to_string(),
            season: SeasonId(2026),
            season_state: empty_season_state(),
        }
    }
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
