//! Roster screen (M21). Two sub-tabs:
//!
//! 1. **My Roster** — sortable player table (OVR/Pos/Age/Salary), per-row
//!    actions for Train / Extend / Cut / Role, and a Player Detail modal
//!    (Stats / Career / Contract / Chemistry).
//! 2. **Free Agents** — sortable FA pool, single-action Sign.
//!
//! Mirrors `home.rs`'s thread_local-RefCell cache pattern so the table is
//! cheap to redraw between key events. `invalidate()` busts the cache after
//! mutations or when other screens (e.g. trade post-process) need a refresh.
//! All state mutations route through `commands::dispatch` wrapped in
//! `with_silenced_io` so inner `println!`s don't corrupt the alt-screen.
//!
//! Key bindings (My Roster tab):
//!   Tab / 1/2  — switch sub-tab        ↑ / ↓        — move row cursor
//!   PgUp/PgDn  — ±10 rows              Enter        — Player Detail modal
//!   o p a s    — sort (OVR/Pos/Age/Sal) t/e/x/R     — Train/Extend/Cut/Role
//!   Esc        — close modal / back to menu
//!
//! Key bindings (Free Agents tab):
//!   ↑ / ↓ / PgUp / PgDn  — move           s — Sign
//!
//! Esc semantics: any open modal swallows the Esc; otherwise the shell
//! returns the user to the menu.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Clear, Paragraph, Row, Table},
    Frame,
};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::cli::{Command, FaAction, FaArgs};
use crate::state::AppState;
use crate::tui::widgets::{
    centered_block, kv_table, ActionBar, Confirm, FormWidget, NumberInput, Picker, Theme,
    WidgetEvent,
};
use crate::tui::{with_silenced_io, TuiApp};
use nba3k_core::{
    Cents, ContractYear, Player, PlayerId, PlayerRole, Position, SeasonId, TeamId,
};
use nba3k_season::career::{career_totals, SeasonAvgRow};

// Roster cap mirrors `commands::FA_ROSTER_CAP` (15 std + 3 two-way = 18).
const FA_ROSTER_CAP: usize = 18;

