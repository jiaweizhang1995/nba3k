//! Draft screen (M22). Two sub-tabs:
//!
//! 1. **Board** - top-60 prospect board with scouting fog.
//! 2. **Order** - deterministic lottery/reverse-record order matching the CLI.
//!
//! This module follows the M21 screen pattern: thread-local cache,
//! `render`/`handle_key`/`invalidate`, modal-first key handling, and all
//! mutations routed through `commands::dispatch` under `with_silenced_io`.

use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent};
use rand::Rng;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha8Rng;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Clear, Paragraph, Row, Table},
    Frame,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::cli::{Command, DraftAction, DraftArgs, JsonFlag};
use crate::state::AppState;
use crate::tui::widgets::{centered_block, ActionBar, Confirm, FormWidget, Theme, WidgetEvent};
use crate::tui::{with_silenced_io, TuiApp};
use nba3k_core::{t, Lang, SeasonId, SeasonPhase, TeamId, T};
use nba3k_season::standings::Standings;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SubTab {
    Board,
    Order,
}

impl Default for SubTab {
    fn default() -> Self {
        SubTab::Board
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CacheKey {
    season: SeasonId,
    day: u32,
    phase: SeasonPhase,
}

#[derive(Clone, Debug)]
struct ProspectRow {
    rank: usize,
    name: String,
    position: String,
    age: u8,
    scouted: bool,
    overall: u8,
    potential: u8,
    projected_pick: String,
}

#[derive(Clone, Debug)]
struct OrderRow {
    pick: usize,
    team_id: TeamId,
    abbrev: String,
    full_name: String,
}

#[derive(Clone, Debug, Default)]
struct DraftData {
    prospects: Vec<ProspectRow>,
    order: Vec<OrderRow>,
    scouted_count: usize,
    prospect_total: usize,
    user_next_pick: Option<usize>,
}

#[derive(Default)]
struct DraftCache {
    tab: SubTab,
    board_cursor: usize,
    order_cursor: usize,
    cached_for: Option<CacheKey>,
    data: Option<DraftData>,
    modal: Modal,
}

#[derive(Default)]
enum Modal {
    #[default]
    None,
    Pick {
        confirm: Confirm,
        target_name: String,
    },
    Sim {
        confirm: Confirm,
    },
}

thread_local! {
    static CACHE: RefCell<DraftCache> = RefCell::new(DraftCache::default());
}

/// Drop cached board/order data. Keeps tab/cursor/modal state intact so a
/// refresh after scouting or drafting does not bounce the user around.
pub fn invalidate() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.cached_for = None;
        c.data = None;
    });
}

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    if !tui.has_save() {
        centered_block(
            f,
            area,
            theme,
            t(tui.lang, T::DraftTitle),
            &[
                t(tui.lang, T::DraftTitle),
                "",
                t(tui.lang, T::CommonNoSaveLoaded),
                "",
                "Use New Game or Ctrl+S to load a save.",
            ],
        );
        return;
    }

    if let Err(e) = ensure_cache(app, tui) {
        let p = Paragraph::new(format!("Draft unavailable: {}", e))
            .block(theme.block(t(tui.lang, T::DraftTitle)));
        f.render_widget(p, area);
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(3),
        ])
        .split(area);

    let tab = CACHE.with(|c| c.borrow().tab);
    draw_tab_strip(f, parts[0], theme, tui.lang, tab);
    draw_summary(f, parts[1], theme, tui);
    match tab {
        SubTab::Board => draw_board(f, parts[2], theme, tui.lang),
        SubTab::Order => draw_order(f, parts[2], theme, tui),
    }
    draw_action_bar(f, parts[3], theme, tui, tab);

    let has_modal = CACHE.with(|c| !matches!(c.borrow().modal, Modal::None));
    if has_modal {
        let rect = modal_rect(area);
        f.render_widget(Clear, rect);
        draw_modal(f, rect, theme, tui.lang);
    }
}

fn draw_tab_strip(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, tab: SubTab) {
    let board_style = if tab == SubTab::Board {
        theme.highlight()
    } else {
        theme.muted_style()
    };
    let order_style = if tab == SubTab::Order {
        theme.highlight()
    } else {
        theme.muted_style()
    };
    let line = Line::from(vec![
        Span::styled(format!(" 1. {} ", t(lang, T::DraftBoard)), board_style),
        Span::styled("   ", theme.text()),
        Span::styled(format!(" 2. {} ", t(lang, T::DraftOrder)), order_style),
    ]);
    let p = Paragraph::new(line).block(theme.block(t(lang, T::DraftTitle)));
    f.render_widget(p, area);
}

