//! Trades screen (M22).
//!
//! Five sub-tabs:
//! - Inbox: AI offers targeting the user's team.
//! - My Proposals: user-involved negotiation chains.
//! - Builder: 2-team or 3-team player-for-player proposal flow.
//! - Rumors: read-only market interest table.
//! - Free Agents: signable FA pool.
//!
//! All mutations route through `commands::dispatch` behind `with_silenced_io`
//! so command output cannot corrupt the alt-screen.

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Clear, Paragraph, Row, Table, Wrap},
    Frame,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::cli::{Command, FaAction, FaArgs, TradeAction, TradeArgs};
use crate::commands::{self, dispatch};
use crate::state::AppState;
use crate::tui::widgets::{ActionBar, Theme};
use crate::tui::{with_silenced_io, TuiApp};
use nba3k_core::{
    t, DraftPick, DraftPickId, Lang, LeagueSnapshot, LeagueYear, NegotiationState, Player, PlayerId,
    PlayerRole, Position, RejectReason, SeasonId, SeasonPhase, Team, TeamId, TeamRecordSummary,
    TradeId, TradeOffer, Verdict, T,
};
use nba3k_models::stat_projection::infer_archetype;
use nba3k_trade::evaluate as evaluate_mod;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SubTab {
    #[default]
    Inbox,
    Proposals,
    Builder,
    Rumors,
    FreeAgents,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BuilderPanel {
    #[default]
    Team,
    Outgoing,
    Incoming,
    Submit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BuilderMode {
    #[default]
    TwoTeam,
    ThreeTeam,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum IncomingSlot {
    #[default]
    First,
    Second,
}

#[derive(Clone, Debug)]
struct OfferRow {
    id: TradeId,
    from: TeamId,
    from_abbrev: String,
    wants: String,
    sends: String,
    verdict: String,
    probability: f32,
    commentary: String,
    detail_lines: Vec<String>,
}

#[derive(Clone, Debug)]
struct ChainRow {
    id: TradeId,
    status: String,
    round: u8,
    teams: String,
    verdict: String,
    open: bool,
    detail_lines: Vec<String>,
}

#[derive(Clone, Debug)]
struct RumorRow {
    player_name: String,
    team_abbrev: String,
    ovr: u8,
    role: PlayerRole,
    interest: u32,
    suitors: Vec<String>,
}

#[derive(Clone, Debug)]
struct FaRow {
    player_id: PlayerId,
    name: String,
    position: Position,
    age: u8,
    overall: u8,
    asking_m: f32,
}

#[derive(Clone, Debug)]
struct TeamOption {
    id: TeamId,
    abbrev: String,
    name: String,
}

#[derive(Clone, Debug)]
struct PlayerOption {
    id: PlayerId,
    name: String,
    raw_name: String,
    position: Position,
    age: u8,
    overall: u8,
    salary_m: f32,
}

// Roster cap mirrors `commands::FA_ROSTER_CAP` (15 std + 3 two-way = 18).
const FA_ROSTER_CAP: usize = 18;

#[derive(Default)]
struct TradesCache {
    inbox_rows: Option<Vec<OfferRow>>,
    chain_rows: Option<Vec<ChainRow>>,
    rumor_rows: Option<Vec<RumorRow>>,
    fa_rows: Option<Vec<FaRow>>,
    teams: Option<Vec<TeamOption>>,
    user_roster: Option<Vec<PlayerOption>>,
    target_roster: Option<Vec<PlayerOption>>,
    third_roster: Option<Vec<PlayerOption>>,
    target_team: Option<TeamId>,
    third_team: Option<TeamId>,

    tab: SubTab,
    inbox_cursor: usize,
    chain_cursor: usize,
    rumor_cursor: usize,
    fa_cursor: usize,

    builder_panel: BuilderPanel,
    builder_mode: BuilderMode,
    incoming_slot: IncomingSlot,
    team_cursor: usize,
    out_cursor: usize,
    in_cursor: usize,
    selected_out: HashSet<PlayerId>,
    selected_in: HashSet<PlayerId>,
    selected_third: HashSet<PlayerId>,
    modal: Modal,
}

#[derive(Default)]
enum Modal {
    #[default]
    None,
    OfferDetail {
        id: TradeId,
    },
    ChainDetail {
        id: TradeId,
    },
    Message {
        title: String,
        lines: Vec<String>,
    },
}

thread_local! {
    static CACHE: RefCell<TradesCache> = RefCell::new(TradesCache::default());
}

pub fn invalidate() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.inbox_rows = None;
        c.chain_rows = None;
        c.rumor_rows = None;
        c.fa_rows = None;
        c.teams = None;
        c.user_roster = None;
        c.target_roster = None;
        c.third_roster = None;
    });
}

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    if !tui.has_save() {
        let p = Paragraph::new(t(tui.lang, T::CommonNoSaveLoaded))
            .block(theme.block(t(tui.lang, T::TradesTitle)));
        f.render_widget(p, area);
        return;
    }
    if let Err(e) = ensure_cache(app, tui) {
        let p = Paragraph::new(format!("Trades unavailable: {}", e))
            .block(theme.block(t(tui.lang, T::TradesTitle)));
        f.render_widget(p, area);
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let tab = CACHE.with(|c| c.borrow().tab);
    draw_tab_strip(f, parts[0], theme, tui.lang, tab);

    match tab {
        SubTab::Inbox => draw_inbox(f, parts[1], theme, tui.lang),
        SubTab::Proposals => draw_proposals(f, parts[1], theme, tui.lang),
        SubTab::Builder => draw_builder(f, parts[1], theme, tui),
        SubTab::Rumors => draw_rumors(f, parts[1], theme, tui.lang),
        SubTab::FreeAgents => draw_free_agents(f, parts[1], theme, tui.lang),
    }

    if CACHE.with(|c| !matches!(c.borrow().modal, Modal::None)) {
        let rect = modal_rect(area);
        f.render_widget(Clear, rect);
        draw_modal(f, rect, theme, tui.lang);
    }
}

fn draw_tab_strip(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, tab: SubTab) {
    let style = |t| {
        if tab == t {
            theme.highlight()
        } else {
            theme.muted_style()
        }
    };
    let line = Line::from(vec![
        Span::styled(format!(" 1. {} ", t(lang, T::TradesInbox)), style(SubTab::Inbox)),
        Span::styled("   ", theme.text()),
        Span::styled(format!(" 2. {} ", t(lang, T::TradesMyProposals)), style(SubTab::Proposals)),
        Span::styled("   ", theme.text()),
        Span::styled(format!(" 3. {} ", t(lang, T::TradesBuilder)), style(SubTab::Builder)),
        Span::styled("   ", theme.text()),
        Span::styled(format!(" 4. {} ", t(lang, T::TradesRumors)), style(SubTab::Rumors)),
        Span::styled("   ", theme.text()),
        Span::styled(
            format!(" 5. {} ", t(lang, T::RosterFreeAgents)),
            style(SubTab::FreeAgents),
        ),
    ]);
    f.render_widget(Paragraph::new(line).block(theme.block(t(lang, T::TradesTitle))), area);
}

fn draw_inbox(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let rows = cache.inbox_rows.as_deref().unwrap_or(&[]);
        let cursor = cache.inbox_cursor.min(rows.len().saturating_sub(1));
        let parts = body_with_bar(area);

        if rows.is_empty() {
            let p =
                Paragraph::new(t(lang, T::TradesIncomingOffersNone)).block(theme.block(t(lang, T::TradesInbox)));
            f.render_widget(p, parts[0]);
        } else {
            let header = Row::new(vec![
                head("ID", theme),
                head("FROM", theme),
                head("WANTS", theme),
                head("SENDS", theme),
                head(t(lang, T::ModalTradeVerdictTitle), theme),
            ]);
            let body: Vec<Row> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let style = if i == cursor {
                        theme.highlight()
                    } else {
                        theme.text()
                    };
                    Row::new(vec![
                        Cell::from(Span::styled(format!("{:>3}", r.id.0), style)),
                        Cell::from(Span::styled(r.from_abbrev.clone(), style)),
                        Cell::from(Span::styled(shorten(&r.wants, 28), style)),
                        Cell::from(Span::styled(shorten(&r.sends, 34), style)),
                        Cell::from(Span::styled(r.verdict.clone(), style)),
                    ])
                })
                .collect();
            let title = format!(" {} ({}) ", t(lang, T::TradesInbox), rows.len());
            let table = Table::new(
                body,
                [
                    Constraint::Length(5),
                    Constraint::Length(7),
                    Constraint::Percentage(30),
                    Constraint::Percentage(36),
                    Constraint::Length(10),
                ],
            )
            .header(header)
            .block(theme.block(&title));
            f.render_widget(table, parts[0]);
        }

        ActionBar::new(&[
            ("a", t(lang, T::TradesAccept)),
            ("r", t(lang, T::TradesReject)),
            ("c", t(lang, T::TradesCounter)),
            ("Enter", t(lang, T::CommonDetail)),
            ("Tab", t(lang, T::CommonTabs)),
            ("Esc", t(lang, T::CommonBack)),
        ])
        .render(f, parts[1], theme);
    });
}

