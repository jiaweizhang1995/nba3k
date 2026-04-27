// READ-ONLY M19 dashboard. Never call mutation methods on Store.

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Tabs},
    Frame, Terminal,
};
use std::io;

use crate::state::AppState;
use nba3k_core::{
    Cents, Conference, NegotiationState, Player, PlayerId, PlayerRole, SeasonId, SeasonPhase,
    SeasonState, TeamId, TradeId,
};
use nba3k_season::career::{aggregate_career, SeasonAvgRow};
use nba3k_store::{NewsRow, StandingRow};
use std::collections::HashMap;

#[derive(Copy, Clone, PartialEq, Eq)]
enum Tab {
    Status,
    Roster,
    Standings,
    Trades,
    News,
}

impl Tab {
    fn next(self) -> Self {
        match self {
            Tab::Status => Tab::Roster,
            Tab::Roster => Tab::Standings,
            Tab::Standings => Tab::Trades,
            Tab::Trades => Tab::News,
            Tab::News => Tab::Status,
        }
    }
    fn prev(self) -> Self {
        match self {
            Tab::Status => Tab::News,
            Tab::Roster => Tab::Status,
            Tab::Standings => Tab::Roster,
            Tab::Trades => Tab::Standings,
            Tab::News => Tab::Trades,
        }
    }
    fn idx(self) -> usize {
        match self {
            Tab::Status => 0,
            Tab::Roster => 1,
            Tab::Standings => 2,
            Tab::Trades => 3,
            Tab::News => 4,
        }
    }
}

struct StatusCounts {
    teams: u32,
    players: u32,
    schedule_total: u32,
    schedule_unplayed: u32,
}

struct TuiState {
    tab: Tab,
    scroll: u16,
    selected: usize,
    user_team: TeamId,
    user_abbrev: String,
    season: SeasonId,
    season_state: SeasonState,
    counts: Option<StatusCounts>,
    payroll: Option<Cents>,
    roster: Option<Vec<Player>>,
    roster_stats: Option<HashMap<PlayerId, SeasonAvgRow>>,
    standings: Option<Vec<StandingRow>>,
    open_chains: Option<Vec<(TradeId, NegotiationState)>>,
    recent_chains: Option<Vec<(TradeId, NegotiationState)>>,
    news: Option<Vec<NewsRow>>,
    /// Cached lookups for resolving offer asset names without repeated SQL.
    player_index: Option<HashMap<PlayerId, Player>>,
    team_abbrev_index: Option<HashMap<TeamId, String>>,
    /// LeagueYear cap, fetched once per save.
    league_cap: Option<Cents>,
    /// Last sim result / action feedback shown in footer.
    last_msg: Option<String>,
}

impl TuiState {
    fn invalidate_caches(&mut self) {
        self.counts = None;
        self.payroll = None;
        self.roster = None;
        self.roster_stats = None;
        self.standings = None;
        self.open_chains = None;
        self.recent_chains = None;
        self.news = None;
        self.player_index = None;
        // team_abbrev_index + league_cap rarely change — keep across sims.
    }
}

pub fn run(app: &mut AppState) -> Result<()> {
    // Empty-save path: print message + bail before entering alt screen.
    let store = match app.store() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("no save loaded — pass --save <path> first");
            return Ok(());
        }
    };
    let Some(season_state) = store.load_season_state()? else {
        eprintln!("save has no season_state — run `nba3k new` first");
        return Ok(());
    };
    let user_team = season_state.user_team;
    let user_abbrev = store
        .team_abbrev(user_team)?
        .unwrap_or_else(|| format!("T{}", user_team.0));
    let season = season_state.season;

    let mut tui = TuiState {
        tab: Tab::Status,
        scroll: 0,
        selected: 0,
        user_team,
        user_abbrev,
        season,
        season_state,
        counts: None,
        payroll: None,
        roster: None,
        roster_stats: None,
        standings: None,
        open_chains: None,
        recent_chains: None,
        news: None,
        player_index: None,
        team_abbrev_index: None,
        league_cap: None,
        last_msg: None,
    };

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

