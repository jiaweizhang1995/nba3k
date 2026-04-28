//! Home dashboard. M24 replaces the inbox/news landing page with a compact
//! front-office dashboard: record, conference standings, leaders, team stats,
//! finances, and starting lineup.

use anyhow::{anyhow, Result};
use crossterm::event::KeyEvent;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
    Frame,
};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::state::AppState;
use crate::tui::widgets::Theme;
use crate::tui::{SaveCtx, TuiApp};
use nba3k_core::{
    t, Cents, LeagueYear, Player, PlayerId, PlayerLine, Position, SeasonId, TeamId, T,
};
use nba3k_store::StandingRow;

thread_local! {
    static CACHE: RefCell<HomeCache> = RefCell::new(HomeCache::default());
}

#[derive(Default)]
struct HomeCache {
    cached_for: Option<(SeasonId, u32, TeamId)>,
    snapshot: Option<HomeSnapshot>,
}

struct HomeSnapshot {
    record: String,
    conference_rank: String,
    standings: Vec<StandingDisplay>,
    user_team: TeamId,
    team_leaders: Vec<LeaderRow>,
    league_leaders: Vec<LeaderRow>,
    team_stats: Vec<StatRow>,
    finances: Vec<(T, String)>,
    lineup: Vec<LineupRow>,
}

struct StandingDisplay {
    rank: usize,
    team: TeamId,
    abbrev: String,
    wins: u16,
    losses: u16,
    gb: String,
}

#[derive(Clone)]
struct LeaderRow {
    metric: &'static str,
    name: String,
    abbrev: Option<String>,
    value: f32,
}

struct StatRow {
    label: T,
    value: f32,
    rank: usize,
}

struct LineupRow {
    position: Position,
    name: String,
    ppg: f32,
    rpg: f32,
    apg: f32,
    mpg: f32,
}

#[derive(Default, Clone)]
struct PlayerTotals {
    gp: u32,
    pts: u32,
    reb: u32,
    ast: u32,
    minutes: u32,
    team: Option<TeamId>,
}

impl PlayerTotals {
    fn from_line(&mut self, line: &PlayerLine, team: TeamId) {
        self.gp += 1;
        self.pts += line.pts as u32;
        self.reb += line.reb as u32;
        self.ast += line.ast as u32;
        self.minutes += line.minutes as u32;
        self.team = Some(team);
    }

    fn ppg(&self) -> f32 { per_game(self.pts, self.gp) }
    fn rpg(&self) -> f32 { per_game(self.reb, self.gp) }
    fn apg(&self) -> f32 { per_game(self.ast, self.gp) }
    fn mpg(&self) -> f32 { per_game(self.minutes, self.gp) }
}

#[derive(Default, Clone)]
struct TeamTotals {
    games: u32,
    pts: u32,
    opp_pts: u32,
    reb: u32,
    ast: u32,
}

impl TeamTotals {
    fn ppg(&self) -> f32 { per_game(self.pts, self.games) }
    fn oppg(&self) -> f32 { per_game(self.opp_pts, self.games) }
    fn rpg(&self) -> f32 { per_game(self.reb, self.games) }
    fn apg(&self) -> f32 { per_game(self.ast, self.games) }
}

/// Drop the cached dashboard so the next render re-fetches. Called from the
/// shell whenever sim or save mutation happens.
pub fn invalidate() {
    CACHE.with(|c| *c.borrow_mut() = HomeCache::default());
}

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

    CACHE.with(|c| {
        let cache = c.borrow();
        let Some(snapshot) = cache.snapshot.as_ref() else {
            return;
        };
        draw_dashboard(f, area, theme, tui, snapshot);
    });
}

fn draw_dashboard(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp, s: &HomeSnapshot) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(10),
            Constraint::Length(8),
        ])
        .split(area);

    draw_header(f, outer[0], theme, tui, s);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(outer[1]);

    draw_standings(f, middle[0], theme, tui, s);

    let center = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(middle[1]);
    draw_leaders(
        f,
        center[0],
        theme,
        t(tui.lang, T::HomeTeamLeaders),
        &s.team_leaders,
    );
    draw_leaders(
        f,
        center[1],
        theme,
        t(tui.lang, T::HomeLeagueLeaders),
        &s.league_leaders,
    );

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(middle[2]);
    draw_team_stats(f, right[0], theme, tui, s);
    draw_finances(f, right[1], theme, tui, s);

    draw_lineup(f, outer[2], theme, tui, s);
}