// ---------------------------------------------------------------------------
// Public types & cache
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SubTab {
    Roster,
    FreeAgents,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SortKey {
    Ovr,         // o — default
    Position,    // p
    Age,         // a
    Salary,      // s
}

#[derive(Clone, Debug)]
struct RosterRow {
    player_id: PlayerId,
    name: String,
    position: Position,
    age: u8,
    overall: u8,
    role: PlayerRole,
    salary_cents: i64,
    cap_pct: f32, // 0..100
    ppg: f32,
    rpg: f32,
    apg: f32,
}

#[derive(Clone, Debug)]
struct FaRow {
    player_id: PlayerId,
    name: String,
    position: Position,
    age: u8,
    overall: u8,
    asking_m: f32, // estimated $/yr in millions
}

#[derive(Default, Clone, Debug)]
struct DetailData {
    /// Header line: name + jersey/team + age + position + OVR.
    title: String,
    /// "Stats" panel rows (current-season averages).
    stats: Vec<(&'static str, String)>,
    /// "Career" panel rows (year-by-year totals + career line).
    career: Vec<CareerLine>,
    /// "Contract" panel rows (year-by-year salary).
    contract: Vec<(String, String)>,
    /// Flags (NTC / kicker) appended above the contract table.
    flags: Vec<String>,
    /// Chemistry panel — placeholder for now (team-level score).
    chemistry: Vec<(&'static str, String)>,
}

#[derive(Clone, Debug)]
struct CareerLine {
    season_label: String,
    team_abbrev: String,
    gp: u32,
    ppg: f32,
    rpg: f32,
    apg: f32,
}

#[derive(Default)]
struct RosterCache {
    /// Cached ordered roster rows for the active sort.
    rows: Option<Vec<RosterRow>>,
    /// Cached FA rows.
    fa_rows: Option<Vec<FaRow>>,
    /// Memoized detail data, keyed by player id.
    details: HashMap<PlayerId, DetailData>,

    /// Active sub-tab.
    tab: SubTab,
    /// Active sort key for My Roster tab.
    sort: SortKey,
    /// Cursor on My Roster tab.
    roster_cursor: usize,
    /// Cursor on Free Agents tab.
    fa_cursor: usize,
    /// Active modal — stacked on top of the table.
    modal: Modal,
}

impl Default for SubTab {
    fn default() -> Self { SubTab::Roster }
}
impl Default for SortKey {
    fn default() -> Self { SortKey::Ovr }
}

#[derive(Default)]
enum Modal {
    #[default]
    None,
    /// Train: pick focus from `["shoot","inside","def","reb","ath","handle"]`.
    Train { picker: Picker<&'static str>, target_id: PlayerId, target_name: String },
    /// Extend step 1: salary in $M (NumberInput holds whole millions; we
    /// convert + bound when the user submits).
    ExtendSalary { input: NumberInput, target_id: PlayerId, target_name: String },
    /// Extend step 2: years.
    ExtendYears { input: NumberInput, target_id: PlayerId, target_name: String, salary_m: i64 },
    /// Cut confirm.
    Cut { confirm: Confirm, target_id: PlayerId, target_name: String },
    /// Role assign.
    Role { picker: Picker<&'static str>, target_id: PlayerId, target_name: String },
    /// FA sign confirm.
    Sign { confirm: Confirm, target_id: PlayerId, target_name: String },
    /// Player Detail overlay.
    Detail { player_id: PlayerId },
}

thread_local! {
    static CACHE: RefCell<RosterCache> = RefCell::new(RosterCache::default());
}

/// Drop the cached roster/FA rows + detail map. Called from this screen
/// after mutations, and exposed for cross-screen invalidation (e.g. a trade
/// completed elsewhere should bust this).
pub fn invalidate() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.rows = None;
        c.fa_rows = None;
        c.details.clear();
        // Preserve cursor + tab + sort + open modal — only data is stale.
    });
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    if !tui.has_save() {
        let p = Paragraph::new("No save loaded — use the wizard to start a game.")
            .block(theme.block(" Roster "));
        f.render_widget(p, area);
        return;
    }
    if let Err(e) = ensure_cache(app, tui) {
        let p = Paragraph::new(format!("Roster unavailable: {}", e))
            .block(theme.block(" Roster "));
        f.render_widget(p, area);
        return;
    }

    // Layout: tab strip (3) | table (rest).
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let tab = CACHE.with(|c| c.borrow().tab);
    draw_tab_strip(f, parts[0], theme, tab);

    match tab {
        SubTab::Roster => draw_roster_tab(f, parts[1], theme),
        SubTab::FreeAgents => draw_fa_tab(f, parts[1], theme),
    }

    // Modal overlay (after tab body so it draws on top).
    let need_modal = CACHE.with(|c| !matches!(c.borrow().modal, Modal::None));
    if need_modal {
        let rect = modal_rect(area);
        // Wipe under the modal so the underlying roster table doesn't bleed
        // through (Paragraph::new("") doesn't actually clear cells).
        f.render_widget(Clear, rect);
        draw_modal(f, rect, theme, app);
    }
}

fn draw_tab_strip(f: &mut Frame, area: Rect, theme: &Theme, tab: SubTab) {
    let (a_style, b_style) = match tab {
        SubTab::Roster => (theme.highlight(), theme.muted_style()),
        SubTab::FreeAgents => (theme.muted_style(), theme.highlight()),
    };
    let line = Line::from(vec![
        Span::styled(" 1. My Roster ", a_style),
        Span::styled("   ", theme.text()),
        Span::styled(" 2. Free Agents ", b_style),
    ]);
    let p = Paragraph::new(line).block(theme.block(" Roster "));
    f.render_widget(p, area);
}

fn draw_roster_tab(f: &mut Frame, area: Rect, theme: &Theme) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let rows = cache.rows.as_deref().unwrap_or(&[]);
        let cursor = cache.roster_cursor.min(rows.len().saturating_sub(1));

        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(area);

        let header = Row::new(vec![
            Cell::from(Span::styled("#", theme.accent_style())),
            Cell::from(Span::styled("NAME", theme.accent_style())),
            Cell::from(Span::styled("POS", theme.accent_style())),
            Cell::from(Span::styled("AGE", theme.accent_style())),
            Cell::from(Span::styled("OVR", theme.accent_style())),
            Cell::from(Span::styled("PPG", theme.accent_style())),
            Cell::from(Span::styled("RPG", theme.accent_style())),
            Cell::from(Span::styled("APG", theme.accent_style())),
            Cell::from(Span::styled("ROLE", theme.accent_style())),
            Cell::from(Span::styled("CAP%", theme.accent_style())),
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
                    Cell::from(Span::styled(format!("{:.1}", r.ppg), style)),
                    Cell::from(Span::styled(format!("{:.1}", r.rpg), style)),
                    Cell::from(Span::styled(format!("{:.1}", r.apg), style)),
                    Cell::from(Span::styled(short_role(r.role), style)),
                    Cell::from(Span::styled(format!("{:.1}%", r.cap_pct), style)),
                ])
            })
            .collect();

        let title = format!(
            " My Roster ({}) — sort: {} ",
            rows.len(),
            sort_label(cache.sort),
        );
        let table = Table::new(
            body,
            [
                Constraint::Length(3),  // #
                Constraint::Min(20),    // name
                Constraint::Length(4),  // pos
                Constraint::Length(4),  // age
                Constraint::Length(4),  // ovr
                Constraint::Length(5),  // ppg
                Constraint::Length(5),  // rpg
                Constraint::Length(5),  // apg
                Constraint::Length(7),  // role
                Constraint::Length(7),  // cap%
            ],
        )
        .header(header)
        .block(theme.block(&title));
        f.render_widget(table, parts[0]);

        let bar = ActionBar::new(&[
            ("t", "Train"),
            ("e", "Extend"),
            ("x", "Cut"),
            ("R", "Role"),
            ("Enter", "Detail"),
            ("o/p/a/s", "Sort"),
            ("Tab", "FA"),
            ("Esc", "Back"),
        ]);
        bar.render(f, parts[1], theme);
    });
}

fn draw_fa_tab(f: &mut Frame, area: Rect, theme: &Theme) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let rows = cache.fa_rows.as_deref().unwrap_or(&[]);
        let cursor = cache.fa_cursor.min(rows.len().saturating_sub(1));

        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(area);

        let header = Row::new(vec![
            Cell::from(Span::styled("#", theme.accent_style())),
            Cell::from(Span::styled("NAME", theme.accent_style())),
            Cell::from(Span::styled("POS", theme.accent_style())),
            Cell::from(Span::styled("AGE", theme.accent_style())),
            Cell::from(Span::styled("OVR", theme.accent_style())),
            Cell::from(Span::styled("ASKING", theme.accent_style())),
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

        let title = format!(" Free Agents ({}) — sort: OVR ", rows.len());
        let table = Table::new(
            body,
            [
                Constraint::Length(3),
                Constraint::Min(20),
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(8),
            ],
        )
        .header(header)
        .block(theme.block(&title));
        f.render_widget(table, parts[0]);

        let bar = ActionBar::new(&[
            ("s", "Sign"),
            ("Tab", "Roster"),
            ("Esc", "Back"),
        ]);
        bar.render(f, parts[1], theme);
    });
}