fn draw_proposals(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let rows = cache.chain_rows.as_deref().unwrap_or(&[]);
        let cursor = cache.chain_cursor.min(rows.len().saturating_sub(1));
        let parts = body_with_bar(area);

        if rows.is_empty() {
            let p = Paragraph::new(t(lang, T::TradesNoProposals))
                .block(theme.block(t(lang, T::TradesMyProposals)));
            f.render_widget(p, parts[0]);
        } else {
            let header = Row::new(vec![
                head("ID", theme),
                head("STATUS", theme),
                head("ROUND", theme),
                head("TEAMS", theme),
                head(t(lang, T::ModalTradeVerdictTitle), theme),
            ]);
            let body: Vec<Row> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let style = if i == cursor {
                        theme.highlight()
                    } else {
                        theme.text()
                    };
                    Row::new(vec![
                        Cell::from(Span::styled(format!("{:>3}", r.id.0), style)),
                        Cell::from(Span::styled(r.status.clone(), style)),
                        Cell::from(Span::styled(format!("{:>2}", r.round), style)),
                        Cell::from(Span::styled(r.teams.clone(), style)),
                        Cell::from(Span::styled(shorten(&r.verdict, 22), style)),
                    ])
                })
                .collect();
            let title = format!(" {} ({}) ", t(lang, T::TradesMyProposals), rows.len());
            let table = Table::new(
                body,
                [
                    Constraint::Length(5),
                    Constraint::Length(10),
                    Constraint::Length(7),
                    Constraint::Percentage(35),
                    Constraint::Percentage(40),
                ],
            )
            .header(header)
            .block(theme.block(&title));
            f.render_widget(table, parts[0]);
        }

        ActionBar::new(&[
            ("a", t(lang, T::TradesAccept)),
            ("r", t(lang, T::TradesReject)),
            ("c", t(lang, T::TradesCounter)),
            ("Enter", t(lang, T::CommonDetail)),
            ("Tab", t(lang, T::CommonTabs)),
            ("Esc", t(lang, T::CommonBack)),
        ])
        .render(f, parts[1], theme);
    });
}

fn draw_builder(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let parts = body_with_bar(area);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(24),
                Constraint::Percentage(32),
                Constraint::Percentage(32),
                Constraint::Percentage(12),
            ])
            .split(parts[0]);

        draw_team_picker(f, cols[0], theme, tui.lang, &cache);
        let (incoming_title, incoming_rows, incoming_selected) = incoming_view(&cache);
        draw_player_picker(
            f,
            cols[1],
            theme,
            tui.lang,
            t(tui.lang, T::TradesYouSend),
            cache.user_roster.as_deref().unwrap_or(&[]),
            cache.out_cursor,
            &cache.selected_out,
            cache.builder_panel == BuilderPanel::Outgoing,
        );
        draw_player_picker(
            f,
            cols[2],
            theme,
            tui.lang,
            &incoming_title,
            incoming_rows,
            cache.in_cursor,
            incoming_selected,
            cache.builder_panel == BuilderPanel::Incoming,
        );
        draw_builder_submit(f, cols[3], theme, &cache, tui);

        ActionBar::new(&[
            ("<- ->", t(tui.lang, T::CommonNavigate)),
            ("Up/Down", t(tui.lang, T::CommonMove)),
            ("Space", t(tui.lang, T::CommonPick)),
            ("m", t(tui.lang, T::TradesToggleTeamMode)),
            ("i", t(tui.lang, T::TradesSwapIncomingTeam)),
            ("Enter", t(tui.lang, T::CommonSubmit)),
            ("p", t(tui.lang, T::TradesPropose)),
            ("Tab", t(tui.lang, T::CommonTabs)),
        ])
        .render(f, parts[1], theme);
    });
}

fn draw_team_picker(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, cache: &TradesCache) {
    let teams = cache.teams.as_deref().unwrap_or(&[]);
    let cursor = cache.team_cursor.min(teams.len().saturating_sub(1));
    let title = if cache.builder_panel == BuilderPanel::Team {
        match cache.builder_mode {
            BuilderMode::TwoTeam => format!(" {} > ", t(lang, T::NewGameTeam)),
            BuilderMode::ThreeTeam => format!(" {} > ", t(lang, T::NewGameTeam)),
        }
    } else {
        match cache.builder_mode {
            BuilderMode::TwoTeam => format!(" {} ", t(lang, T::NewGameTeam)),
            BuilderMode::ThreeTeam => format!(" {} ", t(lang, T::NewGameTeam)),
        }
    };
    let lines: Vec<Line> = teams
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let first = cache.target_team == Some(t.id);
            let second = cache.third_team == Some(t.id);
            let focus = i == cursor && cache.builder_panel == BuilderPanel::Team;
            let style = if focus {
                theme.highlight()
            } else if first || second {
                theme.accent_style()
            } else {
                theme.text()
            };
            let mark = if first {
                "1"
            } else if second {
                "2"
            } else {
                " "
            };
            Line::from(Span::styled(
                format!("{} {:<4} {}", mark, t.abbrev, shorten(&t.name, 18)),
                style,
            ))
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(theme.block(&title)), area);
}

fn incoming_view(cache: &TradesCache) -> (String, &[PlayerOption], &HashSet<PlayerId>) {
    if cache.builder_mode == BuilderMode::ThreeTeam && cache.incoming_slot == IncomingSlot::Second {
        (
            " T2 -> ".to_string(),
            cache.third_roster.as_deref().unwrap_or(&[]),
            &cache.selected_third,
        )
    } else {
        (
            " T1 -> ".to_string(),
            cache.target_roster.as_deref().unwrap_or(&[]),
            &cache.selected_in,
        )
    }
}

fn draw_player_picker(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    lang: Lang,
    title: &str,
    rows: &[PlayerOption],
    cursor: usize,
    selected: &HashSet<PlayerId>,
    focused: bool,
) {
    let cursor = cursor.min(rows.len().saturating_sub(1));
    let header = Row::new(vec![
        head("", theme),
        head(t(lang, T::RosterPlayer), theme),
        head(t(lang, T::RosterPosition), theme),
        head(t(lang, T::RosterOverall), theme),
        head(t(lang, T::RosterSalary), theme),
    ]);
    let body: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if focused && i == cursor {
                theme.highlight()
            } else {
                theme.text()
            };
            let mark = if selected.contains(&p.id) { "x" } else { " " };
            Row::new(vec![
                Cell::from(Span::styled(mark.to_string(), style)),
                Cell::from(Span::styled(shorten(&p.name, 22), style)),
                Cell::from(Span::styled(format!("{}", p.position), style)),
                Cell::from(Span::styled(format!("{}", p.overall), style)),
                Cell::from(Span::styled(format!("{:.1}", p.salary_m), style)),
            ])
        })
        .collect();
    let title = if focused {
        format!("{}> ", title)
    } else {
        title.to_string()
    };
    let table = Table::new(
        body,
        [
            Constraint::Length(3),
            Constraint::Min(14),
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(6),
        ],
    )
    .header(header)
    .block(theme.block(&title));
    f.render_widget(table, area);
}