fn draw_header(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp, s: &HomeSnapshot) {
    let team_identity = if tui.user_team_name.is_empty() {
        tui.user_abbrev.clone()
    } else {
        format!("{} {}", tui.user_abbrev, tui.user_team_name)
    };
    let lines = vec![
        Line::from(Span::styled(team_identity, theme.text())).alignment(Alignment::Center),
        Line::from(Span::styled(s.record.clone(), theme.accent_style()))
            .alignment(Alignment::Center),
        Line::from(Span::styled(
            format!("{} {}", s.conference_rank, t(tui.lang, T::HomeConferenceRank)),
            theme.text(),
        ))
            .alignment(Alignment::Center),
    ];
    let p = Paragraph::new(lines).block(theme.block(t(tui.lang, T::HomeTitle)));
    f.render_widget(p, area);
}

fn draw_standings(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp, s: &HomeSnapshot) {
    let header = Row::new(["#", "Team", "W-L", "GB"]).style(theme.muted_style());
    let rows = s.standings.iter().map(|r| {
        let style = if r.team == s.user_team {
            theme.highlight()
        } else {
            theme.text()
        };
        Row::new(vec![
            Cell::from(r.rank.to_string()),
            Cell::from(r.abbrev.clone()),
            Cell::from(format!("{}-{}", r.wins, r.losses)),
            Cell::from(r.gb.clone()),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(5),
        ],
    )
    .header(header)
    .block(theme.block(t(tui.lang, T::HomeConferenceStandings)));
    f.render_widget(table, area);
}

fn draw_leaders(f: &mut Frame, area: Rect, theme: &Theme, title: &str, rows: &[LeaderRow]) {
    let lines: Vec<Line> = rows
        .iter()
        .map(|r| {
            let name = if let Some(abbrev) = r.abbrev.as_ref() {
                format!("{} {}", short_name(&r.name), abbrev)
            } else {
                short_name(&r.name)
            };
            Line::from(vec![
                Span::styled(format!("{:<3} ", r.metric), theme.muted_style()),
                Span::styled(format!("{:<16}", name), theme.text()),
                Span::styled(format!("{:>5.1}", r.value), theme.accent_style()),
            ])
        })
        .collect();
    let p = Paragraph::new(lines).block(theme.block(title));
    f.render_widget(p, area);
}

fn draw_team_stats(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp, s: &HomeSnapshot) {
    let lines: Vec<Line> = s
        .team_stats
        .iter()
        .map(|r| {
            Line::from(vec![
                Span::styled(format!("{:<10}", t(tui.lang, r.label)), theme.text()),
                Span::styled(format!("{:>6.1} ", r.value), theme.accent_style()),
                Span::styled(format!("({})", ordinal(r.rank)), theme.muted_style()),
            ])
        })
        .collect();
    let p = Paragraph::new(lines).block(theme.block(t(tui.lang, T::HomeTeamStats)));
    f.render_widget(p, area);
}

fn draw_finances(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp, s: &HomeSnapshot) {
    let lines: Vec<Line> = s
        .finances
        .iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(format!("{:<16}", t(tui.lang, *label)), theme.text()),
                Span::styled(value.clone(), theme.accent_style()),
            ])
        })
        .collect();
    let p = Paragraph::new(lines).block(theme.block(t(tui.lang, T::HomeFinances)));
    f.render_widget(p, area);
}

fn draw_lineup(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp, s: &HomeSnapshot) {
    let header = Row::new(["Pos", "Player", "PPG", "RPG", "APG", "MIN"]).style(theme.muted_style());
    let rows = s.lineup.iter().map(|r| {
        Row::new(vec![
            Cell::from(r.position.to_string()),
            Cell::from(short_name(&r.name)),
            Cell::from(format!("{:.1}", r.ppg)),
            Cell::from(format!("{:.1}", r.rpg)),
            Cell::from(format!("{:.1}", r.apg)),
            Cell::from(format!("{:.1}", r.mpg)),
        ])
        .style(theme.text())
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Min(12),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
        ],
    )
    .header(header)
    .block(theme.block(t(tui.lang, T::HomeStartingLineup)));
    f.render_widget(table, area);
}

fn ensure_cache(app: &mut AppState, ctx: &SaveCtx) -> Result<()> {
    let key = (ctx.season, ctx.season_state.day, ctx.user_team);
    let need = CACHE.with(|c| c.borrow().cached_for != Some(key));
    if !need {
        return Ok(());
    }

    let snapshot = build_snapshot(app, ctx)?;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.cached_for = Some(key);
        c.snapshot = Some(snapshot);
    });
    Ok(())
}