fn modal_rect(area: Rect) -> Rect {
    let w = area.width.saturating_sub(8).min(96).max(40);
    let h = area.height.saturating_sub(4).min(28).max(8);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}

fn draw_modal(f: &mut Frame, rect: Rect, theme: &Theme, app: &mut AppState) {
    // Pull whatever the modal needs OUT of the cache borrow first, then render
    // — keeps `app` reachable for the Detail modal path which doesn't really
    // touch app today but keeps the signature uniform for future panels.
    enum DrawSpec {
        None,
        TrainOrRolePicker { title: String, picker: Picker<&'static str> },
        ExtendSalary { input: NumberInput, target_name: String },
        ExtendYears { input: NumberInput, target_name: String, salary_m: i64 },
        Confirm(Confirm),
        Detail { player_id: PlayerId, detail: DetailData },
    }

    let spec = CACHE.with(|c| {
        let cache = c.borrow();
        match &cache.modal {
            Modal::None => DrawSpec::None,
            Modal::Train { picker, target_name, .. } => DrawSpec::TrainOrRolePicker {
                title: format!(" Train: {}", target_name),
                picker: picker.clone(),
            },
            Modal::ExtendSalary { input, target_name, .. } => DrawSpec::ExtendSalary {
                input: input.clone(),
                target_name: target_name.clone(),
            },
            Modal::ExtendYears { input, target_name, salary_m, .. } => DrawSpec::ExtendYears {
                input: input.clone(),
                target_name: target_name.clone(),
                salary_m: *salary_m,
            },
            Modal::Cut { confirm, .. } => DrawSpec::Confirm(confirm.clone()),
            Modal::Role { picker, target_name, .. } => DrawSpec::TrainOrRolePicker {
                title: format!(" Role: {}", target_name),
                picker: picker.clone(),
            },
            Modal::Sign { confirm, .. } => DrawSpec::Confirm(confirm.clone()),
            Modal::Detail { player_id } => DrawSpec::Detail {
                player_id: *player_id,
                detail: cache.details.get(player_id).cloned().unwrap_or_default(),
            },
        }
    });

    match spec {
        DrawSpec::None => {}
        DrawSpec::TrainOrRolePicker { title, picker } => {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0)])
                .split(rect);
            let head = Paragraph::new(Line::from(Span::styled(title, theme.accent_style())))
                .block(theme.block(""));
            f.render_widget(head, parts[0]);
            picker.render(f, parts[1], theme);
        }
        DrawSpec::ExtendSalary { input, target_name } => {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
                .split(rect);
            let head = Paragraph::new(Line::from(Span::styled(
                format!(" Extend: {} (1/2)", target_name),
                theme.accent_style(),
            )))
            .block(theme.block(""));
            f.render_widget(head, parts[0]);
            input.render(f, parts[1], theme);
            let hint = Paragraph::new(Line::from(Span::styled(
                "Salary in $M (1-300). Enter to advance, Esc to cancel.",
                theme.muted_style(),
            )))
            .block(theme.block(""));
            f.render_widget(hint, parts[2]);
        }
        DrawSpec::ExtendYears { input, target_name, salary_m } => {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
                .split(rect);
            let head = Paragraph::new(Line::from(Span::styled(
                format!(" Extend: {} (2/2) — ${} M/yr", target_name, salary_m),
                theme.accent_style(),
            )))
            .block(theme.block(""));
            f.render_widget(head, parts[0]);
            input.render(f, parts[1], theme);
            let hint = Paragraph::new(Line::from(Span::styled(
                "Years (1-5). Enter to submit, Esc to cancel.",
                theme.muted_style(),
            )))
            .block(theme.block(""));
            f.render_widget(hint, parts[2]);
        }
        DrawSpec::Confirm(c) => {
            c.render(f, rect, theme);
        }
        DrawSpec::Detail { player_id, detail } => {
            draw_detail_modal(f, rect, theme, app, player_id, &detail);
        }
    }
}