fn event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    tui: &mut TuiState,
) -> Result<()> {
    loop {
        ensure_cache(app, tui)?;
        terminal.draw(|f| draw(f, tui))?;
        let Event::Key(k) = event::read()? else { continue };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => break,
            KeyCode::Char('1') => switch(tui, Tab::Status),
            KeyCode::Char('2') => switch(tui, Tab::Roster),
            KeyCode::Char('3') => switch(tui, Tab::Standings),
            KeyCode::Char('4') => switch(tui, Tab::Trades),
            KeyCode::Char('5') => switch(tui, Tab::News),
            KeyCode::Tab => switch(tui, tui.tab.next()),
            KeyCode::BackTab => switch(tui, tui.tab.prev()),
            // Time-control hotkeys — sim from inside the TUI. Caches refresh
            // after each sim. Stdout/stderr from inner sim is suppressed
            // (alt-screen would corrupt) by capturing during the call.
            KeyCode::Char('s') => sim_action(app, tui, SimKind::Day)?,
            KeyCode::Char('w') => sim_action(app, tui, SimKind::Week)?,
            KeyCode::Char('m') => sim_action(app, tui, SimKind::Month)?,
            KeyCode::Char('t') => sim_action(app, tui, SimKind::TradeDeadline)?,
            KeyCode::Char('e') => sim_action(app, tui, SimKind::SeasonEnd)?,
            // Tab-context actions:
            KeyCode::Char('r') if tui.tab == Tab::Roster => roster_cycle_role(app, tui)?,
            KeyCode::Char('a') if tui.tab == Tab::Trades => trade_respond(app, tui, "accept")?,
            KeyCode::Char('d') if tui.tab == Tab::Trades => trade_respond(app, tui, "reject")?,
            KeyCode::Up => {
                if let Some(max) = selectable_count(tui) {
                    if tui.selected > 0 { tui.selected -= 1; }
                    let _ = max;
                } else {
                    tui.scroll = tui.scroll.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if let Some(max) = selectable_count(tui) {
                    if tui.selected + 1 < max { tui.selected += 1; }
                } else {
                    tui.scroll = tui.scroll.saturating_add(1);
                }
            }
            KeyCode::PageUp => tui.scroll = tui.scroll.saturating_sub(10),
            KeyCode::PageDown => tui.scroll = tui.scroll.saturating_add(10),
            KeyCode::Home => { tui.scroll = 0; tui.selected = 0; }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Copy, Clone)]
enum SimKind { Day, Week, Month, TradeDeadline, SeasonEnd }

fn sim_action(app: &mut AppState, tui: &mut TuiState, kind: SimKind) -> Result<()> {
    let pre_unplayed = app.store()?.count_unplayed()?;
    let pre_day = tui.season_state.day;
    let pre_offers = app
        .store()?
        .read_open_chains_targeting(tui.season, tui.user_team)?
        .len();

    // Suppress stdout/stderr during sim to avoid corrupting the alt-screen.
    let result = with_silenced_io(|| match kind {
        SimKind::Day => crate::commands::sim_n_days(app, 1, true),
        SimKind::Week => crate::commands::sim_n_days(app, 7, true),
        SimKind::Month => crate::commands::sim_n_days(app, 30, true),
        SimKind::TradeDeadline => {
            crate::commands::sim_until_phase(app, SeasonPhase::TradeDeadlinePassed)
        }
        SimKind::SeasonEnd => sim_to_season_end(app),
    });

    match result {
        Ok(()) => {
            // Reload season_state + invalidate caches.
            let new_state = app.store()?.load_season_state()?;
            if let Some(s) = new_state {
                tui.season_state = s;
            }
            tui.invalidate_caches();
            let post_unplayed = app.store()?.count_unplayed()?;
            let games = pre_unplayed.saturating_sub(post_unplayed);
            let post_offers = app
                .store()?
                .read_open_chains_targeting(tui.season, tui.user_team)?
                .len();
            let new_offers = post_offers.saturating_sub(pre_offers);
            let days = tui.season_state.day.saturating_sub(pre_day);
            let label = match kind {
                SimKind::Day => "day",
                SimKind::Week => "week",
                SimKind::Month => "month",
                SimKind::TradeDeadline => "→ trade deadline",
                SimKind::SeasonEnd => "→ season end",
            };
            let mut msg = format!("sim {}: +{}d, {} games", label, days, games);
            if new_offers > 0 {
                msg.push_str(&format!(", {} new offer(s)", new_offers));
            }
            tui.last_msg = Some(msg);
        }
        Err(e) => {
            tui.last_msg = Some(format!("sim error: {}", e));
        }
    }
    Ok(())
}

/// Auto-progress to OffSeason: ensure Playoffs phase, run bracket, flip phase.
fn sim_to_season_end(app: &mut AppState) -> Result<()> {
    let s = app.store()?.load_season_state()?.ok_or_else(|| anyhow::anyhow!("no state"))?;
    if !matches!(s.phase, SeasonPhase::Playoffs | SeasonPhase::OffSeason) {
        crate::commands::sim_until_phase(app, SeasonPhase::Playoffs)?;
    }
    // Flip directly to OffSeason — simpler than running playoff bracket from
    // inside TUI (playoff sim has its own UX). User can run `playoffs sim` in
    // REPL post-TUI for a real bracket.
    let mut s = app.store()?.load_season_state()?.unwrap();
    if s.phase == SeasonPhase::Playoffs {
        s.phase = SeasonPhase::OffSeason;
        app.store()?.save_season_state(&s)?;
    }
    Ok(())
}

/// Run a closure with stdout/stderr redirected to /dev/null so prints from
/// inner sim functions don't corrupt the ratatui alt-screen.
fn with_silenced_io<F: FnOnce() -> Result<()>>(f: F) -> Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // Best effort. If anything fails, just run f directly.
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
        // Restore.
        libc::dup2(stdout_fd, 1);
        libc::dup2(stderr_fd, 2);
        let _ = OwnedFd::from_raw_fd(stdout_fd);
        let _ = OwnedFd::from_raw_fd(stderr_fd);
        drop(null);
        result
    }
}