fn draw_builder_submit(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    cache: &TradesCache,
    tui: &TuiApp,
) {
    let panel_style = if cache.builder_panel == BuilderPanel::Submit {
        theme.highlight()
    } else {
        theme.text()
    };
    let target = cache
        .teams
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .find(|t| Some(t.id) == cache.target_team)
        .map(|t| t.abbrev.clone())
        .unwrap_or_else(|| "???".into());
    let third = cache
        .teams
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .find(|t| Some(t.id) == cache.third_team)
        .map(|t| t.abbrev.clone())
        .unwrap_or_else(|| "???".into());
    let mode_label = match cache.builder_mode {
        BuilderMode::TwoTeam => format!("2 {}", t(tui.lang, T::TradesTitle)),
        BuilderMode::ThreeTeam => format!("3 {}", t(tui.lang, T::TradesTitle)),
    };
    let ready = match cache.builder_mode {
        BuilderMode::TwoTeam => !cache.selected_out.is_empty() && !cache.selected_in.is_empty(),
        BuilderMode::ThreeTeam => {
            cache.third_team.is_some()
                && !cache.selected_out.is_empty()
                && !cache.selected_in.is_empty()
                && !cache.selected_third.is_empty()
        }
    };
    let status = if ready {
        t(tui.lang, T::CommonReady)
    } else {
        t(tui.lang, T::TradesPickBothSides)
    };
    let mut lines = vec![
        Line::from(Span::styled(mode_label, theme.accent_style())),
        Line::from(""),
        Line::from(Span::styled(
            format!("From: {}", tui.user_abbrev),
            theme.text(),
        )),
        Line::from(Span::styled(format!("T1:   {}", target), theme.text())),
    ];
    if cache.builder_mode == BuilderMode::ThreeTeam {
        lines.push(Line::from(Span::styled(
            format!("T2:   {}", third),
            theme.text(),
        )));
    }
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            format!("Send: {}", cache.selected_out.len()),
            theme.text(),
        )),
        Line::from(Span::styled(
            format!("T1:   {}", cache.selected_in.len()),
            theme.text(),
        )),
    ]);
    if cache.builder_mode == BuilderMode::ThreeTeam {
        lines.push(Line::from(Span::styled(
            format!("T2:   {}", cache.selected_third.len()),
            theme.text(),
        )));
    }
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(status.to_string(), panel_style)),
        Line::from(""),
        Line::from(Span::styled(t(tui.lang, T::TradesToggleTeamMode), theme.muted_style())),
        Line::from(Span::styled(t(tui.lang, T::TradesSwapIncomingTeam), theme.muted_style())),
    ]);
    f.render_widget(
        Paragraph::new(lines)
            .block(theme.block(t(tui.lang, T::TradesSubmit)))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_rumors(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let rows = cache.rumor_rows.as_deref().unwrap_or(&[]);
        let cursor = cache.rumor_cursor.min(rows.len().saturating_sub(1));
        let parts = body_with_bar(area);

        if rows.is_empty() {
            let p = Paragraph::new(t(lang, T::TradesNoRumors)).block(theme.block(t(lang, T::TradesRumors)));
            f.render_widget(p, parts[0]);
        } else {
            let header = Row::new(vec![
                head("#", theme),
                head(t(lang, T::RosterPlayer), theme),
                head("TM", theme),
                head(t(lang, T::RosterOverall), theme),
                head(t(lang, T::RosterRole), theme),
                head("INT", theme),
                head("TOP SUITORS", theme),
            ]);
            let body: Vec<Row> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let style = if i == cursor {
                        theme.highlight()
                    } else {
                        theme.text()
                    };
                    Row::new(vec![
                        Cell::from(Span::styled(format!("{:>2}", i + 1), style)),
                        Cell::from(Span::styled(shorten(&r.player_name, 24), style)),
                        Cell::from(Span::styled(r.team_abbrev.clone(), style)),
                        Cell::from(Span::styled(format!("{}", r.ovr), style)),
                        Cell::from(Span::styled(short_role(lang, r.role), style)),
                        Cell::from(Span::styled(format!("{}", r.interest), style)),
                        Cell::from(Span::styled(
                            r.suitors
                                .iter()
                                .take(5)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", "),
                            style,
                        )),
                    ])
                })
                .collect();
            let title = format!(" {} ({}) ", t(lang, T::TradesRumors), rows.len());
            let table = Table::new(
                body,
                [
                    Constraint::Length(4),
                    Constraint::Percentage(28),
                    Constraint::Length(5),
                    Constraint::Length(5),
                    Constraint::Length(7),
                    Constraint::Length(5),
                    Constraint::Percentage(40),
                ],
            )
            .header(header)
            .block(theme.block(&title));
            f.render_widget(table, parts[0]);
        }

        ActionBar::new(&[
            ("Up/Down", t(lang, T::CommonMove)),
            ("Tab", t(lang, T::CommonTabs)),
            ("Esc", t(lang, T::CommonBack)),
        ])
            .render(f, parts[1], theme);
    });
}

fn draw_free_agents(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let rows = cache.fa_rows.as_deref().unwrap_or(&[]);
        let cursor = cache.fa_cursor.min(rows.len().saturating_sub(1));
        let parts = body_with_bar(area);

        if rows.is_empty() {
            let p = Paragraph::new(t(lang, T::RosterNoPlayers))
                .block(theme.block(t(lang, T::RosterFreeAgents)));
            f.render_widget(p, parts[0]);
        } else {
            let header = Row::new(vec![
                head("#", theme),
                head(t(lang, T::RosterPlayer), theme),
                head(t(lang, T::RosterPosition), theme),
                head(t(lang, T::RosterAge), theme),
                head(t(lang, T::RosterOverall), theme),
                head(t(lang, T::RosterSalary), theme),
            ]);
            let body: Vec<Row> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let style = if i == cursor {
                        theme.highlight()
                    } else {
                        theme.text()
                    };
                    Row::new(vec![
                        Cell::from(Span::styled(format!("{:>2}", i + 1), style)),
                        Cell::from(Span::styled(r.name.clone(), style)),
                        Cell::from(Span::styled(format!("{}", r.position), style)),
                        Cell::from(Span::styled(format!("{}", r.age), style)),
                        Cell::from(Span::styled(format!("{}", r.overall), style)),
                        Cell::from(Span::styled(format!("${:.1}M", r.asking_m), style)),
                    ])
                })
                .collect();
            let title = format!(" {} ({}) ", t(lang, T::RosterFreeAgents), rows.len());
            let table = Table::new(
                body,
                [
                    Constraint::Length(4),
                    Constraint::Percentage(45),
                    Constraint::Length(5),
                    Constraint::Length(5),
                    Constraint::Length(5),
                    Constraint::Length(8),
                ],
            )
            .header(header)
            .block(theme.block(&title));
            f.render_widget(table, parts[0]);
        }

        ActionBar::new(&[
            ("Up/Down", t(lang, T::CommonMove)),
            ("s", t(lang, T::CommonPick)),
            ("Tab", t(lang, T::CommonTabs)),
            ("Esc", t(lang, T::CommonBack)),
        ])
        .render(f, parts[1], theme);
    });
}

fn draw_modal(f: &mut Frame, rect: Rect, theme: &Theme, lang: Lang) {
    let _ = t(lang, T::ModalTradeVerdictTitle);
    let (title, lines) = CACHE.with(|c| {
        let cache = c.borrow();
        match &cache.modal {
            Modal::None => ("".to_string(), Vec::new()),
            Modal::OfferDetail { id } => {
                let lines = cache
                    .inbox_rows
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .find(|r| r.id == *id)
                    .map(|r| r.detail_lines.clone())
                    .unwrap_or_else(|| vec!["Offer not found.".into()]);
                (format!(" Offer #{} ", id.0), lines)
            }
            Modal::ChainDetail { id } => {
                let lines = cache
                    .chain_rows
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .find(|r| r.id == *id)
                    .map(|r| r.detail_lines.clone())
                    .unwrap_or_else(|| vec!["Chain not found.".into()]);
                (format!(" Trade Chain #{} ", id.0), lines)
            }
            Modal::Message { title, lines } => (format!(" {} ", title), lines.clone()),
        }
    });
    let text: Vec<Line> = lines
        .into_iter()
        .map(|s| Line::from(Span::styled(s, theme.text())))
        .collect();
    f.render_widget(
        Paragraph::new(text)
            .block(theme.block(&title))
            .wrap(Wrap { trim: false }),
        rect,
    );
}