fn draw_detail_modal(
    f: &mut Frame,
    rect: Rect,
    theme: &Theme,
    _app: &mut AppState,
    _player_id: PlayerId,
    d: &DetailData,
) {
    // Vertical: header (3) | 2x2 panel grid (rest) | action bar (3).
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(rect);

    let head = Paragraph::new(Line::from(Span::styled(d.title.clone(), theme.accent_style())))
        .block(theme.block(""));
    f.render_widget(head, parts[0]);

    // 2x2 grid.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(parts[1]);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);
    let bot = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    // Stats panel — kv_table.
    let stats_rows: Vec<(&str, String)> = d.stats.iter().map(|(k, v)| (*k, v.clone())).collect();
    let stats_table = kv_table(&stats_rows, theme, " Stats ");
    f.render_widget(stats_table, top[0]);

    // Career panel — line list.
    {
        let mut lines: Vec<Line> = Vec::with_capacity(d.career.len() + 2);
        lines.push(Line::from(Span::styled(
            format!(
                "{:<8} {:<5} {:>3}  {:>4} {:>4} {:>4}",
                "SEASON", "TM", "GP", "PPG", "RPG", "APG"
            ),
            theme.accent_style(),
        )));
        for c in &d.career {
            lines.push(Line::from(Span::styled(
                format!(
                    "{:<8} {:<5} {:>3}  {:>4.1} {:>4.1} {:>4.1}",
                    c.season_label, c.team_abbrev, c.gp, c.ppg, c.rpg, c.apg
                ),
                theme.text(),
            )));
        }
        if d.career.is_empty() {
            lines.push(Line::from(Span::styled(
                "(no games played yet)",
                theme.muted_style(),
            )));
        }
        let p = Paragraph::new(lines).block(theme.block(" Career "));
        f.render_widget(p, top[1]);
    }

    // Contract panel — flags + year list.
    {
        let mut lines: Vec<Line> = Vec::with_capacity(d.contract.len() + d.flags.len() + 1);
        for flag in &d.flags {
            lines.push(Line::from(Span::styled(
                format!("flag: {}", flag),
                theme.accent_style(),
            )));
        }
        if !d.flags.is_empty() {
            lines.push(Line::from(""));
        }
        if d.contract.is_empty() {
            lines.push(Line::from(Span::styled(
                "(no contract)",
                theme.muted_style(),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!("{:<8} {:>14}", "SEASON", "SALARY"),
                theme.accent_style(),
            )));
            for (season, salary) in &d.contract {
                lines.push(Line::from(Span::styled(
                    format!("{:<8} {:>14}", season, salary),
                    theme.text(),
                )));
            }
        }
        let p = Paragraph::new(lines).block(theme.block(" Contract "));
        f.render_widget(p, bot[0]);
    }

    // Chemistry panel — kv_table.
    let chem_rows: Vec<(&str, String)> = d.chemistry.iter().map(|(k, v)| (*k, v.clone())).collect();
    if chem_rows.is_empty() {
        centered_block(f, bot[1], theme, " Chemistry ", &["(no data)"]);
    } else {
        let chem_table = kv_table(&chem_rows, theme, " Chemistry ");
        f.render_widget(chem_table, bot[1]);
    }

    let bar = ActionBar::new(&[
        ("t", "Train"),
        ("e", "Extend"),
        ("x", "Cut"),
        ("R", "Role"),
        ("Esc", "Close"),
    ]);
    bar.render(f, parts[2], theme);
}

// ---------------------------------------------------------------------------
// Cache population
// ---------------------------------------------------------------------------

fn ensure_cache(app: &mut AppState, tui: &TuiApp) -> Result<()> {
    let need_rows = CACHE.with(|c| c.borrow().rows.is_none());
    let need_fa = CACHE.with(|c| c.borrow().fa_rows.is_none());
    if need_rows {
        let rows = build_roster_rows(app, tui)?;
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            c.rows = Some(rows);
            apply_sort(&mut c);
        });
    }
    if need_fa {
        let rows = build_fa_rows(app)?;
        CACHE.with(|c| c.borrow_mut().fa_rows = Some(rows));
    }
    Ok(())
}

fn build_roster_rows(app: &mut AppState, tui: &TuiApp) -> Result<Vec<RosterRow>> {
    let store = app.store()?;
    let user_team = tui.user_team;
    let season = tui.season;

    let roster: Vec<Player> = store.roster_for_team(user_team)?;
    let payroll = store.team_salary(user_team, season)?;
    let payroll_dollars = payroll.as_dollars().max(1);

    // Pull the season averages once per player. read_career_stats walks every
    // season recorded in `games`, so we filter to the current season here.
    let mut out: Vec<RosterRow> = Vec::with_capacity(roster.len());
    for p in &roster {
        let season_stats = store
            .read_career_stats(p.id)
            .unwrap_or_default()
            .into_iter()
            .find(|r| r.season == season);
        let (ppg, rpg, apg) = match season_stats {
            Some(r) => (r.ppg(), r.rpg(), r.apg()),
            None => (0.0, 0.0, 0.0),
        };
        let salary = p
            .contract
            .as_ref()
            .map(|c| c.salary_for(season))
            .unwrap_or(Cents::ZERO);
        let salary_cents = salary.as_dollars();
        let cap_pct = (salary_cents as f32 / payroll_dollars as f32) * 100.0;

        out.push(RosterRow {
            player_id: p.id,
            name: clean_name(&p.name),
            position: p.primary_position,
            age: p.age,
            overall: p.overall,
            role: p.role,
            salary_cents,
            cap_pct,
            ppg,
            rpg,
            apg,
        });
    }
    Ok(out)
}

fn build_fa_rows(app: &mut AppState) -> Result<Vec<FaRow>> {
    let store = app.store()?;
    let pool = store.list_free_agents()?;
    Ok(pool
        .into_iter()
        .map(|p| FaRow {
            player_id: p.id,
            name: clean_name(&p.name),
            position: p.primary_position,
            age: p.age,
            overall: p.overall,
            asking_m: estimate_asking_m(p.overall),
        })
        .collect())
}

/// Estimate asking salary in $M using the same OVR tiers as
/// `nba3k_models::contract_gen::generate_contract` (private there). Mirror
/// kept tiny — only the per-year midpoint, not length.
fn estimate_asking_m(ovr: u8) -> f32 {
    match ovr {
        90..=u8::MAX => 55.0,
        85..=89 => 30.0,
        80..=84 => 15.0,
        70..=79 => 5.0,
        _ => 2.0,
    }
}

fn apply_sort(cache: &mut RosterCache) {
    let Some(rows) = cache.rows.as_mut() else { return };
    match cache.sort {
        SortKey::Ovr => rows.sort_by(|a, b| {
            b.overall.cmp(&a.overall).then_with(|| a.name.cmp(&b.name))
        }),
        SortKey::Position => rows.sort_by(|a, b| {
            position_order(a.position)
                .cmp(&position_order(b.position))
                .then_with(|| b.overall.cmp(&a.overall))
        }),
        SortKey::Age => rows.sort_by(|a, b| {
            a.age.cmp(&b.age).then_with(|| b.overall.cmp(&a.overall))
        }),
        SortKey::Salary => rows.sort_by(|a, b| {
            b.salary_cents
                .cmp(&a.salary_cents)
                .then_with(|| b.overall.cmp(&a.overall))
        }),
    }
    cache.roster_cursor = cache
        .roster_cursor
        .min(rows.len().saturating_sub(1));
}