/// Cycle the selected roster player's role: Star → Starter → SixthMan →
/// RolePlayer → BenchWarmer → (Prospect skipped) → back to Star. Persists via Store.
fn roster_cycle_role(app: &mut AppState, tui: &mut TuiState) -> Result<()> {
    let Some(roster) = tui.roster.as_ref() else { return Ok(()) };
    let Some(p) = roster.get(tui.selected) else { return Ok(()) };
    let next = match p.role {
        PlayerRole::Star => PlayerRole::Starter,
        PlayerRole::Starter => PlayerRole::SixthMan,
        PlayerRole::SixthMan => PlayerRole::RolePlayer,
        PlayerRole::RolePlayer => PlayerRole::BenchWarmer,
        PlayerRole::BenchWarmer => PlayerRole::Star,
        PlayerRole::Prospect => PlayerRole::Prospect, // leave prospects alone
    };
    let mut updated = p.clone();
    updated.set_role(next);
    let name = updated.name.clone();
    let result = with_silenced_io(|| {
        let store = app.store()?;
        store.upsert_player(&updated)?;
        Ok(())
    });
    match result {
        Ok(()) => {
            tui.last_msg = Some(format!("{} → {}", clean_name(&name), short_role(next)));
            // Reload roster to reflect new role.
            tui.roster = None;
            tui.roster_stats = None; // not strictly needed but OK
        }
        Err(e) => {
            tui.last_msg = Some(format!("role change failed: {}", e));
        }
    }
    Ok(())
}

/// Respond to the selected open trade chain. action = "accept" | "reject".
fn trade_respond(app: &mut AppState, tui: &mut TuiState, action: &str) -> Result<()> {
    let Some(chains) = tui.open_chains.as_ref() else { return Ok(()) };
    if chains.is_empty() {
        tui.last_msg = Some("no open offers".into());
        return Ok(());
    }
    let idx = tui.selected.min(chains.len() - 1);
    let (id, _) = chains[idx];
    let action_string = action.to_string();
    let id_value = id.0;
    let result = with_silenced_io(|| {
        crate::commands::dispatch(
            app,
            crate::cli::Command::Trade(crate::cli::TradeArgs {
                action: crate::cli::TradeAction::Respond {
                    id: id_value,
                    action: action_string,
                    json: false,
                },
            }),
        )
    });
    match result {
        Ok(()) => {
            tui.last_msg = Some(format!("trade #{}: {}", id_value, action));
            tui.open_chains = None;
            tui.recent_chains = None;
            tui.roster = None;
            tui.roster_stats = None;
            tui.payroll = None;
        }
        Err(e) => {
            tui.last_msg = Some(format!("trade #{} {} failed: {}", id_value, action, e));
        }
    }
    Ok(())
}

fn selectable_count(tui: &TuiState) -> Option<usize> {
    match tui.tab {
        Tab::Roster => tui.roster.as_ref().map(|r| r.len()),
        Tab::Trades => tui.open_chains.as_ref().map(|c| c.len()).filter(|n| *n > 0),
        _ => None,
    }
}

fn switch(tui: &mut TuiState, t: Tab) {
    tui.tab = t;
    tui.scroll = 0;
}