fn body_with_bar(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area)
}

fn modal_rect(area: Rect) -> Rect {
    let w = area.width.saturating_sub(8).min(104).max(42);
    let h = area.height.saturating_sub(4).min(30).max(8);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

fn head(label: &str, theme: &Theme) -> Cell<'static> {
    Cell::from(Span::styled(label.to_string(), theme.accent_style()))
}

fn ensure_cache(app: &mut AppState, tui: &TuiApp) -> Result<()> {
    let need_snapshot_rows = CACHE.with(|c| {
        let c = c.borrow();
        c.inbox_rows.is_none() || c.chain_rows.is_none()
    });
    if need_snapshot_rows {
        let snap_owned = build_league_snapshot(app)?;
        let inbox = build_inbox_rows(app, tui, &snap_owned)?;
        let chains = build_chain_rows(app, tui, &snap_owned)?;
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.inbox_rows.is_none() {
                c.inbox_rows = Some(inbox);
            }
            if c.chain_rows.is_none() {
                c.chain_rows = Some(chains);
            }
        });
    }

    if CACHE.with(|c| c.borrow().rumor_rows.is_none()) {
        let rows = build_rumor_rows(app, tui.season)?;
        CACHE.with(|c| c.borrow_mut().rumor_rows = Some(rows));
    }

    if CACHE.with(|c| c.borrow().fa_rows.is_none()) {
        let rows = build_fa_rows(app)?;
        CACHE.with(|c| c.borrow_mut().fa_rows = Some(rows));
    }

    if CACHE.with(|c| c.borrow().teams.is_none()) {
        let teams = build_team_options(app, tui.user_team)?;
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.target_team.is_none() {
                c.target_team = teams.first().map(|t| t.id);
            }
            c.team_cursor = c
                .target_team
                .and_then(|id| teams.iter().position(|t| t.id == id))
                .unwrap_or(0);
            c.teams = Some(teams);
        });
    }

    if CACHE.with(|c| c.borrow().user_roster.is_none()) {
        let roster = build_roster_options(app, tui.user_team, tui.season)?;
        CACHE.with(|c| c.borrow_mut().user_roster = Some(roster));
    }

    if CACHE.with(|c| c.borrow().target_roster.is_none()) {
        let target = CACHE.with(|c| c.borrow().target_team);
        if let Some(team) = target {
            let roster = build_roster_options(app, team, tui.season)?;
            CACHE.with(|c| c.borrow_mut().target_roster = Some(roster));
        }
    }

    if CACHE.with(|c| c.borrow().third_roster.is_none()) {
        let third = CACHE.with(|c| c.borrow().third_team);
        if let Some(team) = third {
            let roster = build_roster_options(app, team, tui.season)?;
            CACHE.with(|c| c.borrow_mut().third_roster = Some(roster));
        }
    }
    Ok(())
}

fn build_inbox_rows(
    app: &mut AppState,
    tui: &TuiApp,
    snap_owned: &OwnedSnapshot,
) -> Result<Vec<OfferRow>> {
    let snap = snap_owned.view();
    let mut rng =
        ChaCha8Rng::seed_from_u64(tui.season.0 as u64 ^ tui.user_team.0 as u64 ^ 0xC0FFEE);
    let store = app.store()?;
    let chains = store.read_open_chains_targeting(tui.season, tui.user_team)?;
    let mut rows = Vec::with_capacity(chains.len());
    for (id, st) in chains {
        let NegotiationState::Open { chain } = st else {
            continue;
        };
        let Some(latest) = chain.last() else { continue };
        let from = latest.initiator;
        let from_abbrev = team_abbrev(store, from)?;
        let wants = render_players(store, players_out(latest, tui.user_team))?;
        let sends = render_players(store, players_out(latest, from))?;
        let evaluation = evaluate_mod::evaluate(latest, tui.user_team, &snap, &mut rng);
        let verdict = verdict_label(tui.lang, &evaluation.verdict).to_string();
        let mut detail_lines = offer_detail_lines(store, latest)?;
        detail_lines.push(String::new());
        detail_lines.push(format!(
            "Advisory verdict: {} ({:.0}%)",
            verdict,
            evaluation.confidence * 100.0
        ));
        if !evaluation.commentary.trim().is_empty() {
            detail_lines.push(format!("Commentary: {}", evaluation.commentary));
        }
        if let Verdict::Reject(reason) = &evaluation.verdict {
            detail_lines.push(format!(
                "Reject reason: {}",
                reject_reason_to_string(tui.lang, reason)
            ));
        }
        if let Verdict::Counter(counter) = &evaluation.verdict {
            detail_lines.push("Suggested counter:".to_string());
            detail_lines.extend(offer_detail_lines(store, counter)?);
        }
        rows.push(OfferRow {
            id,
            from,
            from_abbrev,
            wants,
            sends,
            verdict,
            probability: evaluation.confidence,
            commentary: evaluation.commentary,
            detail_lines,
        });
    }
    Ok(rows)
}

fn build_chain_rows(
    app: &mut AppState,
    tui: &TuiApp,
    snap_owned: &OwnedSnapshot,
) -> Result<Vec<ChainRow>> {
    let snap = snap_owned.view();
    let mut rng = ChaCha8Rng::seed_from_u64(tui.season.0 as u64 ^ 0x51ADE);
    let store = app.store()?;
    let chains = store.list_trade_chains(tui.season)?;
    let mut rows = Vec::new();
    for (id, st) in chains {
        if !state_involves_team(&st, tui.user_team) {
            continue;
        }
        let (status, open, latest, offers) = match &st {
            NegotiationState::Open { chain } => {
                ("open".to_string(), true, chain.last(), chain.as_slice())
            }
            NegotiationState::Accepted(o) => (
                "accepted".to_string(),
                false,
                Some(o),
                std::slice::from_ref(o),
            ),
            NegotiationState::Rejected {
                final_offer,
                reason: _,
            } => (
                "rejected".to_string(),
                false,
                Some(final_offer),
                std::slice::from_ref(final_offer),
            ),
            NegotiationState::Stalled => ("stalled".to_string(), false, None, &[][..]),
        };
        let round = latest.map(|o| o.round).unwrap_or(0);
        let teams = latest
            .map(|o| teams_for_offer(store, o))
            .transpose()?
            .unwrap_or_default();
        let verdict = match (&st, latest) {
            (NegotiationState::Open { .. }, Some(o)) => {
                let ev = evaluate_mod::evaluate(o, tui.user_team, &snap, &mut rng);
                format!(
                    "{} ({:.0}%)",
                    verdict_label(tui.lang, &ev.verdict),
                    ev.confidence * 100.0
                )
            }
            (NegotiationState::Accepted(_), _) => "accept".into(),
            (NegotiationState::Rejected { reason, .. }, _) => {
                format!(
                    "{} - {}",
                    t(tui.lang, T::TradesReject),
                    reject_reason_to_string(tui.lang, reason)
                )
            }
            (NegotiationState::Stalled, _) => "stalled".into(),
            _ => "unknown".into(),
        };
        let mut detail_lines = vec![format!("Status: {}", status), format!("Teams: {}", teams)];
        if let NegotiationState::Rejected { reason, .. } = &st {
            detail_lines.push(format!(
                "Reject reason: {}",
                reject_reason_to_string(tui.lang, reason)
            ));
        }
        detail_lines.push("Counter chain:".to_string());
        detail_lines.push(String::new());
        for (idx, offer) in offers.iter().enumerate() {
            detail_lines.push(format!("Offer {} / round {}", idx + 1, offer.round));
            detail_lines.extend(offer_detail_lines(store, offer)?);
            detail_lines.push(String::new());
        }
        rows.push(ChainRow {
            id,
            status,
            round,
            teams,
            verdict,
            open,
            detail_lines,
        });
    }
    Ok(rows)
}