fn position_order(p: Position) -> u8 {
    match p {
        Position::PG => 1,
        Position::SG => 2,
        Position::SF => 3,
        Position::PF => 4,
        Position::C => 5,
    }
}

fn ensure_detail(app: &mut AppState, tui: &TuiApp, player_id: PlayerId) -> Result<()> {
    let already = CACHE.with(|c| c.borrow().details.contains_key(&player_id));
    if already {
        return Ok(());
    }
    let detail = build_detail(app, tui, player_id)?;
    CACHE.with(|c| {
        c.borrow_mut().details.insert(player_id, detail);
    });
    Ok(())
}

fn build_detail(app: &mut AppState, tui: &TuiApp, player_id: PlayerId) -> Result<DetailData> {
    let store = app.store()?;
    let season = tui.season;

    // Player name lookup; if the player is missing from the DB (cut mid-flow),
    // return a stub detail so the modal still renders something useful.
    let name = match store.player_name(player_id)? {
        Some(n) => n,
        None => {
            return Ok(DetailData {
                title: format!("Player #{}", player_id.0),
                ..DetailData::default()
            });
        }
    };
    let player = store
        .find_player_by_name(&name)?
        .unwrap_or_else(|| Player {
            id: player_id,
            name: name.clone(),
            primary_position: Position::SF,
            secondary_position: None,
            age: 0,
            overall: 0,
            potential: 0,
            ratings: Default::default(),
            contract: None,
            team: None,
            injury: None,
            no_trade_clause: false,
            trade_kicker_pct: None,
            role: PlayerRole::RolePlayer,
            morale: 0.5,
        });
    let team_label = match player.team {
        Some(id) => store.team_abbrev(id)?.unwrap_or_else(|| format!("T{}", id.0)),
        None => "FA".into(),
    };
    let title = format!(
        " {} | {} | age {} | {} | OVR {} ",
        clean_name(&player.name),
        team_label,
        player.age,
        player.primary_position,
        player.overall
    );

    // Stats panel — current-season averages (zeros if not yet played).
    let career: Vec<SeasonAvgRow> = store.read_career_stats(player_id).unwrap_or_default();
    let cur = career.iter().find(|r| r.season == season).cloned();
    let stats: Vec<(&'static str, String)> = match cur.as_ref() {
        Some(r) => vec![
            ("GP", format!("{}", r.gp)),
            ("MPG", format!("{:.1}", r.mpg())),
            ("PPG", format!("{:.1}", r.ppg())),
            ("RPG", format!("{:.1}", r.rpg())),
            ("APG", format!("{:.1}", r.apg())),
            ("SPG", format!("{:.1}", r.spg())),
            ("BPG", format!("{:.1}", r.bpg())),
            ("FG%", fmt_pct(r.fg_pct())),
            ("3P%", fmt_pct(r.three_pct())),
            ("FT%", fmt_pct(r.ft_pct())),
        ],
        None => vec![("GP", "0".into()), ("note", "(no games yet)".into())],
    };

    // Career panel.
    let mut team_abbrev_cache: HashMap<TeamId, String> = HashMap::new();
    let mut career_lines: Vec<CareerLine> = Vec::with_capacity(career.len() + 1);
    for r in &career {
        let tm = match r.team {
            Some(id) => team_abbrev_cache
                .entry(id)
                .or_insert_with(|| {
                    store.team_abbrev(id).ok().flatten().unwrap_or_else(|| format!("T{}", id.0))
                })
                .clone(),
            None => "—".into(),
        };
        career_lines.push(CareerLine {
            season_label: format_season_label(r.season),
            team_abbrev: tm,
            gp: r.gp,
            ppg: r.ppg(),
            rpg: r.rpg(),
            apg: r.apg(),
        });
    }
    if career.len() > 1 {
        let totals = career_totals(&career);
        career_lines.push(CareerLine {
            season_label: "career".into(),
            team_abbrev: "".into(),
            gp: totals.gp,
            ppg: totals.ppg(),
            rpg: totals.rpg(),
            apg: totals.apg(),
        });
    }

    // Contract panel.
    let contract: Vec<(String, String)> = match player.contract.as_ref() {
        Some(c) => c
            .years
            .iter()
            .map(|y: &ContractYear| {
                let mut markers = String::new();
                if y.team_option {
                    markers.push_str(" (TO)");
                }
                if y.player_option {
                    markers.push_str(" (PO)");
                }
                if !y.guaranteed {
                    markers.push_str(" (NG)");
                }
                (
                    format_season_label(y.season),
                    format!("${:.1}M{}", y.salary.as_millions_f32(), markers),
                )
            })
            .collect(),
        None => Vec::new(),
    };
    let mut flags: Vec<String> = Vec::new();
    if player.no_trade_clause {
        flags.push("NTC".into());
    }
    if let Some(pct) = player.trade_kicker_pct {
        flags.push(format!("trade kicker {}%", pct));
    }
    if let Some(i) = player.injury.as_ref() {
        if i.games_remaining > 0 {
            flags.push(format!(
                "INJ: {} ({}g)",
                i.description, i.games_remaining
            ));
        }
    }

    // Chemistry panel — placeholder team-level score (same number for every
    // player in the modal). Sourced from the team_chemistry model so the
    // value is non-trivial.
    let chemistry: Vec<(&'static str, String)> = match player.team {
        Some(team_id) => match team_chemistry_value(app, team_id) {
            Some(v) => vec![
                ("team chem", format!("{:.2}", v)),
                ("morale", format!("{:.2}", player.morale)),
                ("role", short_role(player.role)),
            ],
            None => vec![
                ("morale", format!("{:.2}", player.morale)),
                ("role", short_role(player.role)),
            ],
        },
        None => vec![
            ("morale", format!("{:.2}", player.morale)),
            ("role", short_role(player.role)),
        ],
    };

    Ok(DetailData {
        title,
        stats,
        career: career_lines,
        contract,
        flags,
        chemistry,
    })
}