fn build_snapshot(app: &mut AppState, ctx: &SaveCtx) -> Result<HomeSnapshot> {
    let store = app.store()?;
    let season = ctx.season;
    let teams = store.list_teams()?;
    let team_abbrevs: HashMap<TeamId, String> =
        teams.iter().map(|t| (t.id, t.abbrev.clone())).collect();
    let user_conf = teams
        .iter()
        .find(|t| t.id == ctx.user_team)
        .map(|t| t.conference)
        .ok_or_else(|| anyhow!("user team missing from teams table"))?;

    let mut all_rosters: HashMap<TeamId, Vec<Player>> = HashMap::new();
    let mut players: HashMap<PlayerId, Player> = HashMap::new();
    for team in &teams {
        let roster = store.roster_for_team(team.id)?;
        for p in &roster {
            players.insert(p.id, p.clone());
        }
        all_rosters.insert(team.id, roster);
    }

    let standings = store.read_standings(season)?;
    let conference_standings = conference_rows(&standings, user_conf);
    let user_row = conference_standings
        .iter()
        .find(|r| r.team == ctx.user_team)
        .or_else(|| standings.iter().find(|r| r.team == ctx.user_team));
    let wins = user_row.map(|r| r.wins).unwrap_or(0);
    let losses = user_row.map(|r| r.losses).unwrap_or(0);
    let rank = if wins == 0 && losses == 0 {
        1
    } else {
        conference_standings
            .iter()
            .position(|r| r.team == ctx.user_team)
            .map(|idx| idx + 1)
            .or_else(|| user_row.and_then(|r| r.conf_rank.map(|n| n as usize)))
            .unwrap_or(1)
    };

    let games = store.read_games(season)?;
    let (player_totals, team_totals) = aggregate_games(&games);

    let user_roster = all_rosters
        .get(&ctx.user_team)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let team_leaders = leaders_for_roster(user_roster, &player_totals, &team_abbrevs, false);
    let league_leaders = league_leaders(&players, &player_totals, &team_abbrevs);
    let team_stats = team_stat_rows(ctx.user_team, &teams, &team_totals);
    let finances = finance_rows(store.team_salary(ctx.user_team, season)?, season, ctx.user_team);
    let lineup = lineup_rows(
        store.read_starters(ctx.user_team)?,
        user_roster,
        &players,
        &player_totals,
    );

    Ok(HomeSnapshot {
        record: format!("{}-{}", wins, losses),
        conference_rank: ordinal(rank),
        standings: standing_display(conference_standings),
        user_team: ctx.user_team,
        team_leaders,
        league_leaders,
        team_stats,
        finances,
        lineup,
    })
}

fn conference_rows(rows: &[StandingRow], conf: nba3k_core::Conference) -> Vec<StandingRow> {
    let mut out: Vec<StandingRow> = rows
        .iter()
        .filter(|r| r.conference == conf)
        .cloned()
        .collect();
    out.sort_by(|a, b| b.wins.cmp(&a.wins).then(a.losses.cmp(&b.losses)).then(a.abbrev.cmp(&b.abbrev)));
    out
}

fn standing_display(rows: Vec<StandingRow>) -> Vec<StandingDisplay> {
    let leader_delta = rows
        .first()
        .map(|r| r.wins as i32 - r.losses as i32)
        .unwrap_or(0);
    rows.into_iter()
        .enumerate()
        .map(|(idx, r)| {
            let delta = r.wins as i32 - r.losses as i32;
            let gb_half = (leader_delta - delta).max(0);
            let gb = if idx == 0 {
                "-".to_string()
            } else if gb_half % 2 == 0 {
                format!("{:.0}", gb_half as f32 / 2.0)
            } else {
                format!("{:.1}", gb_half as f32 / 2.0)
            };
            StandingDisplay {
                rank: idx + 1,
                team: r.team,
                abbrev: r.abbrev,
                wins: r.wins,
                losses: r.losses,
                gb,
            }
        })
        .collect()
}