fn build_rumor_rows(app: &mut AppState, season: SeasonId) -> Result<Vec<RumorRow>> {
    let store = app.store()?;
    let teams = store.list_teams()?;
    let players = store.all_active_players()?;
    let ly = LeagueYear::for_season(season)
        .ok_or_else(|| anyhow!("no LeagueYear constants for season {}", season.0))?;

    struct TeamCtx {
        id: TeamId,
        abbrev: String,
        archetypes: HashSet<String>,
        position_counts: HashMap<Position, u32>,
        cap_room_cents: i64,
    }

    let mut team_ctx: HashMap<TeamId, TeamCtx> = HashMap::new();
    for t in &teams {
        let mut roster = store.roster_for_team(t.id)?;
        roster.truncate(8);
        let mut archetypes = HashSet::new();
        let mut position_counts: HashMap<Position, u32> = HashMap::new();
        for p in &roster {
            archetypes.insert(infer_archetype(p));
            *position_counts.entry(p.primary_position).or_insert(0) += 1;
        }
        let payroll = store.team_salary(t.id, season)?;
        team_ctx.insert(
            t.id,
            TeamCtx {
                id: t.id,
                abbrev: t.abbrev.clone(),
                archetypes,
                position_counts,
                cap_room_cents: ly.apron_1.0 - payroll.0,
            },
        );
    }

    let mut rumors = Vec::new();
    for p in &players {
        let Some(player_team) = p.team else { continue };
        let archetype = infer_archetype(p);
        let first_year_cents = p
            .contract
            .as_ref()
            .map(|c| c.salary_for(season).0)
            .unwrap_or(0);
        let needed_room = first_year_cents / 2;
        let mut suitors: Vec<(String, f32)> = Vec::new();
        for ctx in team_ctx.values() {
            if ctx.id == player_team || ctx.cap_room_cents < needed_room {
                continue;
            }
            let score = if !ctx.archetypes.contains(&archetype) {
                1.0
            } else if ctx
                .position_counts
                .get(&p.primary_position)
                .copied()
                .unwrap_or(0)
                <= 1
            {
                0.5
            } else {
                0.0
            };
            if score >= 0.5 {
                suitors.push((ctx.abbrev.clone(), score));
            }
        }
        if suitors.is_empty() {
            continue;
        }
        suitors.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        rumors.push(RumorRow {
            player_name: clean_name(&p.name),
            team_abbrev: team_abbrev(store, player_team)?,
            ovr: p.overall,
            role: p.role,
            interest: suitors.len() as u32,
            suitors: suitors.into_iter().map(|(abbr, _)| abbr).collect(),
        });
    }
    rumors.sort_by(|a, b| {
        b.interest
            .cmp(&a.interest)
            .then_with(|| b.ovr.cmp(&a.ovr))
            .then_with(|| a.player_name.cmp(&b.player_name))
    });
    rumors.truncate(30);
    Ok(rumors)
}

fn build_fa_rows(app: &mut AppState) -> Result<Vec<FaRow>> {
    let store = app.store()?;
    let mut rows: Vec<FaRow> = store
        .list_free_agents()?
        .into_iter()
        .map(|p| FaRow {
            player_id: p.id,
            name: clean_name(&p.name),
            position: p.primary_position,
            age: p.age,
            overall: p.overall,
            asking_m: estimate_asking_m(p.overall),
        })
        .collect();
    rows.sort_by(|a, b| {
        b.overall
            .cmp(&a.overall)
            .then_with(|| a.age.cmp(&b.age))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(rows)
}

fn estimate_asking_m(overall: u8) -> f32 {
    match overall {
        90..=u8::MAX => 35.0,
        85..=89 => 25.0,
        80..=84 => 15.0,
        75..=79 => 8.0,
        70..=74 => 3.0,
        _ => 1.0,
    }
}

fn build_team_options(app: &mut AppState, user_team: TeamId) -> Result<Vec<TeamOption>> {
    let store = app.store()?;
    let teams = store.list_teams()?;
    Ok(teams
        .into_iter()
        .filter(|t| t.id != user_team)
        .map(|t| {
            let name = t.full_name();
            TeamOption {
                id: t.id,
                abbrev: t.abbrev,
                name,
            }
        })
        .collect())
}

fn build_roster_options(
    app: &mut AppState,
    team: TeamId,
    season: SeasonId,
) -> Result<Vec<PlayerOption>> {
    let store = app.store()?;
    Ok(store
        .roster_for_team(team)?
        .into_iter()
        .map(|p| {
            let salary_m = p
                .contract
                .as_ref()
                .map(|c| c.salary_for(season).as_millions_f32())
                .unwrap_or(0.0);
            PlayerOption {
                id: p.id,
                name: clean_name(&p.name),
                raw_name: p.name,
                position: p.primary_position,
                age: p.age,
                overall: p.overall,
                salary_m,
            }
        })
        .collect())
}

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    let modal_handled = CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        match &cache.modal {
            Modal::None => false,
            _ => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    cache.modal = Modal::None;
                }
                true
            }
        }
    });
    if modal_handled {
        return Ok(true);
    }

    match key.code {
        KeyCode::Tab => {
            CACHE.with(|c| {
                let tab = c.borrow().tab;
                c.borrow_mut().tab = next_tab(tab);
            });
            Ok(true)
        }
        KeyCode::Char('1') => set_tab(SubTab::Inbox),
        KeyCode::Char('2') => set_tab(SubTab::Proposals),
        KeyCode::Char('3') => set_tab(SubTab::Builder),
        KeyCode::Char('4') => set_tab(SubTab::Rumors),
        KeyCode::Char('5') => set_tab(SubTab::FreeAgents),
        _ => match CACHE.with(|c| c.borrow().tab) {
            SubTab::Inbox => handle_inbox_key(app, tui, key),
            SubTab::Proposals => handle_proposals_key(app, tui, key),
            SubTab::Builder => handle_builder_key(app, tui, key),
            SubTab::Rumors => handle_rumors_key(key),
            SubTab::FreeAgents => handle_free_agents_key(app, tui, key),
        },
    }
}

fn set_tab(tab: SubTab) -> Result<bool> {
    CACHE.with(|c| c.borrow_mut().tab = tab);
    Ok(true)
}

fn next_tab(tab: SubTab) -> SubTab {
    match tab {
        SubTab::Inbox => SubTab::Proposals,
        SubTab::Proposals => SubTab::Builder,
        SubTab::Builder => SubTab::Rumors,
        SubTab::Rumors => SubTab::FreeAgents,
        SubTab::FreeAgents => SubTab::Inbox,
    }
}

fn handle_inbox_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Up => move_inbox(-1),
        KeyCode::Down => move_inbox(1),
        KeyCode::PageUp => move_inbox(-10),
        KeyCode::PageDown => move_inbox(10),
        KeyCode::Enter => {
            if let Some(id) = current_offer_id() {
                CACHE.with(|c| c.borrow_mut().modal = Modal::OfferDetail { id });
            }
            Ok(true)
        }
        KeyCode::Char('a') => respond_current_inbox(app, tui, "accept"),
        KeyCode::Char('r') => respond_current_inbox(app, tui, "reject"),
        KeyCode::Char('c') => respond_current_inbox(app, tui, "counter"),
        _ => Ok(false),
    }
}

fn handle_proposals_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Up => move_chain(-1),
        KeyCode::Down => move_chain(1),
        KeyCode::PageUp => move_chain(-10),
        KeyCode::PageDown => move_chain(10),
        KeyCode::Enter => {
            if let Some(id) = current_chain_id() {
                CACHE.with(|c| c.borrow_mut().modal = Modal::ChainDetail { id });
            }
            Ok(true)
        }
        KeyCode::Char('a') => respond_current_chain(app, tui, "accept"),
        KeyCode::Char('r') => respond_current_chain(app, tui, "reject"),
        KeyCode::Char('c') => respond_current_chain(app, tui, "counter"),
        _ => Ok(false),
    }
}

