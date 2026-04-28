//! Home dashboard. Single-screen overview: mandate · GM inbox · upcoming game ·
//! recent news. Lazy-loads each panel into a per-screen cache the first time
//! Home renders, then reuses the snapshot on subsequent draws until either the
//! user navigates away or `invalidate()` is called from outside.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::cell::RefCell;

use crate::state::AppState;
use crate::tui::widgets::Theme;
use crate::tui::{SaveCtx, TuiApp};
use nba3k_core::{t, Lang, PlayerRole, T};
use nba3k_store::{MandateRow, NewsRow, ScheduledRow};

// ---------------------------------------------------------------------------
// Per-screen cache (single-threaded TUI loop, so a thread_local RefCell is the
// simplest place to park state without touching the Wave-0 `TuiApp`).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct HomeCache {
    mandate: Option<MandatePanel>,
    inbox: Option<Vec<InboxRow>>,
    news: Option<Vec<NewsRow>>,
    upcoming: Option<Option<UpcomingRow>>,
    /// Vertical scroll offset for the inbox list.
    inbox_scroll: usize,
}

struct MandatePanel {
    season: u16,
    team: String,
    goals: Vec<MandateRow>,
}

struct InboxRow {
    kind: &'static str,
    text: String,
}

struct UpcomingRow {
    day: u32,
    opponent_abbrev: String,
    is_home: bool,
}

thread_local! {
    static CACHE: RefCell<HomeCache> = RefCell::new(HomeCache::default());
}

/// Drop the cached panels so the next render re-fetches. Called from the
/// shell whenever sim or save mutation happens. Safe to call any time.
pub fn invalidate() {
    CACHE.with(|c| *c.borrow_mut() = HomeCache::default());
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    let Some(ctx) = tui.save_ctx.as_ref() else {
        let p = Paragraph::new(t(tui.lang, T::CommonNoSaveLoaded))
            .block(theme.block(t(tui.lang, T::HomeTitle)));
        f.render_widget(p, area);
        return;
    };
    if let Err(e) = ensure_cache(app, ctx) {
        let p = Paragraph::new(format!("Home unavailable: {}", e))
            .block(theme.block(t(tui.lang, T::HomeTitle)));
        f.render_widget(p, area);
        return;
    }

    // Top half: mandate (left) + upcoming (right banner) + inbox (right body).
    // Bottom half: news strip.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(outer[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(top[1]);

    CACHE.with(|c| {
        let cache = c.borrow();
        draw_mandate(f, top[0], theme, tui.lang, cache.mandate.as_ref());
        draw_upcoming(
            f,
            right[0],
            theme,
            tui.lang,
            cache.upcoming.as_ref().and_then(|x| x.as_ref()),
            ctx,
        );
        draw_inbox(
            f,
            right[1],
            theme,
            tui.lang,
            cache.inbox.as_deref().unwrap_or(&[]),
            cache.inbox_scroll,
        );
        draw_news(f, outer[1], theme, tui.lang, cache.news.as_deref().unwrap_or(&[]));
    });
}

