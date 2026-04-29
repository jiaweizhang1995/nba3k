//! Trades screen (M22).
//!
//! Four sub-tabs:
//! - Inbox: AI offers targeting the user's team.
//! - My Proposals: user-involved negotiation chains.
//! - Builder: 2-team or 3-team player-for-player proposal flow.
//! - Free Agents: signable FA pool.
//!
//! All mutations route through `commands::dispatch` behind `with_silenced_io`
//! so command output cannot corrupt the alt-screen.

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Cell, Clear, Paragraph, Row, Table, Wrap},
    Frame,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::cli::{Command, FaAction, FaArgs, TradeAction, TradeArgs};
use crate::commands::{self, dispatch};
use crate::state::AppState;
use crate::tui::widgets::{ActionBar, FormWidget, Picker, Theme, WidgetEvent};
use crate::tui::{with_silenced_io, TuiApp};
use indexmap::IndexMap;
use nba3k_core::{
    t, Cents, DraftPick, DraftPickId, Lang, LeagueSnapshot, LeagueYear, NegotiationState, Player,
    PlayerId, Position, RejectReason, SeasonId, SeasonPhase, Team, TeamId, TeamRecordSummary,
    TradeAssets, TradeId, TradeOffer, Verdict, T,
};
use nba3k_trade::cba::{self, CbaViolation};
use nba3k_trade::evaluate as evaluate_mod;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SubTab {
    #[default]
    Inbox,
    Proposals,
    Builder,
    FreeAgents,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BuilderPanel {
    #[default]
    Incoming,
    Outgoing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BuilderStep {
    #[default]
    PickTeam,
    Compose,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TradeResponse {
    Accept,
    Reject,
    Counter,
}

impl TradeResponse {
    fn as_command_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
            Self::Counter => "counter",
        }
    }

    fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::Accept => t(lang, T::TradesAccept),
            Self::Reject => t(lang, T::TradesReject),
            Self::Counter => t(lang, T::TradesCounter),
        }
    }
}

#[derive(Clone, Debug)]
struct OfferRow {
    id: TradeId,
    from: TeamId,
    from_abbrev: String,
    wants: String,
    sends: String,
    verdict: String,
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
    wins: u32,
    losses: u32,
    payroll_m: f32,
    cap_m: f32,
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
    years: u32,
}

// Roster cap mirrors `commands::FA_ROSTER_CAP` (15 std + 3 two-way = 18).
const FA_ROSTER_CAP: usize = 18;

#[derive(Default)]
struct TradesCache {
    inbox_rows: Option<Vec<OfferRow>>,
    chain_rows: Option<Vec<ChainRow>>,
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
    fa_cursor: usize,

    builder_panel: BuilderPanel,
    builder_step: BuilderStep,
    builder_mode: BuilderMode,
    incoming_slot: IncomingSlot,
    team_cursor: usize,
    out_cursor: usize,
    in_cursor: usize,
    selected_out: HashSet<PlayerId>,
    selected_in: HashSet<PlayerId>,
    selected_third: HashSet<PlayerId>,
    gm_dialog: Option<String>,
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
    OfferAction {
        id: TradeId,
        picker: Picker<TradeResponse>,
    },
    ChainAction {
        id: TradeId,
        picker: Picker<TradeResponse>,
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
        SubTab::Builder => draw_builder(f, parts[1], theme, app, tui),
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
        Span::styled(
            format!(" 1. {} ", t(lang, T::TradesInbox)),
            style(SubTab::Inbox),
        ),
        Span::styled("   ", theme.text()),
        Span::styled(
            format!(" 2. {} ", t(lang, T::TradesMyProposals)),
            style(SubTab::Proposals),
        ),
        Span::styled("   ", theme.text()),
        Span::styled(
            format!(" 3. {} ", t(lang, T::TradesBuilder)),
            style(SubTab::Builder),
        ),
        Span::styled("   ", theme.text()),
        Span::styled(
            format!(" 4. {} ", t(lang, T::RosterFreeAgents)),
            style(SubTab::FreeAgents),
        ),
    ]);
    f.render_widget(
        Paragraph::new(line).block(theme.block(t(lang, T::TradesTitle))),
        area,
    );
}

fn draw_inbox(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let rows = cache.inbox_rows.as_deref().unwrap_or(&[]);
        let cursor = cache.inbox_cursor.min(rows.len().saturating_sub(1));
        let parts = body_with_bar(area);

        if rows.is_empty() {
            let p = Paragraph::new(t(lang, T::TradesIncomingOffersNone))
                .block(theme.block(t(lang, T::TradesInbox)));
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
            ("Enter", t(lang, T::TradesActionPickerTitle)),
            ("a/r/c", quick_label(lang)),
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
            ("Enter", t(lang, T::TradesActionPickerTitle)),
            ("a/r/c", quick_label(lang)),
            ("Tab", t(lang, T::CommonTabs)),
            ("Esc", t(lang, T::CommonBack)),
        ])
        .render(f, parts[1], theme);
    });
}