fn handle_builder_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Left => {
            CACHE.with(|c| {
                let panel = c.borrow().builder_panel;
                c.borrow_mut().builder_panel = prev_panel(panel);
            });
            Ok(true)
        }
        KeyCode::Right => {
            CACHE.with(|c| {
                let panel = c.borrow().builder_panel;
                c.borrow_mut().builder_panel = next_panel(panel);
            });
            Ok(true)
        }
        KeyCode::Up => move_builder_cursor(-1),
        KeyCode::Down => move_builder_cursor(1),
        KeyCode::Char('m') => toggle_builder_mode(),
        KeyCode::Char('i') => cycle_incoming_slot(),
        KeyCode::Enter | KeyCode::Char(' ') => builder_activate(app, tui),
        KeyCode::Char('p') => submit_builder(app, tui),
        _ => Ok(false),
    }
}

fn handle_rumors_key(key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Up => move_rumor(-1),
        KeyCode::Down => move_rumor(1),
        KeyCode::PageUp => move_rumor(-10),
        KeyCode::PageDown => move_rumor(10),
        _ => Ok(false),
    }
}

fn handle_free_agents_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Up => move_fa(-1),
        KeyCode::Down => move_fa(1),
        KeyCode::PageUp => move_fa(-10),
        KeyCode::PageDown => move_fa(10),
        KeyCode::Char('s') => sign_current_free_agent(app, tui),
        _ => Ok(false),
    }
}

fn move_inbox(delta: isize) -> Result<bool> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let len = c.inbox_rows.as_ref().map(|r| r.len()).unwrap_or(0);
        c.inbox_cursor = moved(c.inbox_cursor, len, delta);
    });
    Ok(true)
}

fn move_chain(delta: isize) -> Result<bool> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let len = c.chain_rows.as_ref().map(|r| r.len()).unwrap_or(0);
        c.chain_cursor = moved(c.chain_cursor, len, delta);
    });
    Ok(true)
}

fn move_rumor(delta: isize) -> Result<bool> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let len = c.rumor_rows.as_ref().map(|r| r.len()).unwrap_or(0);
        c.rumor_cursor = moved(c.rumor_cursor, len, delta);
    });
    Ok(true)
}

fn move_fa(delta: isize) -> Result<bool> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let len = c.fa_rows.as_ref().map(|r| r.len()).unwrap_or(0);
        c.fa_cursor = moved(c.fa_cursor, len, delta);
    });
    Ok(true)
}

fn move_builder_cursor(delta: isize) -> Result<bool> {
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        match cache.builder_panel {
            BuilderPanel::Team => {
                let len = cache.teams.as_ref().map(|r| r.len()).unwrap_or(0);
                cache.team_cursor = moved(cache.team_cursor, len, delta);
            }
            BuilderPanel::Outgoing => {
                let len = cache.user_roster.as_ref().map(|r| r.len()).unwrap_or(0);
                cache.out_cursor = moved(cache.out_cursor, len, delta);
            }
            BuilderPanel::Incoming => {
                let len = if cache.builder_mode == BuilderMode::ThreeTeam
                    && cache.incoming_slot == IncomingSlot::Second
                {
                    cache.third_roster.as_ref().map(|r| r.len()).unwrap_or(0)
                } else {
                    cache.target_roster.as_ref().map(|r| r.len()).unwrap_or(0)
                };
                cache.in_cursor = moved(cache.in_cursor, len, delta);
            }
            BuilderPanel::Submit => {}
        }
    });
    Ok(true)
}

fn toggle_builder_mode() -> Result<bool> {
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache.builder_mode = match cache.builder_mode {
            BuilderMode::TwoTeam => BuilderMode::ThreeTeam,
            BuilderMode::ThreeTeam => BuilderMode::TwoTeam,
        };
        cache.incoming_slot = IncomingSlot::First;
        if cache.builder_mode == BuilderMode::TwoTeam {
            cache.third_team = None;
            cache.third_roster = None;
            cache.selected_third.clear();
        }
        cache.in_cursor = 0;
    });
    Ok(true)
}

fn cycle_incoming_slot() -> Result<bool> {
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.builder_mode == BuilderMode::ThreeTeam {
            cache.incoming_slot = match cache.incoming_slot {
                IncomingSlot::First => IncomingSlot::Second,
                IncomingSlot::Second => IncomingSlot::First,
            };
            cache.in_cursor = 0;
        }
    });
    Ok(true)
}

fn builder_activate(app: &mut AppState, tui: &mut TuiApp) -> Result<bool> {
    let action = CACHE.with(|c| {
        let cache = c.borrow();
        match cache.builder_panel {
            BuilderPanel::Team => cache.teams.as_ref().and_then(|teams| {
                teams
                    .get(cache.team_cursor)
                    .map(|t| BuilderAction::SetTeam(t.id))
            }),
            BuilderPanel::Outgoing => cache.user_roster.as_ref().and_then(|rows| {
                rows.get(cache.out_cursor)
                    .map(|p| BuilderAction::ToggleOut(p.id))
            }),
            BuilderPanel::Incoming => {
                if cache.builder_mode == BuilderMode::ThreeTeam
                    && cache.incoming_slot == IncomingSlot::Second
                {
                    cache.third_roster.as_ref().and_then(|rows| {
                        rows.get(cache.in_cursor)
                            .map(|p| BuilderAction::ToggleThird(p.id))
                    })
                } else {
                    cache.target_roster.as_ref().and_then(|rows| {
                        rows.get(cache.in_cursor)
                            .map(|p| BuilderAction::ToggleIn(p.id))
                    })
                }
            }
            BuilderPanel::Submit => Some(BuilderAction::Submit),
        }
    });
    match action {
        Some(BuilderAction::SetTeam(team)) => {
            let assignment = CACHE.with(|c| {
                let mut cache = c.borrow_mut();
                let assignment = select_builder_team(&mut cache, team);
                cache.in_cursor = 0;
                assignment
            });
            match assignment {
                TeamAssignment::First(team) => {
                    let roster = build_roster_options(app, team, tui.season)?;
                    CACHE.with(|c| c.borrow_mut().target_roster = Some(roster));
                }
                TeamAssignment::Second(team) => {
                    let roster = build_roster_options(app, team, tui.season)?;
                    CACHE.with(|c| c.borrow_mut().third_roster = Some(roster));
                }
                TeamAssignment::None => {}
            }
            Ok(true)
        }
        Some(BuilderAction::ToggleOut(pid)) => {
            CACHE.with(|c| toggle(&mut c.borrow_mut().selected_out, pid));
            Ok(true)
        }
        Some(BuilderAction::ToggleIn(pid)) => {
            CACHE.with(|c| toggle(&mut c.borrow_mut().selected_in, pid));
            Ok(true)
        }
        Some(BuilderAction::ToggleThird(pid)) => {
            CACHE.with(|c| toggle(&mut c.borrow_mut().selected_third, pid));
            Ok(true)
        }
        Some(BuilderAction::Submit) => submit_builder(app, tui),
        None => Ok(true),
    }
}