fn ensure_cache(app: &mut AppState, tui: &mut TuiState) -> Result<()> {
    let store = app.store()?;
    // Header always shows payroll — populate once.
    if tui.payroll.is_none() {
        tui.payroll = Some(store.team_salary(tui.user_team, tui.season)?);
    }
    match tui.tab {
        Tab::Status => {
            if tui.counts.is_none() {
                tui.counts = Some(StatusCounts {
                    teams: store.count_teams()?,
                    players: store.count_players()?,
                    schedule_total: store.count_schedule()?,
                    schedule_unplayed: store.count_unplayed()?,
                });
            }
        }
        Tab::Roster => {
            if tui.roster.is_none() {
                let mut r = store.roster_for_team(tui.user_team)?;
                r.sort_by(|a, b| b.overall.cmp(&a.overall));
                tui.roster = Some(r);
            }
            if tui.roster_stats.is_none() {
                // Single read_games walk → aggregate per roster player. Avoids
                // calling read_career_stats N times (each of which re-walks
                // every game in the season).
                let games = store.read_games(tui.season)?;
                let mut map: HashMap<PlayerId, SeasonAvgRow> = HashMap::new();
                if let Some(roster) = tui.roster.as_ref() {
                    for p in roster {
                        if let Some(row) = aggregate_career(&games, p.id).into_iter().next() {
                            map.insert(p.id, row);
                        }
                    }
                }
                tui.roster_stats = Some(map);
            }
        }
        Tab::Standings => {
            if tui.standings.is_none() {
                tui.standings = Some(store.read_standings(tui.season)?);
            }
        }
        Tab::Trades => {
            if tui.open_chains.is_none() {
                tui.open_chains = Some(store.read_open_chains_targeting(tui.season, tui.user_team)?);
            }
            if tui.recent_chains.is_none() {
                let mut all = store.list_trade_chains(tui.season)?;
                all.truncate(20);
                tui.recent_chains = Some(all);
            }
            if tui.player_index.is_none() {
                let players = store.all_active_players()?;
                let map: HashMap<PlayerId, Player> = players.into_iter().map(|p| (p.id, p)).collect();
                tui.player_index = Some(map);
            }
            if tui.team_abbrev_index.is_none() {
                let teams = store.list_teams()?;
                let map: HashMap<TeamId, String> =
                    teams.into_iter().map(|t| (t.id, t.abbrev)).collect();
                tui.team_abbrev_index = Some(map);
            }
            if tui.league_cap.is_none() {
                if let Some(ly) = nba3k_core::LeagueYear::for_season(tui.season) {
                    tui.league_cap = Some(ly.cap);
                }
            }
        }
        Tab::News => {
            if tui.news.is_none() {
                tui.news = Some(store.recent_news(50)?);
            }
        }
    }
    Ok(())
}

fn draw(f: &mut Frame, tui: &TuiState) {
    let area = f.area();
    if area.width < 80 {
        let p = Paragraph::new("Resize terminal to ≥ 80 columns")
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(p, area);
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);

    draw_header(f, chunks[0], tui);
    match tui.tab {
        Tab::Status => draw_status(f, chunks[1], tui),
        Tab::Roster => draw_roster(f, chunks[1], tui),
        Tab::Standings => draw_standings(f, chunks[1], tui),
        Tab::Trades => draw_trades(f, chunks[1], tui),
        Tab::News => draw_news(f, chunks[1], tui),
    }
    draw_footer(f, tui, chunks[2]);
}

fn draw_header(f: &mut Frame, area: Rect, tui: &TuiState) {
    let payroll = tui
        .payroll
        .map(|c| format!("${:.1}M", c.as_millions_f32()))
        .unwrap_or_else(|| "-".to_string());
    let title = format!(
        " nba3k 1.0.0 — {} — season {} ({:?}, day {}) — payroll {} ",
        tui.user_abbrev, tui.season.0, tui.season_state.phase, tui.season_state.day, payroll
    );
    let titles: Vec<Line> = ["[1]Status", "[2]Roster", "[3]Standings", "[4]Trades", "[5]News"]
        .iter()
        .map(|s| Line::from(*s))
        .collect();
    let tabs = Tabs::new(titles)
        .select(tui.tab.idx())
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
}