fn draw_builder(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    let step = CACHE.with(|c| c.borrow().builder_step);
    match step {
        BuilderStep::PickTeam => draw_pick_team(f, area, theme, app, tui),
        BuilderStep::Compose => {
            let verdict = build_verdict_view(app, tui);
            CACHE.with(|c| {
                let cache = c.borrow();
                draw_builder_compose(f, area, theme, tui, &cache, verdict);
            });
        }
    }
}

fn draw_pick_team(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    let (teams, cursor) = CACHE.with(|c| {
        let cache = c.borrow();
        (cache.teams.clone().unwrap_or_default(), cache.team_cursor)
    });
    let parts = body_with_bar(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(20)])
        .split(parts[0]);
    let cursor = cursor.min(teams.len().saturating_sub(1));
    let lines: Vec<Line> = teams
        .iter()
        .enumerate()
        .map(|(i, team)| {
            let style = if i == cursor {
                theme.highlight()
            } else {
                theme.text()
            };
            Line::from(Span::styled(
                format!("{:<3} {}", team.abbrev, shorten(&team.name, 18)),
                style,
            ))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(theme.block(t(tui.lang, T::TradesPickTeamTitle))),
        cols[0],
    );

    let mut preview = Vec::new();
    if let Some(team) = teams.get(cursor) {
        preview.push(Line::from(Span::styled(
            format!(
                "{} ({}-{}, ${:.1}M)",
                team.name, team.wins, team.losses, team.payroll_m
            ),
            theme.accent_style(),
        )));
        preview.push(Line::from(""));
        let roster = build_roster_options(app, team.id, tui.season).unwrap_or_default();
        for p in roster.iter().take(12) {
            preview.push(Line::from(Span::styled(
                format!(
                    "{:<2} {} {:>2} OVR {:>7} {:>3}",
                    p.position,
                    pad_display(&p.name, 18),
                    p.overall,
                    money_m(p.salary_m),
                    years_label(p.years)
                ),
                theme.text(),
            )));
        }
        preview.push(Line::from(""));
        preview.push(Line::from(Span::styled(
            format!(
                "{}: ${:.1}M / ${:.1}M",
                t(tui.lang, T::TradesPayrollCap),
                team.payroll_m,
                team.cap_m
            ),
            theme.muted_style(),
        )));
    }
    f.render_widget(
        Paragraph::new(preview)
            .block(theme.block(t(tui.lang, T::TradesRosterPreview)))
            .wrap(Wrap { trim: false }),
        cols[1],
    );

    ActionBar::new(&[
        ("Up/Down", t(tui.lang, T::CommonMove)),
        ("A-Z", t(tui.lang, T::CommonPick)),
        ("Enter", t(tui.lang, T::CommonConfirm)),
        ("Esc", t(tui.lang, T::CommonBack)),
    ])
    .render(f, parts[1], theme);
}

fn draw_builder_compose(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    tui: &TuiApp,
    cache: &TradesCache,
    verdict: VerdictView,
) {
    let target = team_label(cache, cache.target_team).unwrap_or_else(|| "???".into());
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(7),
        ])
        .split(area);
    let mut top = vec![Line::from(Span::styled(
        format!(
            "{} - {} {} · {} {}",
            t(tui.lang, T::TradesBuilderTitle),
            t(tui.lang, T::TradesTargetTeam),
            target,
            t(tui.lang, T::TradesMyTeam),
            tui.user_abbrev
        ),
        theme.accent_style(),
    ))];
    let mut chips = format!("[{}]", t(tui.lang, T::TradesBuilderTopBar));
    if tui.god_mode {
        chips.push_str(&format!("  [{}]", t(tui.lang, T::TradesForceTradeChip)));
    }
    top.push(Line::from(Span::styled(chips, theme.text())));
    f.render_widget(Paragraph::new(top).block(theme.block("")), body[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body[1]);
    let (incoming_title, incoming_rows, incoming_selected) = incoming_view(cache);
    draw_asset_list(
        f,
        cols[0],
        theme,
        tui.lang,
        &format!(
            "{} {} ({} / {})",
            t(tui.lang, T::TradesReceiveList),
            incoming_title,
            incoming_selected.len(),
            incoming_rows.len()
        ),
        incoming_rows,
        cache.in_cursor,
        incoming_selected,
        cache.builder_panel == BuilderPanel::Incoming,
    );
    draw_asset_list(
        f,
        cols[1],
        theme,
        tui.lang,
        &format!(
            "{} ({} / {})",
            t(tui.lang, T::TradesSendList),
            cache.selected_out.len(),
            cache.user_roster.as_ref().map(|r| r.len()).unwrap_or(0)
        ),
        cache.user_roster.as_deref().unwrap_or(&[]),
        cache.out_cursor,
        &cache.selected_out,
        cache.builder_panel == BuilderPanel::Outgoing,
    );
    draw_verdict_bar(f, body[2], theme, tui.lang, verdict);
}