enum BuilderAction {
    SetTeam(TeamId),
    ToggleOut(PlayerId),
    ToggleIn(PlayerId),
    ToggleThird(PlayerId),
    Submit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TeamAssignment {
    First(TeamId),
    Second(TeamId),
    None,
}

fn select_builder_team(cache: &mut TradesCache, team: TeamId) -> TeamAssignment {
    match cache.builder_mode {
        BuilderMode::TwoTeam => {
            cache.target_team = Some(team);
            cache.target_roster = None;
            cache.selected_in.clear();
            TeamAssignment::First(team)
        }
        BuilderMode::ThreeTeam => {
            if cache.target_team == Some(team) {
                cache.target_team = None;
                cache.target_roster = None;
                cache.selected_in.clear();
                if cache.incoming_slot == IncomingSlot::First {
                    cache.incoming_slot = IncomingSlot::Second;
                }
                TeamAssignment::None
            } else if cache.third_team == Some(team) {
                cache.third_team = None;
                cache.third_roster = None;
                cache.selected_third.clear();
                if cache.incoming_slot == IncomingSlot::Second {
                    cache.incoming_slot = IncomingSlot::First;
                }
                TeamAssignment::None
            } else if cache.target_team.is_none() {
                cache.target_team = Some(team);
                cache.target_roster = None;
                cache.selected_in.clear();
                cache.incoming_slot = IncomingSlot::First;
                TeamAssignment::First(team)
            } else {
                cache.third_team = Some(team);
                cache.third_roster = None;
                cache.selected_third.clear();
                cache.incoming_slot = IncomingSlot::Second;
                TeamAssignment::Second(team)
            }
        }
    }
}

fn submit_builder(app: &mut AppState, tui: &mut TuiApp) -> Result<bool> {
    let payload = CACHE.with(|c| {
        let cache = c.borrow();
        let first = cache
            .teams
            .as_ref()
            .and_then(|teams| teams.iter().find(|t| Some(t.id) == cache.target_team))
            .cloned();
        let second = cache
            .teams
            .as_ref()
            .and_then(|teams| teams.iter().find(|t| Some(t.id) == cache.third_team))
            .cloned();
        let send = selected_names(
            cache.user_roster.as_deref().unwrap_or(&[]),
            &cache.selected_out,
        );
        let first_send = selected_names(
            cache.target_roster.as_deref().unwrap_or(&[]),
            &cache.selected_in,
        );
        let second_send = selected_names(
            cache.third_roster.as_deref().unwrap_or(&[]),
            &cache.selected_third,
        );
        (
            cache.builder_mode,
            first,
            second,
            send,
            first_send,
            second_send,
        )
    });

    let (mode, first, second, send, first_send, second_send) = payload;
    match mode {
        BuilderMode::TwoTeam => {
            let Some(target) = first else {
                tui.last_msg = Some("pick a trade partner first".into());
                return Ok(true);
            };
            if send.is_empty() || first_send.is_empty() {
                tui.last_msg = Some("pick at least one player from each side".into());
                return Ok(true);
            }
            let cmd = Command::Trade(TradeArgs {
                action: TradeAction::Propose {
                    from: tui.user_abbrev.clone(),
                    to: target.abbrev.clone(),
                    send,
                    receive: first_send,
                    json: false,
                },
            });
            let res = with_silenced_io(|| dispatch(app, cmd));
            after_trade_mutation(tui, res, &format!("proposed trade with {}", target.abbrev));
        }
        BuilderMode::ThreeTeam => {
            let (Some(first), Some(second)) = (first, second) else {
                tui.last_msg = Some("pick two trade partners for a 3-team proposal".into());
                return Ok(true);
            };
            let legs = match build_propose3_legs(
                &tui.user_abbrev,
                &send,
                &first.abbrev,
                &first_send,
                &second.abbrev,
                &second_send,
            ) {
                Ok(legs) => legs,
                Err(e) => {
                    tui.last_msg = Some(e.to_string());
                    return Ok(true);
                }
            };
            let cmd = Command::Trade(TradeArgs {
                action: TradeAction::Propose3 {
                    leg: legs,
                    json: false,
                },
            });
            let res = with_silenced_io(|| dispatch(app, cmd));
            after_trade_mutation(
                tui,
                res,
                &format!(
                    "proposed 3-team trade with {}/{}",
                    first.abbrev, second.abbrev
                ),
            );
        }
    }
    Ok(true)
}

fn build_propose3_legs(
    user_abbrev: &str,
    user_sends: &[String],
    first_abbrev: &str,
    first_sends: &[String],
    second_abbrev: &str,
    second_sends: &[String],
) -> Result<Vec<String>> {
    if user_sends.is_empty() || first_sends.is_empty() || second_sends.is_empty() {
        return Err(anyhow!("pick at least one player from each team"));
    }
    if user_abbrev.eq_ignore_ascii_case(first_abbrev)
        || user_abbrev.eq_ignore_ascii_case(second_abbrev)
        || first_abbrev.eq_ignore_ascii_case(second_abbrev)
    {
        return Err(anyhow!("3-team proposal needs three distinct teams"));
    }
    Ok(vec![
        format!("{}:{}", user_abbrev, user_sends.join(",")),
        format!("{}:{}", first_abbrev, first_sends.join(",")),
        format!("{}:{}", second_abbrev, second_sends.join(",")),
    ])
}

fn selected_names(rows: &[PlayerOption], selected: &HashSet<PlayerId>) -> Vec<String> {
    rows.iter()
        .filter(|p| selected.contains(&p.id))
        .map(|p| p.raw_name.clone())
        .collect()
}

fn respond_current_inbox(app: &mut AppState, tui: &mut TuiApp, action: &str) -> Result<bool> {
    let Some(id) = current_offer_id() else {
        tui.last_msg = Some("no open inbox offer selected".into());
        return Ok(true);
    };
    respond_to_chain(app, tui, id, action)
}

fn respond_current_chain(app: &mut AppState, tui: &mut TuiApp, action: &str) -> Result<bool> {
    let selected = CACHE.with(|c| {
        let cache = c.borrow();
        cache
            .chain_rows
            .as_ref()
            .and_then(|rows| rows.get(cache.chain_cursor))
            .map(|r| (r.id, r.open))
    });
    let Some((id, open)) = selected else {
        tui.last_msg = Some("no trade chain selected".into());
        return Ok(true);
    };
    if !open {
        tui.last_msg = Some("selected chain is not open".into());
        return Ok(true);
    }
    respond_to_chain(app, tui, id, action)
}

fn respond_to_chain(
    app: &mut AppState,
    tui: &mut TuiApp,
    id: TradeId,
    action: &str,
) -> Result<bool> {
    let cmd = Command::Trade(TradeArgs {
        action: TradeAction::Respond {
            id: id.0,
            action: action.into(),
            json: false,
        },
    });
    let res = with_silenced_io(|| dispatch(app, cmd));
    after_trade_mutation(tui, res, &format!("trade #{}: {}", id.0, action));
    Ok(true)
}

fn sign_current_free_agent(app: &mut AppState, tui: &mut TuiApp) -> Result<bool> {
    let Some(row) = current_fa_row() else {
        tui.last_msg = Some("no free agent selected".into());
        return Ok(true);
    };

    let roster_full = app
        .store()
        .ok()
        .and_then(|s| s.roster_for_team(tui.user_team).ok())
        .map(|r| r.len() >= FA_ROSTER_CAP)
        .unwrap_or(false);
    if roster_full {
        tui.last_msg = Some(format!(
            "roster full ({}/{}), cut a player first",
            FA_ROSTER_CAP, FA_ROSTER_CAP
        ));
        return Ok(true);
    }

    let target_name = row.name;
    let res = with_silenced_io(|| {
        dispatch(
            app,
            Command::Fa(FaArgs {
                action: FaAction::Sign {
                    player: target_name.clone(),
                },
            }),
        )
    });
    after_fa_mutation(tui, res, &format!("signed {}", target_name));
    Ok(true)
}

fn after_trade_mutation(tui: &mut TuiApp, res: Result<()>, success_msg: &str) {
    match res {
        Ok(()) => {
            tui.last_msg = Some(success_msg.into());
            CACHE.with(|c| {
                let mut cache = c.borrow_mut();
                cache.selected_out.clear();
                cache.selected_in.clear();
                cache.selected_third.clear();
            });
        }
        Err(e) => tui.last_msg = Some(format!("trade error: {}", e)),
    }
    invalidate();
    tui.invalidate_caches();
    crate::tui::screens::home::invalidate();
    crate::tui::screens::roster::invalidate();
    crate::tui::screens::rotation::invalidate();
}

fn after_fa_mutation(tui: &mut TuiApp, res: Result<()>, success_msg: &str) {
    match res {
        Ok(()) => tui.last_msg = Some(success_msg.into()),
        Err(e) => tui.last_msg = Some(format!("free agent error: {}", e)),
    }
    invalidate();
    tui.invalidate_caches();
    crate::tui::screens::home::invalidate();
    crate::tui::screens::roster::invalidate();
    crate::tui::screens::rotation::invalidate();
}

fn current_offer_id() -> Option<TradeId> {
    CACHE.with(|c| {
        let cache = c.borrow();
        cache
            .inbox_rows
            .as_ref()
            .and_then(|rows| rows.get(cache.inbox_cursor))
            .map(|r| r.id)
    })
}

fn current_chain_id() -> Option<TradeId> {
    CACHE.with(|c| {
        let cache = c.borrow();
        cache
            .chain_rows
            .as_ref()
            .and_then(|rows| rows.get(cache.chain_cursor))
            .map(|r| r.id)
    })
}

fn current_fa_row() -> Option<FaRow> {
    CACHE.with(|c| {
        let cache = c.borrow();
        cache
            .fa_rows
            .as_ref()
            .and_then(|rows| rows.get(cache.fa_cursor))
            .cloned()
    })
}

fn prev_panel(panel: BuilderPanel) -> BuilderPanel {
    match panel {
        BuilderPanel::Team => BuilderPanel::Submit,
        BuilderPanel::Outgoing => BuilderPanel::Team,
        BuilderPanel::Incoming => BuilderPanel::Outgoing,
        BuilderPanel::Submit => BuilderPanel::Incoming,
    }
}

fn next_panel(panel: BuilderPanel) -> BuilderPanel {
    match panel {
        BuilderPanel::Team => BuilderPanel::Outgoing,
        BuilderPanel::Outgoing => BuilderPanel::Incoming,
        BuilderPanel::Incoming => BuilderPanel::Submit,
        BuilderPanel::Submit => BuilderPanel::Team,
    }
}

fn moved(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let max = len.saturating_sub(1) as isize;
    (current as isize + delta).clamp(0, max) as usize
}

fn toggle(set: &mut HashSet<PlayerId>, id: PlayerId) {
    if !set.insert(id) {
        set.remove(&id);
    }
}

struct OwnedSnapshot {
    teams: Vec<Team>,
    players_by_id: HashMap<PlayerId, Player>,
    picks_by_id: HashMap<DraftPickId, DraftPick>,
    standings: HashMap<TeamId, TeamRecordSummary>,
    season: SeasonId,
    phase: SeasonPhase,
    date: NaiveDate,
    league_year: LeagueYear,
}

impl OwnedSnapshot {
    fn view(&self) -> LeagueSnapshot<'_> {
        LeagueSnapshot {
            current_season: self.season,
            current_phase: self.phase,
            current_date: self.date,
            league_year: self.league_year,
            teams: &self.teams,
            players_by_id: &self.players_by_id,
            picks_by_id: &self.picks_by_id,
            standings: &self.standings,
        }
    }
}

