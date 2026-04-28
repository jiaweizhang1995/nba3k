//! Inbox screen. Read-only GM messages, trade demands, and league news.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Clear, Paragraph, Row, Table, Wrap},
    Frame,
};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::state::AppState;
use crate::tui::widgets::{ActionBar, Theme};
use crate::tui::TuiApp;
use nba3k_core::{t, NegotiationState, PlayerId, PlayerRole, T};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Messages,
    TradeDemands,
    News,
}

#[derive(Default)]
struct InboxCache {
    snapshot: Option<InboxSnapshot>,
    tab: Tab,
    cursor: usize,
    modal: Option<InboxRow>,
}

#[derive(Clone, Debug, Default)]
struct InboxSnapshot {
    messages: Vec<InboxRow>,
    demands: Vec<InboxRow>,
    news: Vec<InboxRow>,
}

#[derive(Clone, Debug)]
struct InboxRow {
    date: String,
    subject: String,
    preview: String,
    body: Vec<String>,
}

thread_local! {
    static CACHE: RefCell<InboxCache> = RefCell::new(InboxCache::default());
}

pub fn invalidate() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.snapshot = None;
        c.cursor = 0;
        c.modal = None;
    });
}

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    if !tui.has_save() {
        let p = Paragraph::new(t(tui.lang, T::CommonNoSaveLoaded))
            .block(theme.block(t(tui.lang, T::InboxTitle)));
        f.render_widget(p, area);
        return;
    }

    if let Err(e) = ensure_cache(app, tui) {
        let p = Paragraph::new(format!("Inbox unavailable: {}", e))
            .block(theme.block(t(tui.lang, T::InboxTitle)));
        f.render_widget(p, area);
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);

    draw_tabs(f, parts[0], theme, tui);
    draw_rows(f, parts[1], theme, tui);
    ActionBar::new(&[
        ("Up/Down", t(tui.lang, T::CommonMove)),
        ("Tab", t(tui.lang, T::CommonTabs)),
        ("Enter", t(tui.lang, T::CommonDetail)),
        ("Esc", t(tui.lang, T::CommonBack)),
    ])
    .render(f, parts[2], theme);

    let modal = CACHE.with(|c| c.borrow().modal.clone());
    if let Some(row) = modal {
        let rect = modal_rect(area);
        f.render_widget(Clear, rect);
        draw_modal(f, rect, theme, row);
    }
}

fn draw_tabs(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp) {
    let (tab, counts) = CACHE.with(|c| {
        let c = c.borrow();
        let snapshot = c.snapshot.clone().unwrap_or_default();
        (
            c.tab,
            [
                snapshot.messages.len(),
                snapshot.demands.len(),
                snapshot.news.len(),
            ],
        )
    });
    let specs = [
        (Tab::Messages, T::InboxMessages, counts[0]),
        (Tab::TradeDemands, T::InboxTradeDemands, counts[1]),
        (Tab::News, T::InboxNews, counts[2]),
    ];
    let mut spans = Vec::new();
    for (idx, (candidate, key, count)) in specs.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw("  "));
        }
        let style = if *candidate == tab {
            theme.highlight()
        } else {
            theme.text()
        };
        spans.push(Span::styled(
            format!("[{}] {} ({})", idx + 1, t(tui.lang, *key), count),
            style,
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).block(theme.block(t(tui.lang, T::InboxTitle))),
        area,
    );
}