fn team_chemistry_value(app: &mut AppState, team_id: TeamId) -> Option<f32> {
    use nba3k_models::team_chemistry::team_chemistry;
    use nba3k_trade::snapshot::{LeagueSnapshot, TeamRecordSummary};
    use nba3k_core::{LeagueYear, SeasonPhase};

    let store = app.store().ok()?;
    let state = store.load_season_state().ok()??;
    let teams = store.list_teams().ok()?;
    let players = store.all_active_players().ok()?;
    let picks = store.all_picks().ok()?;
    let standing_rows = store.read_standings(state.season).ok()?;

    let players_by_id = players.into_iter().map(|p| (p.id, p)).collect();
    let picks_by_id = picks.into_iter().map(|p| (p.id, p)).collect();
    let mut standings: HashMap<TeamId, TeamRecordSummary> = HashMap::new();
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
        standings.entry(t.id).or_default();
    }
    let league_year = LeagueYear::for_season(state.season)?;
    // `current_date` only matters for milestone logic the chem model does not
    // consult — pass an arbitrary in-season date; chemistry is invariant.
    let date = chrono::NaiveDate::from_ymd_opt(2025, 10, 14)?;
    let snap = LeagueSnapshot {
        current_season: state.season,
        current_phase: SeasonPhase::Regular,
        current_date: date,
        league_year,
        teams: &teams,
        players_by_id: &players_by_id,
        picks_by_id: &picks_by_id,
        standings: &standings,
    };
    Some(team_chemistry(&snap, team_id).value as f32)
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    // Modal-first.
    let modal_action = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        match &mut c.modal {
            Modal::None => ModalAction::None,
            Modal::Train { picker, target_id, target_name } => match picker.handle_key(key) {
                WidgetEvent::Submitted => match picker.selected().copied() {
                    Some(focus) => ModalAction::TrainSubmit {
                        target_id: *target_id,
                        target_name: target_name.clone(),
                        focus,
                    },
                    None => ModalAction::CloseModal,
                },
                WidgetEvent::Cancelled => ModalAction::CloseModal,
                _ => ModalAction::Pending,
            },
            Modal::ExtendSalary { input, target_id, target_name } => match input.handle_key(key) {
                WidgetEvent::Submitted => match input.value() {
                    Some(salary_m) => ModalAction::ExtendSalaryNext {
                        target_id: *target_id,
                        target_name: target_name.clone(),
                        salary_m,
                    },
                    None => ModalAction::Pending,
                },
                WidgetEvent::Cancelled => ModalAction::CloseModal,
                _ => ModalAction::Pending,
            },
            Modal::ExtendYears { input, target_id, target_name, salary_m } => {
                match input.handle_key(key) {
                    WidgetEvent::Submitted => match input.value() {
                        Some(years) => ModalAction::ExtendSubmit {
                            target_id: *target_id,
                            target_name: target_name.clone(),
                            salary_m: *salary_m,
                            years,
                        },
                        None => ModalAction::Pending,
                    },
                    WidgetEvent::Cancelled => ModalAction::CloseModal,
                    _ => ModalAction::Pending,
                }
            }
            Modal::Cut { confirm, target_id, target_name } => match confirm.handle_key(key) {
                WidgetEvent::Submitted => ModalAction::CutSubmit {
                    target_id: *target_id,
                    target_name: target_name.clone(),
                },
                WidgetEvent::Cancelled => ModalAction::CloseModal,
                _ => ModalAction::Pending,
            },
            Modal::Role { picker, target_id, target_name } => match picker.handle_key(key) {
                WidgetEvent::Submitted => match picker.selected().copied() {
                    Some(role) => ModalAction::RoleSubmit {
                        target_id: *target_id,
                        target_name: target_name.clone(),
                        role,
                    },
                    None => ModalAction::CloseModal,
                },
                WidgetEvent::Cancelled => ModalAction::CloseModal,
                _ => ModalAction::Pending,
            },
            Modal::Sign { confirm, target_id, target_name } => match confirm.handle_key(key) {
                WidgetEvent::Submitted => ModalAction::SignSubmit {
                    target_id: *target_id,
                    target_name: target_name.clone(),
                },
                WidgetEvent::Cancelled => ModalAction::CloseModal,
                _ => ModalAction::Pending,
            },
            Modal::Detail { player_id } => {
                let pid = *player_id;
                // Detail forwards the row-action shortcuts to the underlying
                // table, so the user can train/extend/etc. without closing.
                match key.code {
                    KeyCode::Esc => ModalAction::CloseModal,
                    KeyCode::Char('t') => ModalAction::OpenTrainFromDetail(pid),
                    KeyCode::Char('e') => ModalAction::OpenExtendFromDetail(pid),
                    KeyCode::Char('x') => ModalAction::OpenCutFromDetail(pid),
                    KeyCode::Char('R') => ModalAction::OpenRoleFromDetail(pid),
                    _ => ModalAction::Pending,
                }
            }
        }
    });

    match modal_action {
        ModalAction::None => {}
        ModalAction::Pending => return Ok(true),
        ModalAction::CloseModal => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            return Ok(true);
        }
        ModalAction::TrainSubmit { target_id: _, target_name, focus } => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let res = with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Training {
                        player: target_name.clone(),
                        focus: focus.to_string(),
                    },
                )
            });
            after_mutation(tui, res, &format!("trained {} ({})", target_name, focus));
            return Ok(true);
        }
        ModalAction::ExtendSalaryNext { target_id, target_name, salary_m } => {
            CACHE.with(|c| {
                c.borrow_mut().modal = Modal::ExtendYears {
                    input: NumberInput::new("Years (1-5)")
                        .with_bounds(1, 5)
                        .with_initial(3),
                    target_id,
                    target_name,
                    salary_m,
                };
            });
            return Ok(true);
        }
        ModalAction::ExtendSubmit { target_id: _, target_name, salary_m, years } => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let res = with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Extend {
                        player: target_name.clone(),
                        salary_m: salary_m as f64,
                        years: years as u8,
                    },
                )
            });
            after_mutation(
                tui,
                res,
                &format!("extension submitted for {} (${}M × {}yr)", target_name, salary_m, years),
            );
            return Ok(true);
        }
        ModalAction::CutSubmit { target_id: _, target_name } => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let res = with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Fa(FaArgs {
                        action: FaAction::Cut { player: target_name.clone() },
                    }),
                )
            });
            after_mutation(tui, res, &format!("cut {}", target_name));
            return Ok(true);
        }
        ModalAction::RoleSubmit { target_id: _, target_name, role } => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let res = with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::RosterSetRole {
                        player: target_name.clone(),
                        role: role.to_string(),
                    },
                )
            });
            after_mutation(tui, res, &format!("{} → {}", target_name, role));
            return Ok(true);
        }
        ModalAction::SignSubmit { target_id: _, target_name } => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            // Roster cap pre-check so the user gets a clean error in the
            // status bar instead of the bail! string out of cmd_fa_sign.
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
            let res = with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Fa(FaArgs {
                        action: FaAction::Sign { player: target_name.clone() },
                    }),
                )
            });
            after_mutation(tui, res, &format!("signed {}", target_name));
            return Ok(true);
        }
        ModalAction::OpenTrainFromDetail(pid) => {
            if let Some(name) = roster_name(pid) {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::Train {
                        picker: train_picker(),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            return Ok(true);
        }
        ModalAction::OpenExtendFromDetail(pid) => {
            if let Some(name) = roster_name(pid) {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::ExtendSalary {
                        input: NumberInput::new("Salary $M (1-300)")
                            .with_bounds(1, 300)
                            .with_initial(25),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            return Ok(true);
        }
        ModalAction::OpenCutFromDetail(pid) => {
            if let Some(name) = roster_name(pid) {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::Cut {
                        confirm: Confirm::new(format!(
                            "Cut {}? They become a free agent.",
                            name
                        )),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            return Ok(true);
        }
        ModalAction::OpenRoleFromDetail(pid) => {
            if let Some(name) = roster_name(pid) {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::Role {
                        picker: role_picker(),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            return Ok(true);
        }
    }

    // No modal — table-level keys.
    let tab = CACHE.with(|c| c.borrow().tab);

    // Sub-tab toggles.
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            c.tab = match c.tab {
                SubTab::Roster => SubTab::FreeAgents,
                SubTab::FreeAgents => SubTab::Roster,
            };
        });
        return Ok(true);
    }
    if matches!(key.code, KeyCode::Char('1')) {
        CACHE.with(|c| c.borrow_mut().tab = SubTab::Roster);
        return Ok(true);
    }
    if matches!(key.code, KeyCode::Char('2')) {
        CACHE.with(|c| c.borrow_mut().tab = SubTab::FreeAgents);
        return Ok(true);
    }

    match tab {
        SubTab::Roster => roster_tab_key(app, tui, key),
        SubTab::FreeAgents => fa_tab_key(app, tui, key),
    }
}

fn roster_tab_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Up => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                if c.roster_cursor > 0 {
                    c.roster_cursor -= 1;
                }
            });
            Ok(true)
        }
        KeyCode::Down => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let len = c.rows.as_ref().map(|r| r.len()).unwrap_or(0);
                if c.roster_cursor + 1 < len {
                    c.roster_cursor += 1;
                }
            });
            Ok(true)
        }
        KeyCode::PageUp => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.roster_cursor = c.roster_cursor.saturating_sub(10);
            });
            Ok(true)
        }
        KeyCode::PageDown => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let len = c.rows.as_ref().map(|r| r.len()).unwrap_or(0);
                c.roster_cursor = (c.roster_cursor + 10).min(len.saturating_sub(1));
            });
            Ok(true)
        }
        KeyCode::Char('o') => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.sort = SortKey::Ovr;
                apply_sort(&mut c);
            });
            Ok(true)
        }
        KeyCode::Char('p') => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.sort = SortKey::Position;
                apply_sort(&mut c);
            });
            Ok(true)
        }
        KeyCode::Char('a') => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.sort = SortKey::Age;
                apply_sort(&mut c);
            });
            Ok(true)
        }
        KeyCode::Char('s') => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.sort = SortKey::Salary;
                apply_sort(&mut c);
            });
            Ok(true)
        }
        KeyCode::Enter => {
            // Open Player Detail modal for the cursor row.
            let target = current_roster_row();
            if let Some((pid, _name)) = target {
                if let Err(e) = ensure_detail(app, tui, pid) {
                    tui.last_msg = Some(format!("detail unavailable: {}", e));
                } else {
                    CACHE.with(|c| {
                        c.borrow_mut().modal = Modal::Detail { player_id: pid };
                    });
                }
            }
            Ok(true)
        }
        KeyCode::Char('t') => {
            if let Some((pid, name)) = current_roster_row() {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::Train {
                        picker: train_picker(),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            Ok(true)
        }
        KeyCode::Char('e') => {
            if let Some((pid, name)) = current_roster_row() {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::ExtendSalary {
                        input: NumberInput::new("Salary $M (1-300)")
                            .with_bounds(1, 300)
                            .with_initial(25),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            Ok(true)
        }
        KeyCode::Char('x') => {
            if let Some((pid, name)) = current_roster_row() {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::Cut {
                        confirm: Confirm::new(format!(
                            "Cut {}? They become a free agent.",
                            name
                        )),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            Ok(true)
        }
        KeyCode::Char('R') => {
            if let Some((pid, name)) = current_roster_row() {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::Role {
                        picker: role_picker(),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn fa_tab_key(_app: &mut AppState, _tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Up => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                if c.fa_cursor > 0 {
                    c.fa_cursor -= 1;
                }
            });
            Ok(true)
        }
        KeyCode::Down => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let len = c.fa_rows.as_ref().map(|r| r.len()).unwrap_or(0);
                if c.fa_cursor + 1 < len {
                    c.fa_cursor += 1;
                }
            });
            Ok(true)
        }
        KeyCode::PageUp => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.fa_cursor = c.fa_cursor.saturating_sub(10);
            });
            Ok(true)
        }
        KeyCode::PageDown => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                let len = c.fa_rows.as_ref().map(|r| r.len()).unwrap_or(0);
                c.fa_cursor = (c.fa_cursor + 10).min(len.saturating_sub(1));
            });
            Ok(true)
        }
        KeyCode::Char('s') => {
            if let Some((pid, name, asking_m)) = current_fa_row() {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::Sign {
                        confirm: Confirm::new(format!(
                            "Sign {}? Estimated cap: ${:.1}M/yr.",
                            name, asking_m
                        )),
                        target_id: pid,
                        target_name: name,
                    };
                });
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn current_roster_row() -> Option<(PlayerId, String)> {
    CACHE.with(|c| {
        let c = c.borrow();
        c.rows
            .as_ref()
            .and_then(|rows| rows.get(c.roster_cursor))
            .map(|r| (r.player_id, r.name.clone()))
    })
}