fn build_league_snapshot(app: &mut AppState) -> Result<OwnedSnapshot> {
    let store = app.store()?;
    let state = store
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season_state in save"))?;
    let teams = store.list_teams()?;
    let players = store.all_active_players()?;
    let picks = store.all_picks()?;
    let standing_rows = store.read_standings(state.season)?;
    let players_by_id = players.into_iter().map(|p| (p.id, p)).collect();
    let picks_by_id = picks.into_iter().map(|p| (p.id, p)).collect();
    let mut standings = HashMap::new();
    for (i, r) in standing_rows.iter().enumerate() {
        standings.insert(
            r.team,
            TeamRecordSummary {
                wins: r.wins,
                losses: r.losses,
                conf_rank: r.conf_rank.unwrap_or((i as u8) + 1),
                point_diff: 0,
            },
        );
    }
    for t in &teams {
        standings
            .entry(t.id)
            .or_insert_with(TeamRecordSummary::default);
    }
    let league_year = LeagueYear::for_season(state.season)
        .ok_or_else(|| anyhow!("no LeagueYear constants for season {}", state.season.0))?;
    Ok(OwnedSnapshot {
        teams,
        players_by_id,
        picks_by_id,
        standings,
        season: state.season,
        phase: state.phase,
        date: commands::day_index_to_date(state.day),
        league_year,
    })
}

fn state_involves_team(state: &NegotiationState, team: TeamId) -> bool {
    match state {
        NegotiationState::Open { chain } => chain.iter().any(|o| offer_involves_team(o, team)),
        NegotiationState::Accepted(o) => offer_involves_team(o, team),
        NegotiationState::Rejected { final_offer, .. } => offer_involves_team(final_offer, team),
        NegotiationState::Stalled => false,
    }
}

fn offer_involves_team(offer: &TradeOffer, team: TeamId) -> bool {
    offer.initiator == team || offer.assets_by_team.contains_key(&team)
}

fn players_out(offer: &TradeOffer, team: TeamId) -> &[PlayerId] {
    offer
        .assets_by_team
        .get(&team)
        .map(|a| a.players_out.as_slice())
        .unwrap_or(&[])
}

fn offer_detail_lines(store: &nba3k_store::Store, offer: &TradeOffer) -> Result<Vec<String>> {
    let mut lines = vec![
        format!("Round: {}", offer.round),
        format!("Initiator: {}", team_abbrev(store, offer.initiator)?),
        String::new(),
    ];
    for (team, assets) in &offer.assets_by_team {
        let abbrev = team_abbrev(store, *team)?;
        let players = render_players(store, &assets.players_out)?;
        lines.push(format!("{} sends: {}", abbrev, players));
    }
    Ok(lines)
}

fn teams_for_offer(store: &nba3k_store::Store, offer: &TradeOffer) -> Result<String> {
    offer
        .assets_by_team
        .keys()
        .map(|t| team_abbrev(store, *t))
        .collect::<Result<Vec<_>>>()
        .map(|v| v.join("/"))
}

fn render_players(store: &nba3k_store::Store, pids: &[PlayerId]) -> Result<String> {
    if pids.is_empty() {
        return Ok("(nothing)".into());
    }
    let names = pids
        .iter()
        .map(|pid| {
            store.player_name(*pid).map(|opt| {
                opt.map(|n| clean_name(&n))
                    .unwrap_or_else(|| format!("#{}", pid.0))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names.join(", "))
}

fn team_abbrev(store: &nba3k_store::Store, team: TeamId) -> Result<String> {
    Ok(store
        .team_abbrev(team)?
        .unwrap_or_else(|| format!("T{}", team.0)))
}

fn verdict_label(lang: Lang, v: &Verdict) -> &'static str {
    match v {
        Verdict::Accept => t(lang, T::TradesAccept),
        Verdict::Reject(_) => t(lang, T::TradesReject),
        Verdict::Counter(_) => t(lang, T::TradesCounter),
    }
}

fn reject_reason_to_string(lang: Lang, r: &RejectReason) -> String {
    match r {
        RejectReason::InsufficientValue => t(lang, T::TradesInsufficientValue).to_string(),
        RejectReason::CbaViolation(s) => format!("CBA: {}", s),
        RejectReason::NoTradeClause(pid) => format!("no-trade clause (player #{})", pid.0),
        RejectReason::BadFaith => t(lang, T::TradesReject).to_string(),
        RejectReason::OutOfRoundCap => t(lang, T::TradesReject).to_string(),
        RejectReason::Other(s) => s.clone(),
    }
}

fn short_role(lang: Lang, r: PlayerRole) -> &'static str {
    match r {
        PlayerRole::Star => t(lang, T::RoleStar),
        PlayerRole::Starter => t(lang, T::RoleStarter),
        PlayerRole::SixthMan => t(lang, T::RoleSixthMan),
        PlayerRole::RolePlayer => t(lang, T::RoleRolePlayer),
        PlayerRole::BenchWarmer => t(lang, T::RoleBenchWarmer),
        PlayerRole::Prospect => t(lang, T::RoleProspect),
    }
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(3);
    format!("{}...", s.chars().take(keep).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_builds_three_team_propose_legs_in_trade_order() {
        let legs = build_propose3_legs(
            "BOS",
            &["Jayson Tatum".to_string()],
            "LAL",
            &["LeBron James".to_string()],
            "DAL",
            &["Anthony Davis".to_string()],
        )
        .expect("valid 3-team legs");

        assert_eq!(
            legs,
            vec![
                "BOS:Jayson Tatum".to_string(),
                "LAL:LeBron James".to_string(),
                "DAL:Anthony Davis".to_string(),
            ]
        );
    }

    #[test]
    fn builder_rejects_incomplete_or_duplicate_three_team_legs() {
        let empty = build_propose3_legs(
            "BOS",
            &["Jayson Tatum".to_string()],
            "LAL",
            &[],
            "DAL",
            &["Anthony Davis".to_string()],
        );
        assert!(empty.is_err());

        let duplicate = build_propose3_legs(
            "BOS",
            &["Jayson Tatum".to_string()],
            "bos",
            &["Jaylen Brown".to_string()],
            "DAL",
            &["Anthony Davis".to_string()],
        );
        assert!(duplicate.is_err());
    }
}