fn draw_summary(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp) {
    let active = draft_active(tui.season_state.phase);
    let data = CACHE.with(|c| c.borrow().data.clone().unwrap_or_default());
    let pick = data
        .user_next_pick
        .map(|p| format!("#{} {}", p, tui.user_abbrev))
        .unwrap_or_else(|| "no pick in round".to_string());
    let lead = if active {
        Span::styled(t(tui.lang, T::CommonReady), theme.accent_style())
    } else {
        Span::styled(
            t(tui.lang, T::DraftNotActive),
            theme.accent_style(),
        )
    };
    let line = Line::from(vec![
        lead,
        Span::styled(
            format!("  Phase: {:?}", tui.season_state.phase),
            theme.muted_style(),
        ),
        Span::styled(
            format!(
                "  Scouted: {}/{}  Your next pick: {}",
                data.scouted_count, data.prospect_total, pick
            ),
            theme.text(),
        ),
    ]);
    let p = Paragraph::new(line).block(theme.block(""));
    f.render_widget(p, area);
}

fn draw_board(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let data = cache.data.as_ref().cloned().unwrap_or_default();
        let cursor = cache
            .board_cursor
            .min(data.prospects.len().saturating_sub(1));

        let header = Row::new(vec![
            Cell::from(Span::styled("RANK", theme.accent_style())),
            Cell::from(Span::styled(t(lang, T::DraftProspect), theme.accent_style())),
            Cell::from(Span::styled("POS", theme.accent_style())),
            Cell::from(Span::styled("AGE", theme.accent_style())),
            Cell::from(Span::styled("OVR/POT", theme.accent_style())),
            Cell::from(Span::styled(t(lang, T::DraftProjectedPick), theme.accent_style())),
        ]);

        let rows: Vec<Row> = data
            .prospects
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let ovr_pot = if r.scouted {
                    format!("{}/{}", r.overall, r.potential)
                } else {
                    "???".to_string()
                };
                let style = if i == cursor {
                    theme.highlight()
                } else {
                    theme.text()
                };
                Row::new(vec![
                    Cell::from(format!("{:>2}", r.rank)),
                    Cell::from(r.name.clone()),
                    Cell::from(r.position.clone()),
                    Cell::from(r.age.to_string()),
                    Cell::from(ovr_pot),
                    Cell::from(r.projected_pick.clone()),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Min(18),
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Length(9),
                Constraint::Length(8),
            ],
        )
        .header(header)
        .column_spacing(1)
        .block(theme.block(t(lang, T::DraftBoard)));
        f.render_widget(table, area);
    });
}

fn draw_order(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let data = cache.data.as_ref().cloned().unwrap_or_default();
        let cursor = cache.order_cursor.min(data.order.len().saturating_sub(1));
        let title = data
            .user_next_pick
            .map(|p| format!(" {} - {} #{} ", t(tui.lang, T::DraftOrder), tui.user_abbrev, p))
            .unwrap_or_else(|| format!(" {} ", t(tui.lang, T::DraftOrder)));

        let header = Row::new(vec![
            Cell::from(Span::styled("PICK", theme.accent_style())),
            Cell::from(Span::styled("TEAM", theme.accent_style())),
            Cell::from(Span::styled("FRANCHISE", theme.accent_style())),
            Cell::from(Span::styled("OWNER", theme.accent_style())),
        ]);

        let rows: Vec<Row> = data
            .order
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let is_user = r.team_id == tui.user_team;
                let style = if i == cursor || is_user {
                    theme.highlight()
                } else {
                    theme.text()
                };
                Row::new(vec![
                    Cell::from(format!("{:>2}", r.pick)),
                    Cell::from(r.abbrev.clone()),
                    Cell::from(r.full_name.clone()),
                    Cell::from(if is_user { "*" } else { "" }),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Length(8),
                Constraint::Min(18),
                Constraint::Length(8),
            ],
        )
        .header(header)
        .column_spacing(1)
        .block(theme.block(&title));
        f.render_widget(table, area);
    });
}