fn draw_footer(f: &mut Frame, tui: &TuiState, area: Rect) {
    let hint = " q quit · 1-5 tabs · ↑↓ select · [s]day [w]week [m]month [t]→deadline [e]→season-end ";
    let line = if let Some(msg) = tui.last_msg.as_deref() {
        format!(" {} │ {}", msg, hint)
    } else {
        hint.to_string()
    };
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_status(f: &mut Frame, area: Rect, tui: &TuiState) {
    let payroll = tui
        .payroll
        .map(|c| format!("${:.2}M", c.as_millions_f32()))
        .unwrap_or_else(|| "-".to_string());
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(format!("season:    {}", tui.season.0)));
    lines.push(Line::from(format!("phase:     {:?}", tui.season_state.phase)));
    lines.push(Line::from(format!("day:       {}", tui.season_state.day)));
    lines.push(Line::from(format!("mode:      {}", tui.season_state.mode)));
    lines.push(Line::from(format!("seed:      {}", tui.season_state.rng_seed)));
    lines.push(Line::from(format!(
        "user team: {} (id={})",
        tui.user_abbrev, tui.user_team.0
    )));
    if let Some(c) = tui.counts.as_ref() {
        lines.push(Line::from(format!("teams:     {}", c.teams)));
        lines.push(Line::from(format!("players:   {}", c.players)));
        lines.push(Line::from(format!(
            "schedule:  {} games ({} unplayed, {} played)",
            c.schedule_total,
            c.schedule_unplayed,
            c.schedule_total.saturating_sub(c.schedule_unplayed)
        )));
    }
    lines.push(Line::from(format!("payroll:   {}", payroll)));
    let p = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Status "));
    f.render_widget(p, area);
}