fn aggregate_games(
    games: &[nba3k_core::GameResult],
) -> (HashMap<PlayerId, PlayerTotals>, HashMap<TeamId, TeamTotals>) {
    let mut players: HashMap<PlayerId, PlayerTotals> = HashMap::new();
    let mut teams: HashMap<TeamId, TeamTotals> = HashMap::new();

    for g in games.iter().filter(|g| !g.is_playoffs) {
        let home_reb: u32 = g.box_score.home_lines.iter().map(|l| l.reb as u32).sum();
        let home_ast: u32 = g.box_score.home_lines.iter().map(|l| l.ast as u32).sum();
        let away_reb: u32 = g.box_score.away_lines.iter().map(|l| l.reb as u32).sum();
        let away_ast: u32 = g.box_score.away_lines.iter().map(|l| l.ast as u32).sum();

        add_team_game(&mut teams, g.home, g.home_score, g.away_score, home_reb, home_ast);
        add_team_game(&mut teams, g.away, g.away_score, g.home_score, away_reb, away_ast);

        for line in &g.box_score.home_lines {
            players.entry(line.player).or_default().from_line(line, g.home);
        }
        for line in &g.box_score.away_lines {
            players.entry(line.player).or_default().from_line(line, g.away);
        }
    }

    (players, teams)
}

fn add_team_game(
    teams: &mut HashMap<TeamId, TeamTotals>,
    team: TeamId,
    pts: u16,
    opp_pts: u16,
    reb: u32,
    ast: u32,
) {
    let row = teams.entry(team).or_default();
    row.games += 1;
    row.pts += pts as u32;
    row.opp_pts += opp_pts as u32;
    row.reb += reb;
    row.ast += ast;
}

fn leaders_for_roster(
    roster: &[Player],
    totals: &HashMap<PlayerId, PlayerTotals>,
    abbrevs: &HashMap<TeamId, String>,
    include_team: bool,
) -> Vec<LeaderRow> {
    [
        ("PPG", Metric::Ppg),
        ("RPG", Metric::Rpg),
        ("APG", Metric::Apg),
    ]
    .into_iter()
    .map(|(label, metric)| {
        let player = roster.iter().max_by(|a, b| {
            metric_value(totals.get(&a.id), metric)
                .partial_cmp(&metric_value(totals.get(&b.id), metric))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.overall.cmp(&b.overall))
        });
        leader_from_player(label, player, totals, abbrevs, include_team)
    })
    .collect()
}

fn league_leaders(
    players: &HashMap<PlayerId, Player>,
    totals: &HashMap<PlayerId, PlayerTotals>,
    abbrevs: &HashMap<TeamId, String>,
) -> Vec<LeaderRow> {
    [
        ("PPG", Metric::Ppg),
        ("RPG", Metric::Rpg),
        ("APG", Metric::Apg),
    ]
    .into_iter()
    .map(|(label, metric)| {
        let player = players.values().max_by(|a, b| {
            metric_value(totals.get(&a.id), metric)
                .partial_cmp(&metric_value(totals.get(&b.id), metric))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.overall.cmp(&b.overall))
        });
        leader_from_player(label, player, totals, abbrevs, true)
    })
    .collect()
}

fn leader_from_player(
    label: &'static str,
    player: Option<&Player>,
    totals: &HashMap<PlayerId, PlayerTotals>,
    abbrevs: &HashMap<TeamId, String>,
    include_team: bool,
) -> LeaderRow {
    let Some(p) = player else {
        return LeaderRow {
            metric: label,
            name: "-".to_string(),
            abbrev: None,
            value: 0.0,
        };
    };
    let row = totals.get(&p.id);
    let team = row.and_then(|r| r.team).or(p.team);
    LeaderRow {
        metric: label,
        name: clean_name(&p.name),
        abbrev: if include_team {
            team.and_then(|id| abbrevs.get(&id).cloned())
        } else {
            None
        },
        value: match label {
            "PPG" => metric_value(row, Metric::Ppg),
            "RPG" => metric_value(row, Metric::Rpg),
            "APG" => metric_value(row, Metric::Apg),
            _ => 0.0,
        },
    }
}

#[derive(Copy, Clone)]
enum Metric {
    Ppg,
    Rpg,
    Apg,
}

fn metric_value(row: Option<&PlayerTotals>, metric: Metric) -> f32 {
    let Some(row) = row else { return 0.0 };
    match metric {
        Metric::Ppg => row.ppg(),
        Metric::Rpg => row.rpg(),
        Metric::Apg => row.apg(),
    }
}

fn team_stat_rows(
    user_team: TeamId,
    teams: &[nba3k_core::Team],
    totals: &HashMap<TeamId, TeamTotals>,
) -> Vec<StatRow> {
    let user = totals.get(&user_team).cloned().unwrap_or_default();
    vec![
        StatRow {
            label: T::HomeStatPoints,
            value: user.ppg(),
            rank: rank_team(teams, totals, |t| t.ppg(), true, user_team),
        },
        StatRow {
            label: T::HomeStatAllowed,
            value: user.oppg(),
            rank: rank_team(teams, totals, |t| t.oppg(), false, user_team),
        },
        StatRow {
            label: T::HomeStatRebounds,
            value: user.rpg(),
            rank: rank_team(teams, totals, |t| t.rpg(), true, user_team),
        },
        StatRow {
            label: T::HomeStatAssists,
            value: user.apg(),
            rank: rank_team(teams, totals, |t| t.apg(), true, user_team),
        },
    ]
}