fn draw_action_bar(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp, tab: SubTab) {
    let hints: &[(&str, &str)] = match tab {
        SubTab::Board => &[
            ("Tab", t(tui.lang, T::CommonTabs)),
            ("Up/Dn", t(tui.lang, T::CommonNavigate)),
            ("s", t(tui.lang, T::DraftScout)),
            ("Enter", t(tui.lang, T::CommonPick)),
            ("A", t(tui.lang, T::DraftAutoPick)),
            ("Esc", t(tui.lang, T::CommonBack)),
        ],
        SubTab::Order => &[
            ("Tab", t(tui.lang, T::CommonTabs)),
            ("Up/Dn", t(tui.lang, T::CommonNavigate)),
            ("A", t(tui.lang, T::DraftAutoPick)),
            ("Esc", t(tui.lang, T::CommonBack)),
        ],
    };
    let bar = match tui.last_msg.as_deref() {
        Some(s) => ActionBar::new(hints).with_status(s),
        None => ActionBar::new(hints),
    };
    bar.render(f, area, theme);
}

fn draw_modal(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang) {
    let _ = t(lang, T::ModalDraftPickTitle);
    CACHE.with(|c| {
        let cache = c.borrow();
        match &cache.modal {
            Modal::None => {}
            Modal::Pick { confirm, .. } | Modal::Sim { confirm } => {
                confirm.render(f, area, theme);
            }
        }
    });
}

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    let modal_action = CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        match &mut cache.modal {
            Modal::None => ModalAction::None,
            Modal::Pick {
                confirm,
                target_name,
            } => match confirm.handle_key(key) {
                WidgetEvent::Submitted => ModalAction::PickSubmit {
                    target_name: target_name.clone(),
                },
                WidgetEvent::Cancelled => ModalAction::Close,
                _ => ModalAction::Pending,
            },
            Modal::Sim { confirm } => match confirm.handle_key(key) {
                WidgetEvent::Submitted => ModalAction::SimSubmit,
                WidgetEvent::Cancelled => ModalAction::Close,
                _ => ModalAction::Pending,
            },
        }
    });

    match modal_action {
        ModalAction::None => {}
        ModalAction::Pending => return Ok(true),
        ModalAction::Close => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            return Ok(true);
        }
        ModalAction::PickSubmit { target_name } => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let res = with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Draft(DraftArgs {
                        action: DraftAction::Pick {
                            player: target_name.clone(),
                        },
                    }),
                )
            });
            after_mutation(
                app,
                tui,
                res,
                MutationKind::Draft,
                &format!("drafted {}", target_name),
            );
            return Ok(true);
        }
        ModalAction::SimSubmit => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let res = with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Draft(DraftArgs {
                        action: DraftAction::Sim(JsonFlag { json: false }),
                    }),
                )
            });
            after_mutation(
                app,
                tui,
                res,
                MutationKind::Draft,
                "draft auto-sim complete",
            );
            return Ok(true);
        }
    }

    match key.code {
        KeyCode::Tab => {
            toggle_tab();
            Ok(true)
        }
        KeyCode::Char('1') => {
            CACHE.with(|c| c.borrow_mut().tab = SubTab::Board);
            Ok(true)
        }
        KeyCode::Char('2') => {
            CACHE.with(|c| c.borrow_mut().tab = SubTab::Order);
            Ok(true)
        }
        KeyCode::Up => {
            move_cursor(-1);
            Ok(true)
        }
        KeyCode::Down => {
            move_cursor(1);
            Ok(true)
        }
        KeyCode::PageUp => {
            move_cursor(-10);
            Ok(true)
        }
        KeyCode::PageDown => {
            move_cursor(10);
            Ok(true)
        }
        KeyCode::Char('s') => {
            if let Some(name) = current_prospect_name() {
                let res = with_silenced_io(|| {
                    crate::commands::dispatch(
                        app,
                        Command::Scout {
                            player: name.clone(),
                        },
                    )
                });
                after_mutation(
                    app,
                    tui,
                    res,
                    MutationKind::Scout,
                    &format!("scouted {}", name),
                );
                return Ok(true);
            }
            tui.last_msg = Some("no prospect selected".to_string());
            Ok(true)
        }
        KeyCode::Enter => {
            if !draft_active(tui.season_state.phase) {
                tui.last_msg = Some("Draft not active. Sim to end of season.".to_string());
                return Ok(true);
            }
            if let Some(name) = current_prospect_name() {
                CACHE.with(|c| {
                    c.borrow_mut().modal = Modal::Pick {
                        confirm: Confirm::new(format!("Draft {} to {}?", name, tui.user_abbrev)),
                        target_name: name,
                    };
                });
                return Ok(true);
            }
            tui.last_msg = Some("no prospect selected".to_string());
            Ok(true)
        }
        KeyCode::Char('A') | KeyCode::Char('a') => {
            if !draft_active(tui.season_state.phase) {
                tui.last_msg = Some("Draft not active. Sim to end of season.".to_string());
                return Ok(true);
            }
            CACHE.with(|c| {
                c.borrow_mut().modal = Modal::Sim {
                    confirm: Confirm::new("Auto-pick the rest of the draft?"),
                };
            });
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn ensure_cache(app: &mut AppState, tui: &TuiApp) -> Result<()> {
    let key = CacheKey {
        season: tui.season,
        day: tui.season_state.day,
        phase: tui.season_state.phase,
    };
    let fresh = CACHE.with(|c| {
        let c = c.borrow();
        c.cached_for.as_ref() == Some(&key) && c.data.is_some()
    });
    if fresh {
        return Ok(());
    }

    let data = load_draft_data(app, tui)?;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.cached_for = Some(key);
        c.board_cursor = c.board_cursor.min(data.prospects.len().saturating_sub(1));
        c.order_cursor = c.order_cursor.min(data.order.len().saturating_sub(1));
        c.data = Some(data);
    });
    Ok(())
}