fn draw_rows(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp) {
    let (tab, cursor, rows) = CACHE.with(|c| {
        let c = c.borrow();
        let snapshot = c.snapshot.clone().unwrap_or_default();
        let rows = match c.tab {
            Tab::Messages => snapshot.messages,
            Tab::TradeDemands => snapshot.demands,
            Tab::News => snapshot.news,
        };
        (c.tab, c.cursor, rows)
    });

    if rows.is_empty() {
        let empty_key = match tab {
            Tab::Messages => T::InboxNoMessages,
            Tab::TradeDemands => T::InboxNoDemands,
            Tab::News => T::InboxNoNews,
        };
        let p = Paragraph::new(Span::styled(t(tui.lang, empty_key), theme.muted_style()))
            .block(theme.block(""));
        f.render_widget(p, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from(Span::styled("Date", theme.accent_style())),
        Cell::from(Span::styled("Subject", theme.accent_style())),
        Cell::from(Span::styled("Preview", theme.accent_style())),
    ]);
    let body: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let style = if i == cursor {
                theme.highlight()
            } else {
                theme.text()
            };
            Row::new(vec![
                Cell::from(Span::styled(row.date.clone(), style)),
                Cell::from(Span::styled(truncate(&row.subject, 28), style)),
                Cell::from(Span::styled(truncate(&row.preview, 70), style)),
            ])
        })
        .collect();
    let table = Table::new(
        body,
        [
            Constraint::Length(12),
            Constraint::Length(30),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(theme.block(""));
    f.render_widget(table, area);
}

fn draw_modal(f: &mut Frame, area: Rect, theme: &Theme, row: InboxRow) {
    let mut lines = vec![
        Line::from(Span::styled(row.date, theme.muted_style())),
        Line::from(""),
    ];
    for body in row.body {
        lines.push(Line::from(Span::styled(body, theme.text())));
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(theme.block(&format!(" {} ", row.subject)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn ensure_cache(app: &mut AppState, tui: &TuiApp) -> Result<()> {
    let needs_load = CACHE.with(|c| c.borrow().snapshot.is_none());
    if !needs_load {
        return Ok(());
    }

    let snapshot = build_snapshot(app, tui)?;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.snapshot = Some(snapshot);
        c.cursor = 0;
    });
    Ok(())
}

fn build_snapshot(app: &mut AppState, tui: &TuiApp) -> Result<InboxSnapshot> {
    let store = app.store()?;
    let roster = store.roster_for_team(tui.user_team)?;
    let mut messages = Vec::new();
    let mut demands = Vec::new();

    for p in &roster {
        let name = clean_name(&p.name);
        if let Some(injury) = p.injury.as_ref() {
            if injury.games_remaining > 0 {
                messages.push(row(
                    "Today",
                    "Injury report",
                    format!(
                        "{}: {}, {} game{} out.",
                        name,
                        injury.description,
                        injury.games_remaining,
                        if injury.games_remaining == 1 { "" } else { "s" }
                    ),
                ));
                continue;
            }
        }

        if p.overall >= 80 && p.morale < 0.5 {
            let msg = format!(
                "{} (OVR {}, role {}) is unhappy (morale {:.2}) and asking out.",
                name, p.overall, p.role, p.morale
            );
            let demand = row("Today", "Trade demand", msg);
            messages.push(demand.clone());
            demands.push(demand);
            continue;
        }

        if p.overall >= 80 && matches!(p.role, PlayerRole::BenchWarmer | PlayerRole::SixthMan) {
            messages.push(row(
                "Today",
                "Role mismatch",
                format!(
                    "{} is an OVR-{} talent slotted as {}; morale risk is rising.",
                    name, p.overall, p.role
                ),
            ));
            continue;
        }

        if p.age >= 36 && p.morale < 0.5 {
            messages.push(row(
                "Today",
                "Veteran restless",
                format!("{} ({}yo) wants a contender.", name, p.age),
            ));
        }
    }

    for (id, state) in store.read_open_chains_targeting(tui.season, tui.user_team)? {
        let NegotiationState::Open { chain } = state else {
            continue;
        };
        let Some(latest) = chain.last() else { continue };
        let from = store
            .team_abbrev(latest.initiator)?
            .unwrap_or_else(|| format!("T{}", latest.initiator.0));
        messages.push(InboxRow {
            date: "Today".to_string(),
            subject: format!("Trade offer #{}", id.0),
            preview: format!("Incoming offer from {}.", from),
            body: vec![
                format!("Incoming trade offer from {}.", from),
                "Review and respond from the Trades screen.".to_string(),
                format!("Participants: {}", latest.assets_by_team.len()),
            ],
        });
    }

    let notes = store.list_notes()?;
    if !notes.is_empty() {
        let active = store.all_active_players()?;
        let names_by_id: HashMap<PlayerId, String> = active
            .into_iter()
            .map(|p| (p.id, clean_name(&p.name)))
            .collect();
        for note in notes {
            let name = names_by_id
                .get(&note.player_id)
                .cloned()
                .or(store.player_name(note.player_id)?.map(|s| clean_name(&s)))
                .unwrap_or_else(|| format!("#{}", note.player_id.0));
            let text = note.text.unwrap_or_else(|| "(tracked)".to_string());
            messages.push(row(
                note.created_at,
                "Tracked player",
                format!("{}: {}", name, text),
            ));
        }
    }

    let news = store
        .recent_news(50)?
        .into_iter()
        .map(|n| InboxRow {
            date: format!("S{} D{}", n.season.0, n.day),
            subject: n.headline.clone(),
            preview: format!("[{}] {}", n.kind, n.body.clone().unwrap_or_default()),
            body: vec![
                format!("Kind: {}", n.kind),
                n.body.unwrap_or_else(|| n.headline),
            ],
        })
        .collect();

    Ok(InboxSnapshot {
        messages,
        demands,
        news,
    })
}

fn row(
    date: impl Into<String>,
    subject: impl Into<String>,
    message: impl Into<String>,
) -> InboxRow {
    let message = message.into();
    InboxRow {
        date: date.into(),
        subject: subject.into(),
        preview: message.clone(),
        body: vec![message],
    }
}

pub fn handle_key(_app: &mut AppState, _tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    let modal_handled = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.modal.is_some() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                c.modal = None;
            }
            true
        } else {
            false
        }
    });
    if modal_handled {
        return Ok(true);
    }

    match key.code {
        KeyCode::Tab => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.tab = next_tab(c.tab);
                c.cursor = 0;
            });
            Ok(true)
        }
        KeyCode::Char('1') => set_tab(Tab::Messages),
        KeyCode::Char('2') => set_tab(Tab::TradeDemands),
        KeyCode::Char('3') => set_tab(Tab::News),
        KeyCode::Up => move_cursor(-1),
        KeyCode::Down => move_cursor(1),
        KeyCode::PageUp => move_cursor(-10),
        KeyCode::PageDown => move_cursor(10),
        KeyCode::Enter => {
            let row = current_row();
            CACHE.with(|c| c.borrow_mut().modal = row);
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn set_tab(tab: Tab) -> Result<bool> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.tab = tab;
        c.cursor = 0;
    });
    Ok(true)
}