fn current_fa_row() -> Option<(PlayerId, String, f32)> {
    CACHE.with(|c| {
        let c = c.borrow();
        c.fa_rows
            .as_ref()
            .and_then(|rows| rows.get(c.fa_cursor))
            .map(|r| (r.player_id, r.name.clone(), r.asking_m))
    })
}

fn roster_name(pid: PlayerId) -> Option<String> {
    CACHE.with(|c| {
        let c = c.borrow();
        c.rows
            .as_ref()
            .and_then(|rows| rows.iter().find(|r| r.player_id == pid))
            .map(|r| r.name.clone())
    })
}

fn after_mutation(tui: &mut TuiApp, res: Result<()>, success_msg: &str) {
    match res {
        Ok(()) => {
            tui.last_msg = Some(success_msg.into());
        }
        Err(e) => {
            tui.last_msg = Some(format!("error: {}", e));
        }
    }
    invalidate();
    // Home + saves caches share the load — bust their data too so a roster
    // change (e.g. cut leading to morale shift) shows up next visit.
    crate::tui::screens::home::invalidate();
}

fn train_picker() -> Picker<&'static str> {
    Picker::new(
        "Training focus",
        vec!["shoot", "inside", "def", "reb", "ath", "handle"],
        |s| (*s).to_string(),
    )
}