fn draw_asset_list(
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
    let visible = area.height.saturating_sub(5).max(1) as usize;
    let start = if cursor >= visible {
        cursor + 1 - visible
    } else {
        0
    };
    let mut lines = vec![Line::from(Span::styled(
        format!("-- {} --", t(lang, T::TradesSectionPlayers)),
        theme.muted_style(),
    ))];
    for (i, p) in rows.iter().enumerate().skip(start).take(visible) {
        let is_selected = selected.contains(&p.id);
        let style = if focused && i == cursor || is_selected {
            theme.highlight()
        } else {
            theme.text()
        };
        let mark = if is_selected { "✓" } else { " " };
        lines.push(Line::from(Span::styled(
            format!(
                "{} {} {:<2} {:>2} {:>2} {:>7} {:>3}",
                mark,
                pad_display(&p.name, 16),
                p.position,
                p.age,
                p.overall,
                money_m(p.salary_m),
                years_label(p.years)
            ),
            style,
        )));
    }
    lines.push(Line::from(Span::styled(
        format!(
            "-- {} ({}) --",
            t(lang, T::TradesSectionPicks),
            t(lang, T::TradesPicksDeferred)
        ),
        theme.muted_style(),
    )));
    lines.push(Line::from(Span::styled(
        t(lang, T::TradesPicksDeferred),
        theme.muted_style(),
    )));
    let title = if focused {
        format!("{} > ", title)
    } else {
        title.to_string()
    };
    f.render_widget(Paragraph::new(lines).block(theme.block(&title)), area);
}

#[derive(Default)]
struct VerdictView {
    sent_m: f32,
    received_m: f32,
    warnings: Vec<String>,
    gm_dialog: Option<String>,
    cap_pass: bool,
}