fn draw_mandate(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    lang: Lang,
    panel: Option<&MandatePanel>,
) {
    let block = theme.block(t(lang, T::HomeOwnerMandate));
    let lines: Vec<Line> = match panel {
        None => vec![Line::from(Span::styled(t(lang, T::HomeNoMandate), theme.muted_style()))],
        Some(m) => {
            let mut out: Vec<Line> = Vec::with_capacity(m.goals.len() + 2);
            out.push(Line::from(Span::styled(
                format!("Season {} · {}", m.season, m.team),
                theme.accent_style(),
            )));
            out.push(Line::from(""));
            if m.goals.is_empty() {
                out.push(Line::from(Span::styled(t(lang, T::HomeNoGoals), theme.muted_style())));
            } else {
                for g in &m.goals {
                    out.push(Line::from(vec![
                        Span::styled(
                            format!(" {:<14}", g.kind),
                            theme.text(),
                        ),
                        Span::styled(
                            format!("target {:>3}  weight {:.2}", g.target, g.weight),
                            theme.muted_style(),
                        ),
                    ]));
                }
            }
            out
        }
    };
    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_upcoming(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    lang: Lang,
    row: Option<&UpcomingRow>,
    ctx: &SaveCtx,
) {
    let line = match row {
        None => Line::from(vec![Span::styled(
            t(lang, T::HomeNoUpcomingGames),
            theme.muted_style(),
        )]),
        Some(u) => {
            let prep = if u.is_home { "vs" } else { "@" };
            let venue = if u.is_home { "home" } else { "away" };
            Line::from(vec![
                Span::styled(format!(" Day {} ", u.day), theme.accent_style()),
                Span::styled(
                    format!("— {} {} {} ({})", ctx.user_abbrev, prep, u.opponent_abbrev, venue),
                    theme.text(),
                ),
            ])
        }
    };
    let p = Paragraph::new(line).block(theme.block(t(lang, T::HomeNextGame)));
    f.render_widget(p, area);
}

fn draw_inbox(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, rows: &[InboxRow], scroll: usize) {
    let block = theme.block(t(lang, T::HomeGmInbox));
    if rows.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            t(lang, T::HomeNoAlerts),
            theme.muted_style(),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    }
    let visible_h = area.height.saturating_sub(2) as usize;
    let start = scroll.min(rows.len().saturating_sub(1));
    let take = visible_h.max(1);
    let items: Vec<ListItem> = rows
        .iter()
        .skip(start)
        .take(take)
        .map(|r| {
            let line = Line::from(vec![
                Span::styled(format!("[{}] ", r.kind), theme.accent_style()),
                Span::styled(r.text.clone(), theme.text()),
            ]);
            ListItem::new(line)
        })
        .collect();
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn draw_news(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, rows: &[NewsRow]) {
    let block = theme.block(t(lang, T::HomeRecentNews));
    if rows.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            t(lang, T::HomeNoNews),
            theme.muted_style(),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    }
    let visible_h = area.height.saturating_sub(2) as usize;
    let take = visible_h.max(1);
    let lines: Vec<Line> = rows
        .iter()
        .take(take)
        .map(|n| {
            Line::from(vec![
                Span::styled(
                    format!("S{} D{:<3} ", n.season.0, n.day),
                    theme.muted_style(),
                ),
                Span::styled(format!("[{:<8}] ", n.kind), theme.accent_style()),
                Span::styled(n.headline.clone(), theme.text()),
            ])
        })
        .collect();
    let p = Paragraph::new(lines)
        .block(block)
        .style(Style::default());
    f.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Cache population
// ---------------------------------------------------------------------------

fn ensure_cache(app: &mut AppState, ctx: &SaveCtx) -> Result<()> {
    let need_mandate = CACHE.with(|c| c.borrow().mandate.is_none());
    let need_inbox = CACHE.with(|c| c.borrow().inbox.is_none());
    let need_news = CACHE.with(|c| c.borrow().news.is_none());
    let need_upcoming = CACHE.with(|c| c.borrow().upcoming.is_none());

    if need_mandate {
        let store = app.store()?;
        let goals = store.read_mandates(ctx.season, ctx.user_team)?;
        let team = store
            .team_abbrev(ctx.user_team)?
            .unwrap_or_else(|| ctx.user_abbrev.clone());
        CACHE.with(|c| {
            c.borrow_mut().mandate = Some(MandatePanel {
                season: ctx.season.0,
                team,
                goals,
            });
        });
    }

    if need_inbox {
        let inbox = build_inbox(app, ctx)?;
        CACHE.with(|c| c.borrow_mut().inbox = Some(inbox));
    }

    if need_news {
        let news = app.store()?.recent_news(10)?;
        CACHE.with(|c| c.borrow_mut().news = Some(news));
    }

    if need_upcoming {
        let upcoming = find_next_user_game(app, ctx)?;
        CACHE.with(|c| c.borrow_mut().upcoming = Some(upcoming));
    }

    Ok(())
}

fn build_inbox(app: &mut AppState, ctx: &SaveCtx) -> Result<Vec<InboxRow>> {
    let store = app.store()?;
    let roster = store.roster_for_team(ctx.user_team)?;
    let mut out: Vec<InboxRow> = Vec::new();

    // Roster alerts mirror the rules in `cmd_messages` (commands.rs L3329+):
    // injuries first, then trade-demand / role-mismatch / veteran-restless.
    for p in &roster {
        let name = clean_name(&p.name);
        if let Some(i) = p.injury.as_ref() {
            if i.games_remaining > 0 {
                out.push(InboxRow {
                    kind: "injury",
                    text: format!(
                        "{} — {}, {} game{} out.",
                        name,
                        i.description,
                        i.games_remaining,
                        if i.games_remaining == 1 { "" } else { "s" }
                    ),
                });
                continue;
            }
        }
        if p.overall >= 80 && p.morale < 0.5 {
            out.push(InboxRow {
                kind: "demand",
                text: format!(
                    "{} (OVR {}) is unhappy (morale {:.2}) — asking out.",
                    name, p.overall, p.morale
                ),
            });
            continue;
        }
        if p.overall >= 80
            && matches!(p.role, PlayerRole::BenchWarmer | PlayerRole::SixthMan)
        {
            out.push(InboxRow {
                kind: "role",
                text: format!(
                    "{} (OVR {}) slotted as {} — morale will drop.",
                    name, p.overall, p.role
                ),
            });
            continue;
        }
        if p.age >= 36 && p.morale < 0.5 {
            out.push(InboxRow {
                kind: "veteran",
                text: format!("{} ({}yo) wants a contender.", name, p.age),
            });
        }
    }

    // Open offers targeting the user team.
    let offers = store.read_open_chains_targeting(ctx.season, ctx.user_team)?;
    for (id, _) in &offers {
        out.push(InboxRow {
            kind: "offer",
            text: format!("Trade offer #{} pending review.", id.0),
        });
    }

    // Notes (favorite players).
    let notes = store.list_notes()?;
    for n in &notes {
        let name = match store.player_name(n.player_id)? {
            Some(s) => clean_name(&s),
            None => format!("#{}", n.player_id.0),
        };
        out.push(InboxRow {
            kind: "note",
            text: format!("{}: {}", name, n.text.as_deref().unwrap_or("(tracked)")),
        });
    }

    out.truncate(10);
    Ok(out)
}

fn find_next_user_game(app: &mut AppState, ctx: &SaveCtx) -> Result<Option<UpcomingRow>> {
    let store = app.store()?;
    let last = store.last_scheduled_date()?;
    let through = match last {
        Some(d) => d,
        None => return Ok(None),
    };
    let pending: Vec<ScheduledRow> = store.pending_games_through(through)?;
    let next = pending
        .into_iter()
        .find(|g| g.home == ctx.user_team || g.away == ctx.user_team);
    let Some(g) = next else { return Ok(None) };

    let is_home = g.home == ctx.user_team;
    let opp_id = if is_home { g.away } else { g.home };
    let opp_abbrev = store
        .team_abbrev(opp_id)?
        .unwrap_or_else(|| format!("T{}", opp_id.0));

    // Day index: derive from the season-start date (legacy convention is
    // 2025-10-14). The shell exposes `season_state.day` for the current day,
    // so a simple offset gives a "Day N" label.
    let start = chrono::NaiveDate::from_ymd_opt(2025, 10, 14).expect("valid");
    let day = g.date.signed_duration_since(start).num_days().max(0) as u32;

    Ok(Some(UpcomingRow {
        day,
        opponent_abbrev: opp_abbrev,
        is_home,
    }))
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Key handling — only ↑/↓ scroll the inbox; everything else falls through.
// ---------------------------------------------------------------------------

pub fn handle_key(
    _app: &mut AppState,
    _tui: &mut TuiApp,
    key: KeyEvent,
) -> Result<bool> {
    match key.code {
        KeyCode::Up => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                if c.inbox_scroll > 0 {
                    c.inbox_scroll -= 1;
                }
            });
            Ok(true)
        }
        KeyCode::Down => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let max = c.inbox.as_ref().map(|v| v.len()).unwrap_or(0).saturating_sub(1);
                if c.inbox_scroll < max {
                    c.inbox_scroll += 1;
                }
            });
            Ok(true)
        }
        _ => Ok(false),
    }
}