fn load_draft_data(app: &mut AppState, tui: &TuiApp) -> Result<DraftData> {
    let prospects_visible = app.store()?.list_prospects_visible()?;
    let prospect_total = prospects_visible.len();
    let scouted_count = prospects_visible
        .iter()
        .filter(|(_, scouted)| *scouted)
        .count();
    let prospects = prospects_visible
        .into_iter()
        .take(60)
        .enumerate()
        .map(|(i, (p, scouted))| ProspectRow {
            rank: i + 1,
            name: p.name,
            position: p.primary_position.to_string(),
            age: p.age,
            scouted,
            overall: p.overall,
            potential: p.potential,
            projected_pick: format!("#{}", i + 1),
        })
        .collect::<Vec<_>>();

    let order_ids = local_draft_order(app)?;
    let teams = app.store()?.list_teams()?;
    let names: HashMap<TeamId, (String, String)> = teams
        .into_iter()
        .map(|t| (t.id, (t.abbrev.clone(), t.full_name())))
        .collect();
    let order = order_ids
        .into_iter()
        .enumerate()
        .map(|(i, team_id)| {
            let (abbrev, full_name) = names
                .get(&team_id)
                .cloned()
                .unwrap_or_else(|| (format!("T{}", team_id.0), "Unknown Team".to_string()));
            OrderRow {
                pick: i + 1,
                team_id,
                abbrev,
                full_name,
            }
        })
        .collect::<Vec<_>>();
    let user_next_pick = order
        .iter()
        .find(|r| r.team_id == tui.user_team)
        .map(|r| r.pick);

    Ok(DraftData {
        prospects,
        order,
        scouted_count,
        prospect_total,
        user_next_pick,
    })
}