fn draw_verdict_bar(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, verdict: VerdictView) {
    let delta = verdict.received_m - verdict.sent_m;
    let mut salary_spans = vec![Span::styled(
        format!(
            "{} ${:.1}M / {} ${:.1}M / {} {:+.1}M",
            t(lang, T::TradesVerdictSent),
            verdict.sent_m,
            t(lang, T::TradesVerdictReceived),
            verdict.received_m,
            t(lang, T::TradesVerdictDelta),
            delta
        ),
        theme.text(),
    )];
    if verdict.cap_pass {
        salary_spans.push(Span::styled(
            format!("  ✓ {}", t(lang, T::TradesVerdictCapPass)),
            Style::default().fg(Color::Green),
        ));
    }
    let mut lines = vec![Line::from(salary_spans)];
    for warning in verdict.warnings.iter().take(3) {
        lines.push(Line::from(Span::styled(
            warning.clone(),
            theme.accent_style(),
        )));
    }
    if let Some(dialog) = verdict.gm_dialog {
        lines.push(Line::from(Span::styled(dialog, theme.accent_style())));
    } else if verdict.warnings.is_empty() {
        lines.push(Line::from(Span::styled(
            t(lang, T::TradesVerdictPrompt),
            theme.muted_style(),
        )));
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(theme.block(t(lang, T::TradesVerdictTitle)))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn build_verdict_view(app: &mut AppState, tui: &TuiApp) -> VerdictView {
    let payload = CACHE.with(|c| {
        let cache = c.borrow();
        let sent_m = selected_salary(
            cache.user_roster.as_deref().unwrap_or(&[]),
            &cache.selected_out,
        );
        let received_m = selected_salary(
            cache.target_roster.as_deref().unwrap_or(&[]),
            &cache.selected_in,
        ) + selected_salary(
            cache.third_roster.as_deref().unwrap_or(&[]),
            &cache.selected_third,
        );
        (
            sent_m,
            received_m,
            cache.gm_dialog.clone(),
            build_offer_from_cache(&cache, tui),
        )
    });
    let (sent_m, received_m, gm_dialog, offer) = payload;
    let mut view = VerdictView {
        sent_m,
        received_m,
        gm_dialog,
        ..Default::default()
    };
    let Some(offer) = offer else {
        return view;
    };
    if let Ok(snapshot) = build_league_snapshot(app) {
        match cba::validate(&offer, &snapshot.view()) {
            Ok(()) => view.cap_pass = true,
            Err(violation) => {
                view.cap_pass = cba_cap_rules_passed(&violation);
                if matches!(violation, CbaViolation::RosterSize { .. }) {
                    view.warnings
                        .extend(roster_size_warnings(tui.lang, &offer, &snapshot.view()));
                } else {
                    view.warnings.push(cba_warning_to_text(
                        tui.lang,
                        &violation,
                        &offer,
                        &snapshot.view(),
                    ));
                }
            }
        }
        view.warnings
            .extend(trade_kicker_notes(tui.lang, &offer, &snapshot.view()));
    }
    view
}

fn cba_cap_rules_passed(violation: &CbaViolation) -> bool {
    !matches!(
        violation,
        CbaViolation::SalaryMatching { .. }
            | CbaViolation::HardCapTrigger { .. }
            | CbaViolation::NoTradeClause(_)
            | CbaViolation::Apron2Restriction { .. }
    )
}

fn cba_warning_to_text(
    lang: Lang,
    violation: &CbaViolation,
    offer: &TradeOffer,
    league: &LeagueSnapshot<'_>,
) -> String {
    match violation {
        CbaViolation::SalaryMatching {
            team,
            out_dollars,
            in_dollars,
            ..
        } => {
            let out = Cents::from_dollars(*out_dollars);
            let inc = Cents::from_dollars(*in_dollars);
            let tier = cba::classify_salary_tier(*team, league);
            let limit = cba::max_incoming_for_tier(tier, out, *team, league);
            let diff_m = Cents(inc.0.saturating_sub(limit.0)).as_millions_f32();
            let ratio = if out.0 > 0 {
                (inc.0 as f32 / out.0 as f32) * 100.0
            } else {
                0.0
            };
            format!(
                "⚠ {} 当前进/送 = {:.0}%, 需削减进薪约 ${:.1}M.",
                t(lang, T::TradesWarnSalaryMatch),
                ratio,
                diff_m
            )
        }
        CbaViolation::HardCapTrigger { team, apron } => {
            let pre = cba::team_total_salary(*team, league);
            let out = cba::outgoing_salary_pre_kicker(*team, offer, league);
            let inc = cba::incoming_salary_post_kicker(*team, offer, league);
            let post = Cents(pre.0.saturating_sub(out.0).saturating_add(inc.0));
            let over_m =
                Cents(post.0.saturating_sub(league.league_year.apron_2.0)).as_millions_f32();
            format!(
                "⚠ {} 第{}档, 超出约 ${:.1}M.",
                t(lang, T::TradesWarnHardCap),
                apron,
                over_m
            )
        }
        CbaViolation::NoTradeClause(pid) => {
            let name = league
                .player(*pid)
                .map(|p| clean_name(&p.name))
                .unwrap_or_else(|| format!("#{}", pid.0));
            format!("⚠ {} {}", name, t(lang, T::TradesWarnNTC))
        }
        CbaViolation::RosterSize { size, .. } => {
            format_roster_size_warning(lang, violation_team(violation), *size, league)
        }
        CbaViolation::Apron2Restriction { .. } => t(lang, T::TradesWarnSalaryMatch).to_string(),
        CbaViolation::CashLimitExceeded { .. } => t(lang, T::TradesWarnSalaryMatch).to_string(),
        CbaViolation::AggregationCooldown { .. } => t(lang, T::TradesWarnSalaryMatch).to_string(),
    }
}

fn roster_size_warnings(
    lang: Lang,
    offer: &TradeOffer,
    league: &LeagueSnapshot<'_>,
) -> Vec<String> {
    offer
        .assets_by_team
        .keys()
        .filter_map(|team| match cba::check_roster_size(*team, offer, league) {
            Err(CbaViolation::RosterSize { team, size }) => {
                Some(format_roster_size_warning(lang, team, size, league))
            }
            _ => None,
        })
        .collect()
}

fn violation_team(violation: &CbaViolation) -> TeamId {
    match violation {
        CbaViolation::SalaryMatching { team, .. }
        | CbaViolation::HardCapTrigger { team, .. }
        | CbaViolation::CashLimitExceeded { team, .. }
        | CbaViolation::AggregationCooldown { team, .. }
        | CbaViolation::RosterSize { team, .. }
        | CbaViolation::Apron2Restriction { team } => *team,
        CbaViolation::NoTradeClause(_) => TeamId(0),
    }
}

fn format_roster_size_warning(
    lang: Lang,
    team: TeamId,
    size: u32,
    league: &LeagueSnapshot<'_>,
) -> String {
    let abbrev = league
        .team(team)
        .map(|t| t.abbrev.clone())
        .unwrap_or_else(|| format!("T{}", team.0));
    t(lang, T::TradesWarnRosterSize)
        .replace("{team}", &abbrev)
        .replace("{count}", &size.to_string())
}

fn trade_kicker_notes(lang: Lang, offer: &TradeOffer, league: &LeagueSnapshot<'_>) -> Vec<String> {
    offer
        .assets_by_team
        .values()
        .flat_map(|assets| assets.players_out.iter())
        .filter_map(|pid| league.player(*pid))
        .filter(|p| p.trade_kicker_pct.unwrap_or(0) > 0)
        .map(|p| {
            format!(
                "ℹ {} {}",
                clean_name(&p.name),
                t(lang, T::TradesNoteTradeKicker)
            )
        })
        .collect()
}

fn selected_salary(rows: &[PlayerOption], selected: &HashSet<PlayerId>) -> f32 {
    rows.iter()
        .filter(|p| selected.contains(&p.id))
        .map(|p| p.salary_m)
        .sum()
}

fn build_offer_from_cache(cache: &TradesCache, tui: &TuiApp) -> Option<TradeOffer> {
    let target = cache.target_team?;
    let mut assets_by_team = IndexMap::new();
    assets_by_team.insert(
        tui.user_team,
        TradeAssets {
            players_out: selected_ids(
                cache.user_roster.as_deref().unwrap_or(&[]),
                &cache.selected_out,
            ),
            picks_out: vec![],
            cash_out: Cents::ZERO,
        },
    );
    assets_by_team.insert(
        target,
        TradeAssets {
            players_out: selected_ids(
                cache.target_roster.as_deref().unwrap_or(&[]),
                &cache.selected_in,
            ),
            picks_out: vec![],
            cash_out: Cents::ZERO,
        },
    );
    if cache.builder_mode == BuilderMode::ThreeTeam {
        if let Some(third) = cache.third_team {
            assets_by_team.insert(
                third,
                TradeAssets {
                    players_out: selected_ids(
                        cache.third_roster.as_deref().unwrap_or(&[]),
                        &cache.selected_third,
                    ),
                    picks_out: vec![],
                    cash_out: Cents::ZERO,
                },
            );
        }
    }
    Some(TradeOffer {
        id: TradeId(0),
        initiator: tui.user_team,
        assets_by_team,
        round: 1,
        parent: None,
    })
}

fn selected_ids(rows: &[PlayerOption], selected: &HashSet<PlayerId>) -> Vec<PlayerId> {
    rows.iter()
        .filter(|p| selected.contains(&p.id))
        .map(|p| p.id)
        .collect()
}

fn team_label(cache: &TradesCache, id: Option<TeamId>) -> Option<String> {
    let id = id?;
    cache
        .teams
        .as_ref()?
        .iter()
        .find(|team| team.id == id)
        .map(|team| team.abbrev.clone())
}

fn incoming_view(cache: &TradesCache) -> (String, &[PlayerOption], &HashSet<PlayerId>) {
    if cache.builder_mode == BuilderMode::ThreeTeam && cache.incoming_slot == IncomingSlot::Second {
        (
            team_label(cache, cache.third_team)
                .map(|label| format!("({})", label))
                .unwrap_or_else(|| "(T2)".to_string()),
            cache.third_roster.as_deref().unwrap_or(&[]),
            &cache.selected_third,
        )
    } else {
        (
            team_label(cache, cache.target_team)
                .map(|label| format!("({})", label))
                .unwrap_or_else(|| "(T1)".to_string()),
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
    let panel_style = theme.text();
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
        Line::from(Span::styled(
            t(tui.lang, T::TradesToggleTeamMode),
            theme.muted_style(),
        )),
        Line::from(Span::styled(
            t(tui.lang, T::TradesSwapIncomingTeam),
            theme.muted_style(),
        )),
    ]);
    f.render_widget(
        Paragraph::new(lines)
            .block(theme.block(t(tui.lang, T::TradesSubmit)))
            .wrap(Wrap { trim: true }),
        area,
    );
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
    let rendered_picker = CACHE.with(|c| {
        let cache = c.borrow();
        match &cache.modal {
            Modal::OfferAction { picker, .. } | Modal::ChainAction { picker, .. } => {
                picker.render(f, rect, theme);
                true
            }
            _ => false,
        }
    });
    if rendered_picker {
        return;
    }

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
            Modal::OfferAction { .. } | Modal::ChainAction { .. } => unreachable!(),
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

    if CACHE.with(|c| c.borrow().fa_rows.is_none()) {
        let rows = build_fa_rows(app)?;
        CACHE.with(|c| c.borrow_mut().fa_rows = Some(rows));
    }

    if CACHE.with(|c| c.borrow().teams.is_none()) {
        let teams = build_team_options(app, tui.user_team, tui.season)?;
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
        detail_lines.push(format!("Advisory verdict: {}", verdict));
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
                verdict_label(tui.lang, &ev.verdict).to_string()
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

fn build_team_options(
    app: &mut AppState,
    user_team: TeamId,
    season: SeasonId,
) -> Result<Vec<TeamOption>> {
    let store = app.store()?;
    let standings = store
        .read_standings(season)?
        .into_iter()
        .map(|row| (row.team, (row.wins, row.losses)))
        .collect::<HashMap<_, _>>();
    let cap_m = LeagueYear::for_season(season)
        .map(|ly| ly.cap.as_millions_f32())
        .unwrap_or(0.0);
    let teams = store.list_teams()?;
    let mut out = Vec::new();
    for t in teams.into_iter().filter(|t| t.id != user_team) {
        let payroll_m = store
            .team_salary(t.id, season)
            .map(|c| c.as_millions_f32())
            .unwrap_or(0.0);
        let (wins, losses) = standings.get(&t.id).copied().unwrap_or((0, 0));
        let name = t.full_name();
        out.push(TeamOption {
            id: t.id,
            abbrev: t.abbrev,
            name,
            wins: wins.into(),
            losses: losses.into(),
            payroll_m,
            cap_m,
        });
    }
    Ok(out)
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
                years: p
                    .contract
                    .as_ref()
                    .map(|c| c.years.iter().filter(|y| y.season.0 >= season.0).count() as u32)
                    .unwrap_or(0),
            }
        })
        .collect())
}