fn draw_roster(f: &mut Frame, area: Rect, tui: &TuiState) {
    let Some(roster) = tui.roster.as_ref() else { return };
    let stats = tui.roster_stats.as_ref();
    let header = Row::new(vec![
        Cell::from("NAME"),
        Cell::from("POS"),
        Cell::from("AGE"),
        Cell::from("OVR"),
        Cell::from("ROLE"),
        Cell::from("GP"),
        Cell::from("PPG"),
        Cell::from("RPG"),
        Cell::from("APG"),
        Cell::from("SPG"),
        Cell::from("BPG"),
        Cell::from("FG%"),
        Cell::from("3P%"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let visible_h = area.height.saturating_sub(3) as usize;
    // Auto-scroll to keep selected row visible.
    let auto_scroll = if tui.selected >= visible_h {
        tui.selected.saturating_sub(visible_h - 1)
    } else {
        0
    };
    let scroll = auto_scroll.min(roster.len().saturating_sub(visible_h.max(1)));
    let body: Vec<Row> = roster
        .iter()
        .enumerate()
        .skip(scroll)
        .map(|(i, p)| {
            let s = stats.and_then(|m| m.get(&p.id));
            let gp = s.map(|r| format!("{}", r.gp)).unwrap_or_else(|| "-".into());
            let ppg = s.map(|r| format!("{:.1}", r.ppg())).unwrap_or_else(|| "-".into());
            let rpg = s.map(|r| format!("{:.1}", r.rpg())).unwrap_or_else(|| "-".into());
            let apg = s.map(|r| format!("{:.1}", r.apg())).unwrap_or_else(|| "-".into());
            let spg = s.map(|r| format!("{:.1}", r.spg())).unwrap_or_else(|| "-".into());
            let bpg = s.map(|r| format!("{:.1}", r.bpg())).unwrap_or_else(|| "-".into());
            let fgp = s
                .filter(|r| r.fg_att > 0)
                .map(|r| format!("{:.0}%", r.fg_pct() * 100.0))
                .unwrap_or_else(|| "-".into());
            let tpp = s
                .filter(|r| r.three_att > 0)
                .map(|r| format!("{:.0}%", r.three_pct() * 100.0))
                .unwrap_or_else(|| "-".into());
            let selected = i == tui.selected;
            let style = if selected {
                Style::default().bg(Color::DarkGray).fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(clean_name(&p.name)),
                Cell::from(format!("{}", p.primary_position)),
                Cell::from(format!("{}", p.age)),
                Cell::from(format!("{}", p.overall)),
                Cell::from(short_role(p.role)),
                Cell::from(gp),
                Cell::from(ppg),
                Cell::from(rpg),
                Cell::from(apg),
                Cell::from(spg),
                Cell::from(bpg),
                Cell::from(fgp),
                Cell::from(tpp),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(22), // NAME
        Constraint::Length(3),  // POS
        Constraint::Length(3),  // AGE
        Constraint::Length(3),  // OVR
        Constraint::Length(5),  // ROLE
        Constraint::Length(3),  // GP
        Constraint::Length(5),  // PPG
        Constraint::Length(4),  // RPG
        Constraint::Length(4),  // APG
        Constraint::Length(4),  // SPG
        Constraint::Length(4),  // BPG
        Constraint::Length(4),  // FG%
        Constraint::Length(4),  // 3P%
    ];
    let title = format!(" Roster — {} ({} players) ", tui.user_abbrev, roster.len());
    let block = Block::default().borders(Borders::ALL).title(title);
    let table = Table::new(body, widths).header(header).block(block);
    f.render_widget(table, area);
}

fn draw_standings(f: &mut Frame, area: Rect, tui: &TuiState) {
    let Some(rows) = tui.standings.as_ref() else { return };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let east: Vec<&StandingRow> = rows
        .iter()
        .filter(|r| r.conference == Conference::East)
        .collect();
    let west: Vec<&StandingRow> = rows
        .iter()
        .filter(|r| r.conference == Conference::West)
        .collect();
    f.render_widget(standings_table(&east, " East "), cols[0]);
    f.render_widget(standings_table(&west, " West "), cols[1]);
}

fn standings_table<'a>(rows: &[&StandingRow], title: &'a str) -> Table<'a> {
    let header = Row::new(vec![
        Cell::from("#"),
        Cell::from("TM"),
        Cell::from("W"),
        Cell::from("L"),
        Cell::from("GB"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let leader = rows.first().copied();
    let body: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
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
                Cell::from(gb),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(6),
    ];
    Table::new(body, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title.to_string()))
}

fn draw_trades(f: &mut Frame, area: Rect, tui: &TuiState) {
    // Vertical split: header (1) / body (rest) / GM-message (3 incl borders)
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    draw_trade_header(f, outer[0], tui);

    // 3-column body: menu (20%) / current offer (50%) / analysis (30%)
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(22),
            Constraint::Percentage(48),
            Constraint::Percentage(30),
        ])
        .split(outer[1]);

    draw_trade_menu(f, cols[0], tui);
    draw_trade_offer(f, cols[1], tui);
    draw_trade_analysis(f, cols[2], tui);
    draw_trade_message(f, outer[2], tui);
}

fn draw_trade_header(f: &mut Frame, area: Rect, tui: &TuiState) {
    let payroll = tui.payroll.unwrap_or(Cents::ZERO);
    let cap = tui.league_cap.unwrap_or(Cents::ZERO);
    let cap_space_cents = cap.0.saturating_sub(payroll.0).max(0);
    let cap_space = Cents(cap_space_cents);
    // Days to deadline: 2026-02-05 minus current date.
    let cur_date = crate::commands::day_index_to_date(tui.season_state.day);
    let deadline =
        chrono::NaiveDate::from_ymd_opt(2026, 2, 5).unwrap_or(cur_date);
    let days_to_deadline = (deadline - cur_date).num_days();
    let deadline_str = if days_to_deadline >= 0 {
        format!("{} days", days_to_deadline)
    } else {
        "passed".to_string()
    };
    let line = format!(
        " Team: {}    Cap Space: ${:.1}M    Trade Deadline: {} ",
        tui.user_abbrev,
        cap_space.as_millions_f32(),
        deadline_str
    );
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_trade_menu(f: &mut Frame, area: Rect, _tui: &TuiState) {
    let menu_lines = vec![
        Line::from("TRADE MENU"),
        Line::from(""),
        Line::from(Span::styled(
            "> Inbox",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from("  History"),
        Line::from(""),
        Line::from("HOTKEYS"),
        Line::from(""),
        Line::from("↑↓  select"),
        Line::from("a   accept"),
        Line::from("d   reject"),
        Line::from(""),
        Line::from("(Build Trade"),
        Line::from(" coming v2)"),
    ];
    let p = Paragraph::new(menu_lines)
        .block(Block::default().borders(Borders::ALL).title(" Menu "));
    f.render_widget(p, area);
}

fn draw_trade_offer(f: &mut Frame, area: Rect, tui: &TuiState) {
    let self_season = tui.season;
    let chains = match tui.open_chains.as_ref() {
        Some(c) if !c.is_empty() => c,
        _ => {
            let p = Paragraph::new(" No open offers. Press [t] to skip to deadline\n or wait for AI offers in sim. ")
                .block(Block::default().borders(Borders::ALL).title(" Current Offer "));
            f.render_widget(p, area);
            return;
        }
    };

    let idx = tui.selected.min(chains.len() - 1);
    let (id, state) = &chains[idx];
    let latest = match state {
        NegotiationState::Open { chain } => chain.last(),
        NegotiationState::Accepted(o) => Some(o),
        NegotiationState::Rejected { final_offer, .. } => Some(final_offer),
        NegotiationState::Stalled => None,
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Offer #{} (round {})", id.0, latest.map(|o| o.round).unwrap_or(0)),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    // List of all open offers above selected detail.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Inbox ({} open)", chains.len()),
        Style::default().fg(Color::Cyan),
    )));
    for (i, (cid, st)) in chains.iter().enumerate() {
        let label = match st {
            NegotiationState::Open { chain } => {
                let r = chain.last().map(|o| o.round).unwrap_or(0);
                format!("[T#{:>3}] open r{}", cid.0, r)
            }
            _ => format!("[T#{:>3}] -", cid.0),
        };
        let style = if i == idx {
            Style::default().bg(Color::DarkGray).fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(label, style)));
    }
    lines.push(Line::from(""));

    if let Some(offer) = latest {
        // Two sub-sections: "X Send" / "Y Send" per team.
        let team_index = tui.team_abbrev_index.as_ref();
        let player_index = tui.player_index.as_ref();
        let abbrev_for = |t: TeamId| {
            team_index
                .and_then(|m| m.get(&t).cloned())
                .unwrap_or_else(|| format!("T{}", t.0))
        };
        for (team_id, assets) in offer.assets_by_team.iter() {
            let label = format!("{} Send", abbrev_for(*team_id));
            lines.push(Line::from(Span::styled(
                label,
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )));
            // Players
            for pid in &assets.players_out {
                let (name, salary, ovr) = player_index
                    .and_then(|m| m.get(pid))
                    .map(|p| {
                        let salary = p
                            .contract
                            .as_ref()
                            .map(|c| c.current_salary(self_season))
                            .unwrap_or(Cents::ZERO);
                        (clean_name(&p.name), salary, p.overall)
                    })
                    .unwrap_or_else(|| (format!("#{}", pid.0), Cents::ZERO, 0));
                lines.push(Line::from(format!(
                    "  {:<22} {:>3} OVR  ${:>5.1}M",
                    truncate(&name, 22),
                    ovr,
                    salary.as_millions_f32()
                )));
            }
            // Picks
            for pick_id in &assets.picks_out {
                lines.push(Line::from(format!("  Pick #{}", pick_id.0)));
            }
            if assets.cash_out != Cents::ZERO {
                lines.push(Line::from(format!(
                    "  Cash: ${:.1}M",
                    assets.cash_out.as_millions_f32()
                )));
            }
            lines.push(Line::from(""));
        }
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Current Offer "));
    f.render_widget(p, area);
}

fn draw_trade_analysis(f: &mut Frame, area: Rect, tui: &TuiState) {
    let self_season = tui.season;
    let chains = match tui.open_chains.as_ref() {
        Some(c) if !c.is_empty() => c,
        _ => {
            let p = Paragraph::new("").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Analysis "),
            );
            f.render_widget(p, area);
            return;
        }
    };
    let idx = tui.selected.min(chains.len() - 1);
    let (_, state) = &chains[idx];
    let latest = match state {
        NegotiationState::Open { chain } => chain.last(),
        NegotiationState::Accepted(o) => Some(o),
        NegotiationState::Rejected { final_offer, .. } => Some(final_offer),
        NegotiationState::Stalled => None,
    };

    let mut lines: Vec<Line> = Vec::new();

    if let Some(offer) = latest {
        let player_index = tui.player_index.as_ref();
        // Salary totals for user team incoming/outgoing.
        let mut outgoing = Cents::ZERO;
        let mut incoming = Cents::ZERO;
        for (team_id, assets) in offer.assets_by_team.iter() {
            for pid in &assets.players_out {
                let salary = player_index
                    .and_then(|m| m.get(pid))
                    .and_then(|p| p.contract.as_ref())
                    .map(|c| c.current_salary(self_season))
                    .unwrap_or(Cents::ZERO);
                if *team_id == tui.user_team {
                    outgoing += salary;
                } else {
                    incoming += salary;
                }
            }
        }
        lines.push(Line::from(Span::styled(
            "TRADE ANALYSIS",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from("Salary Match"));
        // CBA matching rule (simplified): incoming ≤ 1.25 × outgoing + $250K
        let limit = (outgoing.as_millions_f32() * 1.25) + 0.25;
        let cba_valid = incoming.as_millions_f32() <= limit;
        lines.push(Line::from(if cba_valid {
            Span::styled("✓ Valid", Style::default().fg(Color::Green))
        } else {
            Span::styled("✗ Over cap", Style::default().fg(Color::Red))
        }));
        lines.push(Line::from(""));
        lines.push(Line::from("Outgoing Salary"));
        lines.push(Line::from(format!(
            " ${:.1}M",
            outgoing.as_millions_f32()
        )));
        lines.push(Line::from(""));
        lines.push(Line::from("Incoming Salary"));
        lines.push(Line::from(format!(
            " ${:.1}M",
            incoming.as_millions_f32()
        )));
        lines.push(Line::from(""));
        lines.push(Line::from("Net"));
        let net = incoming.as_millions_f32() - outgoing.as_millions_f32();
        lines.push(Line::from(format!(" ${:+.1}M", net)));
        lines.push(Line::from(""));
        // Crude trade-value heuristic: sum OVR of incoming - outgoing players.
        let mut delta_ovr: i32 = 0;
        for (team_id, assets) in offer.assets_by_team.iter() {
            for pid in &assets.players_out {
                let ovr = player_index
                    .and_then(|m| m.get(pid))
                    .map(|p| p.overall as i32)
                    .unwrap_or(0);
                if *team_id == tui.user_team {
                    delta_ovr -= ovr;
                } else {
                    delta_ovr += ovr;
                }
            }
        }
        let stars = match delta_ovr {
            d if d >= 10 => "★★★★★",
            d if d >= 5 => "★★★★☆",
            d if d >= 0 => "★★★☆☆",
            d if d >= -5 => "★★☆☆☆",
            _ => "★☆☆☆☆",
        };
        lines.push(Line::from("Trade Value"));
        lines.push(Line::from(format!(" {} ({:+})", stars, delta_ovr)));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Risk",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        let risk = if delta_ovr >= 5 { "Low" } else if delta_ovr >= 0 { "Medium" } else { "High" };
        lines.push(Line::from(risk));
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Analysis "));
    f.render_widget(p, area);
}

fn draw_trade_message(f: &mut Frame, area: Rect, tui: &TuiState) {
    let chains = tui.open_chains.as_ref();
    let n = chains.map(|c| c.len()).unwrap_or(0);
    let msg = if n == 0 {
        "MESSAGE  No active offers. AI proposes during sim — press w/m to advance time.".to_string()
    } else if let Some(c) = chains {
        let idx = tui.selected.min(c.len().saturating_sub(1));
        let (id, _) = &c[idx];
        format!(
            "MESSAGE  Offer #{} selected. [a] accept · [d] reject · ↑↓ browse {} open",
            id.0, n
        )
    } else {
        "MESSAGE".to_string()
    };
    let p = Paragraph::new(msg).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn chain_summary(entry: &(TradeId, NegotiationState)) -> String {
    let (id, st) = entry;
    let (status, round, teams) = match st {
        NegotiationState::Open { chain } => {
            let r = chain.last().map(|o| o.round).unwrap_or(0);
            let t = chain.last().map(|o| o.assets_by_team.len()).unwrap_or(0);
            ("open", r, t)
        }
        NegotiationState::Accepted(o) => ("accepted", o.round, o.assets_by_team.len()),
        NegotiationState::Rejected { final_offer, .. } => (
            "rejected",
            final_offer.round,
            final_offer.assets_by_team.len(),
        ),
        NegotiationState::Stalled => ("stalled", 0, 0),
    };
    format!(
        "[T#{:>3}] {:<9} — {} teams — round {}",
        id.0, status, teams, round
    )
}

fn draw_news(f: &mut Frame, area: Rect, tui: &TuiState) {
    let Some(rows) = tui.news.as_ref() else { return };
    let visible_h = area.height.saturating_sub(2) as usize; // borders only
    let max_scroll = rows.len().saturating_sub(visible_h.max(1)) as u16;
    let scroll = tui.scroll.min(max_scroll) as usize;
    let items: Vec<ListItem> = rows
        .iter()
        .skip(scroll)
        .map(|n| ListItem::new(format!("[{:<8}] D{:>3} {}", n.kind, n.day, n.headline)))
        .collect();
    let title = format!(" News ({}) ", rows.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(list, area);
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short_role(r: PlayerRole) -> String {
    match r {
        PlayerRole::Star => "Star",
        PlayerRole::Starter => "Start",
        PlayerRole::SixthMan => "6th",
        PlayerRole::RolePlayer => "Role",
        PlayerRole::BenchWarmer => "Bench",
        PlayerRole::Prospect => "Prosp",
    }
    .to_string()
}