fn local_draft_order(app: &mut AppState) -> Result<Vec<TeamId>> {
    let state = app
        .store()?
        .load_season_state()?
        .ok_or_else(|| anyhow!("no season state in save"))?;
    let teams = app.store()?.list_teams()?;

    let mut standings = Standings::new(&teams);
    let mut games = app.store()?.read_games(state.season)?;
    if games.iter().all(|g| g.is_playoffs) || games.is_empty() {
        if state.season.0 > 1 {
            games = app.store()?.read_games(SeasonId(state.season.0 - 1))?;
        }
    }
    for g in &games {
        if !g.is_playoffs {
            standings.record_game_result(g);
        }
    }

    let mut by_record: Vec<(TeamId, u16, i32, u8)> = teams
        .iter()
        .map(|t| {
            let r = standings.records.get(&t.id);
            (
                t.id,
                r.map(|r| r.wins).unwrap_or(0),
                r.map(|r| r.point_diff).unwrap_or(0),
                t.id.0,
            )
        })
        .collect();
    by_record.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });

    let lottery_count = by_record.len().min(14);
    let post_lottery: Vec<TeamId> = by_record
        .iter()
        .skip(lottery_count)
        .map(|(id, _, _, _)| *id)
        .collect();

    const ODDS_BPS: [u32; 14] = [
        1400, 1400, 1400, 1250, 1050, 900, 750, 600, 450, 300, 200, 150, 100, 50,
    ];
    let mut pool: Vec<(TeamId, u32)> = by_record
        .iter()
        .take(lottery_count)
        .enumerate()
        .map(|(i, (id, _, _, _))| (*id, ODDS_BPS.get(i).copied().unwrap_or(0)))
        .collect();

    let mut rng =
        ChaCha8Rng::seed_from_u64((state.season.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut lottery_order: Vec<TeamId> = Vec::with_capacity(lottery_count);
    for _ in 0..lottery_count.min(4) {
        let total: u32 = pool.iter().map(|(_, w)| *w).sum();
        if total == 0 {
            break;
        }
        let mut roll = rng.gen_range(0..total);
        let mut picked = 0usize;
        for (i, (_, w)) in pool.iter().enumerate() {
            if roll < *w {
                picked = i;
                break;
            }
            roll -= *w;
        }
        lottery_order.push(pool[picked].0);
        pool.remove(picked);
    }

    let drawn: HashSet<TeamId> = lottery_order.iter().copied().collect();
    for (id, _, _, _) in by_record.iter().take(lottery_count) {
        if !drawn.contains(id) {
            lottery_order.push(*id);
        }
    }

    let mut final_order = lottery_order;
    final_order.extend(post_lottery);
    Ok(final_order)
}

fn toggle_tab() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.tab = match c.tab {
            SubTab::Board => SubTab::Order,
            SubTab::Order => SubTab::Board,
        };
    });
}

fn move_cursor(delta: isize) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let len = match c.tab {
            SubTab::Board => c.data.as_ref().map(|d| d.prospects.len()).unwrap_or(0),
            SubTab::Order => c.data.as_ref().map(|d| d.order.len()).unwrap_or(0),
        };
        if len == 0 {
            return;
        }
        let cursor = match c.tab {
            SubTab::Board => &mut c.board_cursor,
            SubTab::Order => &mut c.order_cursor,
        };
        let next = (*cursor as isize + delta).clamp(0, len.saturating_sub(1) as isize);
        *cursor = next as usize;
    });
}

fn current_prospect_name() -> Option<String> {
    CACHE.with(|c| {
        let c = c.borrow();
        if c.tab != SubTab::Board {
            return None;
        }
        c.data
            .as_ref()
            .and_then(|d| d.prospects.get(c.board_cursor))
            .map(|r| r.name.clone())
    })
}

fn draft_active(phase: SeasonPhase) -> bool {
    matches!(phase, SeasonPhase::OffSeason | SeasonPhase::Playoffs)
}

fn after_mutation(
    app: &mut AppState,
    tui: &mut TuiApp,
    res: Result<()>,
    kind: MutationKind,
    success_msg: &str,
) {
    match res {
        Ok(()) => {
            invalidate();
            crate::tui::screens::home::invalidate();
            if matches!(kind, MutationKind::Draft) {
                crate::tui::screens::roster::invalidate();
                tui.invalidate_caches();
                if let Err(e) = tui.refresh_save_ctx(app) {
                    tui.last_msg = Some(format!("{}; refresh failed: {}", success_msg, e));
                    return;
                }
            }
            tui.last_msg = Some(success_msg.to_string());
        }
        Err(e) => {
            tui.last_msg = Some(format!("error: {}", e));
        }
    }
}

fn modal_rect(area: Rect) -> Rect {
    let w = 58.min(area.width.saturating_sub(4));
    let h = 7.min(area.height.saturating_sub(4));
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

enum ModalAction {
    None,
    Pending,
    Close,
    PickSubmit { target_name: String },
    SimSubmit,
}

#[derive(Copy, Clone)]
enum MutationKind {
    Scout,
    Draft,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba3k_core::SeasonPhase;

    #[test]
    fn draft_actions_are_gated_to_playoffs_and_offseason() {
        assert!(draft_active(SeasonPhase::Playoffs));
        assert!(draft_active(SeasonPhase::OffSeason));

        assert!(!draft_active(SeasonPhase::PreSeason));
        assert!(!draft_active(SeasonPhase::Regular));
        assert!(!draft_active(SeasonPhase::TradeDeadlinePassed));
        assert!(!draft_active(SeasonPhase::Draft));
        assert!(!draft_active(SeasonPhase::FreeAgency));
    }
}