enum ModalKeyAction {
    None,
    Consumed,
    Respond { id: TradeId, action: TradeResponse },
}

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    let modal_action = CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        match &mut cache.modal {
            Modal::None => ModalKeyAction::None,
            Modal::OfferAction { id, picker } | Modal::ChainAction { id, picker } => {
                match picker.handle_key(key) {
                    WidgetEvent::Submitted => {
                        let id = *id;
                        let action = picker.selected().copied();
                        cache.modal = Modal::None;
                        action
                            .map(|action| ModalKeyAction::Respond { id, action })
                            .unwrap_or(ModalKeyAction::Consumed)
                    }
                    WidgetEvent::Cancelled => {
                        cache.modal = Modal::None;
                        ModalKeyAction::Consumed
                    }
                    _ => ModalKeyAction::Consumed,
                }
            }
            _ => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                    cache.modal = Modal::None;
                }
                ModalKeyAction::Consumed
            }
        }
    });
    match modal_action {
        ModalKeyAction::None => {}
        ModalKeyAction::Consumed => return Ok(true),
        ModalKeyAction::Respond { id, action } => {
            return respond_to_chain(app, tui, id, action.as_command_str());
        }
    }

    match key.code {
        KeyCode::Tab => {
            CACHE.with(|c| {
                let tab = c.borrow().tab;
                c.borrow_mut().tab = next_tab(tab);
            });
            Ok(true)
        }
        KeyCode::BackTab => {
            CACHE.with(|c| {
                let tab = c.borrow().tab;
                c.borrow_mut().tab = prev_tab(tab);
            });
            Ok(true)
        }
        KeyCode::Char('1') => set_tab(SubTab::Inbox),
        KeyCode::Char('2') => set_tab(SubTab::Proposals),
        KeyCode::Char('3') => set_tab(SubTab::Builder),
        KeyCode::Char('4') => set_tab(SubTab::FreeAgents),
        _ => match CACHE.with(|c| c.borrow().tab) {
            SubTab::Inbox => handle_inbox_key(app, tui, key),
            SubTab::Proposals => handle_proposals_key(app, tui, key),
            SubTab::Builder => handle_builder_key(app, tui, key),
            SubTab::FreeAgents => handle_free_agents_key(app, tui, key),
        },
    }
}