fn role_picker() -> Picker<&'static str> {
    Picker::new(
        "Role",
        vec!["star", "starter", "sixth", "role", "bench", "prospect"],
        |s| (*s).to_string(),
    )
}

// ---------------------------------------------------------------------------
// Modal action enum — flat list of post-key outcomes so we drop the borrow
// before touching `app`/`tui`.
// ---------------------------------------------------------------------------

enum ModalAction {
    None,
    Pending,
    CloseModal,
    TrainSubmit { target_id: PlayerId, target_name: String, focus: &'static str },
    ExtendSalaryNext { target_id: PlayerId, target_name: String, salary_m: i64 },
    ExtendSubmit { target_id: PlayerId, target_name: String, salary_m: i64, years: i64 },
    CutSubmit { target_id: PlayerId, target_name: String },
    RoleSubmit { target_id: PlayerId, target_name: String, role: &'static str },
    SignSubmit { target_id: PlayerId, target_name: String },
    OpenTrainFromDetail(PlayerId),
    OpenExtendFromDetail(PlayerId),
    OpenCutFromDetail(PlayerId),
    OpenRoleFromDetail(PlayerId),
}

// ---------------------------------------------------------------------------
// Formatting helpers (mirror commands.rs helpers).
// ---------------------------------------------------------------------------

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

fn sort_label(s: SortKey) -> &'static str {
    match s {
        SortKey::Ovr => "OVR",
        SortKey::Position => "Pos",
        SortKey::Age => "Age",
        SortKey::Salary => "Salary",
    }
}

fn format_season_label(s: SeasonId) -> String {
    let end_full = s.0;
    if end_full == 0 {
        return "—".into();
    }
    let end_short = end_full % 100;
    format!("{}-{:02}", end_full - 1, end_short)
}

fn fmt_pct(v: f32) -> String {
    if v <= 0.0 {
        return ".000".into();
    }
    let scaled = (v * 1000.0).round() as i32;
    if scaled >= 1000 {
        return "1.000".into();
    }
    format!(".{:03}", scaled)
}