fn next_tab(tab: Tab) -> Tab {
    match tab {
        Tab::Messages => Tab::TradeDemands,
        Tab::TradeDemands => Tab::News,
        Tab::News => Tab::Messages,
    }
}

fn move_cursor(delta: isize) -> Result<bool> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let len = rows_for(&c).len();
        if len == 0 {
            c.cursor = 0;
            return;
        }
        let max = len.saturating_sub(1) as isize;
        c.cursor = (c.cursor as isize + delta).clamp(0, max) as usize;
    });
    Ok(true)
}

fn current_row() -> Option<InboxRow> {
    CACHE.with(|c| {
        let c = c.borrow();
        rows_for(&c).get(c.cursor).cloned()
    })
}

fn rows_for(cache: &InboxCache) -> Vec<InboxRow> {
    let snapshot = cache.snapshot.clone().unwrap_or_default();
    match cache.tab {
        Tab::Messages => snapshot.messages,
        Tab::TradeDemands => snapshot.demands,
        Tab::News => snapshot.news,
    }
}

fn modal_rect(area: Rect) -> Rect {
    let w = area.width.saturating_sub(8).min(96).max(40);
    let h = area.height.saturating_sub(4).min(24).max(8);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(s: &str, n: usize) -> String {
    let mut out = s.chars().take(n).collect::<String>();
    if s.chars().count() > n {
        out.push_str("...");
    }
    out
}