fn set_tab(tab: SubTab) -> Result<bool> {
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache.tab = tab;
        if tab == SubTab::Builder {
            cache.builder_step = BuilderStep::PickTeam;
        }
    });
    Ok(true)
}

fn next_tab(tab: SubTab) -> SubTab {
    match tab {
        SubTab::Inbox => SubTab::Proposals,
        SubTab::Proposals => SubTab::Builder,
        SubTab::Builder => SubTab::FreeAgents,
        SubTab::FreeAgents => SubTab::Inbox,
    }
}

fn prev_tab(tab: SubTab) -> SubTab {
    match tab {
        SubTab::Inbox => SubTab::FreeAgents,
        SubTab::Proposals => SubTab::Inbox,
        SubTab::Builder => SubTab::Proposals,
        SubTab::FreeAgents => SubTab::Builder,
    }
}

fn handle_inbox_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Up => move_inbox(-1),
        KeyCode::Down => move_inbox(1),
        KeyCode::PageUp => move_inbox(-10),
        KeyCode::PageDown => move_inbox(10),
        KeyCode::Enter => open_current_inbox_action_picker(tui),
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
        KeyCode::Enter => open_current_chain_action_picker(tui),
        KeyCode::Char('a') => respond_current_chain(app, tui, "accept"),
        KeyCode::Char('r') => respond_current_chain(app, tui, "reject"),
        KeyCode::Char('c') => respond_current_chain(app, tui, "counter"),
        _ => Ok(false),
    }
}

fn open_current_inbox_action_picker(tui: &mut TuiApp) -> Result<bool> {
    let Some(id) = current_offer_id() else {
        tui.last_msg = Some("no open inbox offer selected".into());
        return Ok(true);
    };
    CACHE.with(|c| {
        c.borrow_mut().modal = Modal::OfferAction {
            id,
            picker: trade_response_picker(tui.lang),
        };
    });
    Ok(true)
}

fn open_current_chain_action_picker(tui: &mut TuiApp) -> Result<bool> {
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
    CACHE.with(|c| {
        c.borrow_mut().modal = Modal::ChainAction {
            id,
            picker: trade_response_picker(tui.lang),
        };
    });
    Ok(true)
}