fn rank_team<F>(
    teams: &[nba3k_core::Team],
    totals: &HashMap<TeamId, TeamTotals>,
    value: F,
    high_best: bool,
    user_team: TeamId,
) -> usize
where
    F: Fn(&TeamTotals) -> f32,
{
    let mut rows: Vec<(TeamId, f32)> = teams
        .iter()
        .map(|team| {
            let totals = totals.get(&team.id).cloned().unwrap_or_default();
            (team.id, value(&totals))
        })
        .collect();
    rows.sort_by(|a, b| {
        let ord = a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal);
        if high_best { ord.reverse() } else { ord }
    });
    rows.iter()
        .position(|(team, _)| *team == user_team)
        .map(|idx| idx + 1)
        .unwrap_or(1)
}

fn finance_rows(payroll: Cents, season: SeasonId, user_team: TeamId) -> Vec<(T, String)> {
    let league_year = LeagueYear::for_season(season).unwrap_or_else(|| LeagueYear {
        season,
        cap: Cents::ZERO,
        tax: Cents::ZERO,
        apron_1: Cents::ZERO,
        apron_2: Cents::ZERO,
        mle_non_taxpayer: Cents::ZERO,
        mle_taxpayer: Cents::ZERO,
        mle_room: Cents::ZERO,
        bae: Cents::ZERO,
        min_team_salary: Cents::ZERO,
        max_trade_cash: Cents::ZERO,
    });
    let attendance = 16_000 + ((user_team.0 as u32 * 379) % 3_000);
    let revenue = Cents(payroll.0.saturating_mul(13) / 10);
    let operating = Cents::from_dollars(18_000_000);
    let profit = revenue - payroll - operating;
    let cash = Cents::from_dollars(150_000_000 + user_team.0 as i64 * 1_000_000);
    vec![
        (T::FinanceAvgAttendance, format_number(attendance)),
        (T::FinanceRevenueYTD, money_cell(revenue)),
        (T::FinanceProfitYTD, money_cell(profit)),
        (T::FinanceCash, money_cell(cash)),
        (T::FinancePayroll, money_cell(payroll)),
        (T::FinanceCap, money_cell(league_year.cap)),
    ]
}

fn lineup_rows(
    starters: nba3k_core::Starters,
    _roster: &[Player],
    players: &HashMap<PlayerId, Player>,
    totals: &HashMap<PlayerId, PlayerTotals>,
) -> Vec<LineupRow> {
    Position::all()
        .into_iter()
        .map(|pos| {
            let pid = starters.slot(pos);
            let player = pid.and_then(|id| players.get(&id));
            let stats = pid.and_then(|id| totals.get(&id));
            LineupRow {
                position: pos,
                name: player.map(|p| clean_name(&p.name)).unwrap_or_else(|| "-".to_string()),
                ppg: stats.map(PlayerTotals::ppg).unwrap_or(0.0),
                rpg: stats.map(PlayerTotals::rpg).unwrap_or(0.0),
                apg: stats.map(PlayerTotals::apg).unwrap_or(0.0),
                mpg: stats.map(PlayerTotals::mpg).unwrap_or(0.0),
            }
        })
        .collect()
}

fn per_game(num: u32, gp: u32) -> f32 {
    if gp == 0 { 0.0 } else { num as f32 / gp as f32 }
}

fn ordinal(n: usize) -> String {
    let suffix = if (11..=13).contains(&(n % 100)) {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{}{}", n, suffix)
}

fn money_cell(c: Cents) -> String {
    if c == Cents::ZERO {
        "-".to_string()
    } else {
        format!("{}", c)
    }
}

fn format_number(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (idx, ch) in s.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short_name(name: &str) -> String {
    let clean = clean_name(name);
    let mut parts = clean.split_whitespace().collect::<Vec<_>>();
    if parts.len() <= 1 {
        clean
    } else {
        parts.pop().unwrap_or("").to_string()
    }
}

pub fn handle_key(
    _app: &mut AppState,
    _tui: &mut TuiApp,
    _key: KeyEvent,
) -> Result<bool> {
    Ok(false)
}