fn handle_builder_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    let step = CACHE.with(|c| c.borrow().builder_step);
    if step == BuilderStep::PickTeam {
        return match key.code {
            KeyCode::Up => move_builder_cursor(-1),
            KeyCode::Down => move_builder_cursor(1),
            KeyCode::Enter => builder_activate(app, tui),
            KeyCode::Char(ch) if ch.is_ascii_alphabetic() => {
                jump_team(ch);
                Ok(true)
            }
            KeyCode::Esc => Ok(false),
            _ => Ok(false),
        };
    }
    match key.code {
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
            CACHE.with(|c| {
                let panel = c.borrow().builder_panel;
                c.borrow_mut().builder_panel = match panel {
                    BuilderPanel::Incoming => BuilderPanel::Outgoing,
                    BuilderPanel::Outgoing => BuilderPanel::Incoming,
                };
            });
            Ok(true)
        }
        KeyCode::Up => move_builder_cursor(-1),
        KeyCode::Down => move_builder_cursor(1),
        KeyCode::Char('m') => toggle_builder_mode(),
        KeyCode::Char('i') => cycle_incoming_slot(),
        KeyCode::Char('t') | KeyCode::Char('T') => {
            CACHE.with(|c| c.borrow_mut().builder_step = BuilderStep::PickTeam);
            Ok(true)
        }
        KeyCode::Char('f') | KeyCode::Char('F') if tui.god_mode => submit_builder(app, tui, true),
        KeyCode::Enter => submit_builder(app, tui, false),
        KeyCode::Char(' ') => builder_activate(app, tui),
        KeyCode::Char('c') => {
            CACHE.with(|c| {
                let mut cache = c.borrow_mut();
                cache.selected_out.clear();
                cache.selected_in.clear();
                cache.selected_third.clear();
                cache.gm_dialog = None;
            });
            Ok(true)
        }
        KeyCode::Esc => {
            CACHE.with(|c| c.borrow_mut().builder_step = BuilderStep::PickTeam);
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn jump_team(ch: char) {
    let target = ch.to_ascii_uppercase();
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if let Some(idx) = cache.teams.as_ref().and_then(|teams| {
            teams
                .iter()
                .position(|team| team.abbrev.starts_with(target))
        }) {
            cache.team_cursor = idx;
        }
    });
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
        match cache.builder_step {
            BuilderStep::PickTeam => {
                let len = cache.teams.as_ref().map(|r| r.len()).unwrap_or(0);
                cache.team_cursor = moved(cache.team_cursor, len, delta);
            }
            BuilderStep::Compose => match cache.builder_panel {
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
            },
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
        cache.gm_dialog = None;
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
            cache.gm_dialog = None;
        }
    });
    Ok(true)
}

fn builder_activate(app: &mut AppState, tui: &mut TuiApp) -> Result<bool> {
    let action = CACHE.with(|c| {
        let cache = c.borrow();
        if cache.builder_step == BuilderStep::PickTeam {
            cache.teams.as_ref().and_then(|teams| {
                teams
                    .get(cache.team_cursor)
                    .map(|t| BuilderAction::SetTeam(t.id))
            })
        } else {
            match cache.builder_panel {
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
            }
        }
    });
    match action {
        Some(BuilderAction::SetTeam(team)) => {
            let assignment = CACHE.with(|c| {
                let mut cache = c.borrow_mut();
                let assignment = select_builder_team(&mut cache, team);
                cache.builder_step = BuilderStep::Compose;
                cache.builder_panel = BuilderPanel::Incoming;
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
            CACHE.with(|c| {
                let mut cache = c.borrow_mut();
                toggle(&mut cache.selected_out, pid);
                cache.gm_dialog = None;
            });
            Ok(true)
        }
        Some(BuilderAction::ToggleIn(pid)) => {
            CACHE.with(|c| {
                let mut cache = c.borrow_mut();
                toggle(&mut cache.selected_in, pid);
                cache.gm_dialog = None;
            });
            Ok(true)
        }
        Some(BuilderAction::ToggleThird(pid)) => {
            CACHE.with(|c| {
                let mut cache = c.borrow_mut();
                toggle(&mut cache.selected_third, pid);
                cache.gm_dialog = None;
            });
            Ok(true)
        }
        Some(BuilderAction::Submit) => submit_builder(app, tui, false),
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

fn submit_builder(app: &mut AppState, tui: &mut TuiApp, force: bool) -> Result<bool> {
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
            build_offer_from_cache(&cache, tui),
        )
    });

    let (mode, first, second, send, first_send, second_send, offer) = payload;
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
            let dialog = offer
                .as_ref()
                .map(|offer| gm_dialog_for_offer(app, tui, &target.abbrev, target.id, offer, force))
                .unwrap_or_else(|| t(tui.lang, T::TradesGmRejectBadFaith).to_string());
            let cmd = Command::Trade(TradeArgs {
                action: TradeAction::Propose {
                    from: tui.user_abbrev.clone(),
                    to: target.abbrev.clone(),
                    send,
                    receive: first_send,
                    json: false,
                    force,
                },
            });
            let res = with_silenced_io(|| dispatch(app, cmd));
            CACHE.with(|c| c.borrow_mut().gm_dialog = Some(dialog));
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
            CACHE.with(|c| {
                c.borrow_mut().gm_dialog = Some(format!(
                    "{} {}",
                    first.abbrev,
                    t(tui.lang, T::TradesGmRejectBadFaith)
                ));
            });
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

fn gm_dialog_for_offer(
    app: &mut AppState,
    tui: &TuiApp,
    target_abbrev: &str,
    target: TeamId,
    offer: &TradeOffer,
    force: bool,
) -> String {
    if force {
        return format!(
            "{} {}",
            target_abbrev,
            t(tui.lang, T::TradesGodAcceptDialog)
        );
    }
    let Ok(snapshot) = build_league_snapshot(app) else {
        return format!(
            "{} {}",
            target_abbrev,
            t(tui.lang, T::TradesGmRejectBadFaith)
        );
    };
    let snap = snapshot.view();
    if let Err(violation) = cba::validate(offer, &snap) {
        return cba_gm_reject_dialog(tui.lang, target_abbrev, &violation, &snap);
    }
    let mut rng = ChaCha8Rng::seed_from_u64(tui.season.0 as u64 ^ target.0 as u64 ^ 0xB17D);
    let evaluation = evaluate_mod::evaluate(offer, target, &snap, &mut rng);
    match evaluation.verdict {
        Verdict::Accept => format!("{} {}", target_abbrev, t(tui.lang, T::TradesGmAccept)),
        Verdict::Reject(RejectReason::InsufficientValue) => {
            format!(
                "{} {}",
                target_abbrev,
                t(tui.lang, T::TradesGmRejectInsufficient)
            )
        }
        Verdict::Reject(RejectReason::CbaViolation(_)) => {
            format!("{} {}", target_abbrev, t(tui.lang, T::TradesGmRejectCba))
        }
        Verdict::Reject(RejectReason::NoTradeClause(pid)) => {
            let player = snap
                .player(pid)
                .map(|p| clean_name(&p.name))
                .unwrap_or_else(|| format!("#{}", pid.0));
            format!(
                "{} GM: \"{} {}\"",
                target_abbrev,
                player,
                t(tui.lang, T::TradesGmRejectUntouchable)
            )
        }
        Verdict::Reject(_) => format!(
            "{} {}",
            target_abbrev,
            t(tui.lang, T::TradesGmRejectBadFaith)
        ),
        Verdict::Counter(counter) => {
            let names = counter_names_for_team(&counter, target, &snap);
            CACHE.with(|c| {
                let mut cache = c.borrow_mut();
                if let Some(user_assets) = counter.assets_by_team.get(&tui.user_team) {
                    cache.selected_out = user_assets.players_out.iter().copied().collect();
                }
                if let Some(target_assets) = counter.assets_by_team.get(&target) {
                    cache.selected_in = target_assets.players_out.iter().copied().collect();
                }
            });
            if names.len() <= 1 {
                format!(
                    "{} GM: \"差不多, 但我这边觉得你还得加 {}.\"",
                    target_abbrev,
                    names
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "more value".into())
                )
            } else {
                format!(
                    "{} GM: \"你给的太轻了, 至少得加上 {} 我才考虑.\"",
                    target_abbrev,
                    names.join(" + ")
                )
            }
        }
    }
}

fn cba_gm_reject_dialog(
    lang: Lang,
    target_abbrev: &str,
    violation: &CbaViolation,
    snap: &LeagueSnapshot<'_>,
) -> String {
    match violation {
        CbaViolation::SalaryMatching { .. } => {
            format!("{} {}", target_abbrev, t(lang, T::TradesGmRejectSalaryMatch))
        }
        CbaViolation::HardCapTrigger { .. } | CbaViolation::Apron2Restriction { .. } => {
            format!("{} {}", target_abbrev, t(lang, T::TradesGmRejectHardCap))
        }
        CbaViolation::RosterSize { .. } => {
            format!("{} {}", target_abbrev, t(lang, T::TradesGmRejectRoster))
        }
        CbaViolation::NoTradeClause(pid) => {
            let player = snap
                .player(*pid)
                .map(|p| clean_name(&p.name))
                .unwrap_or_else(|| format!("#{}", pid.0));
            format!(
                "{} GM: \"{} {}\"",
                target_abbrev,
                player,
                t(lang, T::TradesGmRejectUntouchable)
            )
        }
        CbaViolation::CashLimitExceeded { .. } | CbaViolation::AggregationCooldown { .. } => {
            format!("{} {}", target_abbrev, t(lang, T::TradesGmRejectCba))
        }
    }
}

fn counter_names_for_team(
    offer: &TradeOffer,
    team: TeamId,
    snap: &LeagueSnapshot<'_>,
) -> Vec<String> {
    offer
        .assets_by_team
        .get(&team)
        .map(|assets| {
            assets
                .players_out
                .iter()
                .filter_map(|pid| snap.player(*pid))
                .map(|p| clean_name(&p.name))
                .collect()
        })
        .unwrap_or_default()
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
        BuilderPanel::Incoming => BuilderPanel::Outgoing,
        BuilderPanel::Outgoing => BuilderPanel::Incoming,
    }
}

fn next_panel(panel: BuilderPanel) -> BuilderPanel {
    prev_panel(panel)
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

fn trade_response_picker(lang: Lang) -> Picker<TradeResponse> {
    Picker::new(
        t(lang, T::TradesActionPickerTitle),
        vec![
            TradeResponse::Accept,
            TradeResponse::Reject,
            TradeResponse::Counter,
        ],
        |response| response.label(lang).to_string(),
    )
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

fn quick_label(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Quick",
        Lang::Zh => "快捷",
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

fn pad_display(s: &str, width: usize) -> String {
    let mut out = shorten(s, width);
    let len = out.chars().count();
    if len < width {
        out.push_str(&" ".repeat(width - len));
    }
    out
}

fn money_m(value: f32) -> String {
    if value <= 0.0 {
        "—".to_string()
    } else {
        format!("${:.1}M", value)
    }
}

fn years_label(years: u32) -> String {
    if years == 0 {
        "—".to_string()
    } else {
        format!("{}y", years)
    }
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
