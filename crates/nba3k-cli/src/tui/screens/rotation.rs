//! Rotation screen (M21 Level A — starters only).
//!
//! Single-screen lineup picker: 5 positional slots (PG/SG/SF/PF/C), one player
//! each. Bench order, minutes split, and closing lineup stay auto-built —
//! Level A only lets the GM lock who starts. Empty slots fall through to the
//! sim's auto rotation (`Starters::is_complete` gates the override).
//!
//! All writes go through the Wave-0 `Store` API (`upsert_starter` /
//! `clear_starter` / `clear_all_starters`) — no `Command` enum variant.
//! Each mutation invalidates this screen's cache + the home screen's cache so
//! a roster-affecting change elsewhere stays in sync.
//!
//! Key bindings:
//!   ↑ / ↓        — move slot cursor (wraps)
//!   Enter        — open player picker for slot
//!   c            — clear current slot
//!   C            — clear all slots (confirm)
//!   Esc          — back to menu (shell handles)

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::state::AppState;
use crate::tui::widgets::{ActionBar, Confirm, FormWidget, Picker, Theme, WidgetEvent};
use crate::tui::TuiApp;
use nba3k_core::{t, Lang, Player, PlayerId, Position, Starters, T};

// ---------------------------------------------------------------------------
// Cache + modal types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct PlayerOption {
    id: PlayerId,
    name: String,
    primary: Position,
    overall: u8,
    on_roster: bool,
}

#[derive(Default)]
struct RotationCache {
    /// Current persisted starters for the user team.
    starters: Option<Starters>,
    /// Active roster keyed by id (used to render names + OVR + on-roster flag).
    roster_index: Option<HashMap<PlayerId, PlayerOption>>,
    /// Eligible options per slot, OVR-desc sorted. Computed once per cache fill.
    eligible: Option<HashMap<Position, Vec<PlayerOption>>>,
    /// Cursor in the 5-slot list (0..=4).
    slot_cursor: usize,
    /// Currently open modal.
    modal: Modal,
}

#[derive(Default)]
enum Modal {
    #[default]
    None,
    /// Player picker for one slot.
    Pick {
        slot: Position,
        picker: Picker<PlayerOption>,
    },
    /// "Clear all" confirmation.
    ClearAll {
        confirm: Confirm,
    },
}

thread_local! {
    static CACHE: RefCell<RotationCache> = RefCell::new(RotationCache::default());
}

/// Drop the cached starters/eligibility/index. Called after mutations from
/// this screen, and exposed for cross-screen invalidation (post-trade roster
/// change should bust this so a no-longer-on-roster starter renders dimmed).
pub fn invalidate() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.starters = None;
        c.roster_index = None;
        c.eligible = None;
        // Preserve cursor + modal (the modal owns target state).
    });
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, tui: &TuiApp) {
    if !tui.has_save() {
        let p = Paragraph::new(t(tui.lang, T::CommonNoSaveLoaded))
            .block(theme.block(t(tui.lang, T::RotationTitle)));
        f.render_widget(p, area);
        return;
    }
    if let Err(e) = ensure_cache(app, tui) {
        let p = Paragraph::new(format!("Rotation unavailable: {}", e))
            .block(theme.block(t(tui.lang, T::RotationTitle)));
        f.render_widget(p, area);
        return;
    }

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    draw_slot_list(f, parts[0], theme, tui);
    let hints = [
        ("↑↓", t(tui.lang, T::CommonNavigate)),
        ("Enter", t(tui.lang, T::CommonPick)),
        ("c", t(tui.lang, T::RotationClearSlot)),
        ("C", t(tui.lang, T::RotationClearAll)),
        ("Esc", t(tui.lang, T::CommonBack)),
    ];
    let bar = ActionBar::new(&hints);
    bar.render(f, parts[1], theme);

    let need_modal = CACHE.with(|c| !matches!(c.borrow().modal, Modal::None));
    if need_modal {
        let rect = modal_rect(area);
        f.render_widget(Clear, rect);
        draw_modal(f, rect, theme, tui.lang);
    }
}

fn draw_slot_list(f: &mut Frame, area: Rect, theme: &Theme, tui: &TuiApp) {
    CACHE.with(|c| {
        let cache = c.borrow();
        let starters = cache.starters.clone().unwrap_or_default();
        let index = cache.roster_index.as_ref();
        let cursor = cache.slot_cursor.min(4);

        let title = format!(" {} - {} ", t(tui.lang, T::RotationStarters), tui.user_abbrev);
        let mut lines: Vec<Line> = Vec::with_capacity(8);
        lines.push(Line::from(""));

        for (i, pos) in Position::all().iter().enumerate() {
            let is_cursor = i == cursor;
            let prefix = if is_cursor { "> " } else { "  " };
            let row_style = if is_cursor {
                theme.highlight()
            } else {
                theme.text()
            };

            let pos_label = format!("{:<2}", pos);
            let assigned = starters.slot(*pos);
            let body = match assigned {
                None => Span::styled(format!("{:<28}", "[empty — auto-pick]"), theme.muted_style()),
                Some(pid) => match index.and_then(|m| m.get(&pid)) {
                    Some(opt) => {
                        let mut text = format!("{} ({} OVR)", opt.name, opt.overall);
                        if !opt.on_roster {
                            text.push_str("  (off roster)");
                        }
                        let style = if !opt.on_roster {
                            theme.muted_style()
                        } else {
                            row_style
                        };
                        Span::styled(format!("{:<28}", text), style)
                    }
                    None => Span::styled(
                        format!("{:<28}", format!("#{} (off roster)", pid.0)),
                        theme.muted_style(),
                    ),
                },
            };

            let hint = match assigned {
                None => Span::styled(
                    "press Enter to choose".to_string(),
                    theme.muted_style(),
                ),
                Some(_) => Span::styled(
                    "press Enter to change, c to clear".to_string(),
                    theme.muted_style(),
                ),
            };

            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), row_style),
                Span::styled(pos_label, theme.accent_style()),
                Span::styled("  ".to_string(), row_style),
                body,
                Span::styled("  ".to_string(), row_style),
                hint,
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Tip: Bench + minutes remain auto. Rotation applies on next sim.",
            theme.muted_style(),
        )));

        let p = Paragraph::new(lines).block(theme.block(&title));
        f.render_widget(p, area);
    });
}

fn modal_rect(area: Rect) -> Rect {
    let w = area.width.saturating_sub(8).min(72).max(40);
    let h = area.height.saturating_sub(4).min(20).max(8);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}

fn draw_modal(f: &mut Frame, rect: Rect, theme: &Theme, lang: Lang) {
    enum DrawSpec {
        None,
        Pick { picker: Picker<PlayerOption> },
        ClearAll { confirm: Confirm },
    }

    let spec = CACHE.with(|c| {
        let cache = c.borrow();
        match &cache.modal {
            Modal::None => DrawSpec::None,
            Modal::Pick { picker, .. } => DrawSpec::Pick { picker: picker.clone() },
            Modal::ClearAll { confirm } => DrawSpec::ClearAll { confirm: confirm.clone() },
        }
    });

    match spec {
        DrawSpec::None => {}
        DrawSpec::Pick { picker } => {
            let _ = t(lang, T::CommonPick);
            picker.render(f, rect, theme);
        }
        DrawSpec::ClearAll { confirm } => {
            confirm.render(f, rect, theme);
        }
    }
}

// ---------------------------------------------------------------------------
// Cache population
// ---------------------------------------------------------------------------

fn ensure_cache(app: &mut AppState, tui: &TuiApp) -> Result<()> {
    let need_starters = CACHE.with(|c| c.borrow().starters.is_none());
    let need_roster = CACHE.with(|c| c.borrow().roster_index.is_none());
    let need_elig = CACHE.with(|c| c.borrow().eligible.is_none());

    if need_starters {
        let starters = app.store()?.read_starters(tui.user_team)?;
        CACHE.with(|c| c.borrow_mut().starters = Some(starters));
    }
    if need_roster || need_elig {
        let store = app.store()?;
        let roster: Vec<Player> = store.roster_for_team(tui.user_team)?;

        let on_roster_options: Vec<PlayerOption> = roster
            .iter()
            .map(|p| PlayerOption {
                id: p.id,
                name: clean_name(&p.name),
                primary: p.primary_position,
                overall: p.overall,
                on_roster: true,
            })
            .collect();

        // Index includes the on-roster set first, then any starter ids that
        // aren't on the roster anymore (cut/traded/retired) so render can show
        // a "(off roster)" stub instead of a bare numeric id.
        let mut index: HashMap<PlayerId, PlayerOption> = on_roster_options
            .iter()
            .map(|o| (o.id, o.clone()))
            .collect();

        let starters_now = CACHE.with(|c| c.borrow().starters.clone()).unwrap_or_default();
        for (_, pid) in starters_now.iter_assigned() {
            if !index.contains_key(&pid) {
                let name = store
                    .player_name(pid)?
                    .map(|n| clean_name(&n))
                    .unwrap_or_else(|| format!("#{}", pid.0));
                index.insert(
                    pid,
                    PlayerOption {
                        id: pid,
                        name,
                        primary: Position::SF,
                        overall: 0,
                        on_roster: false,
                    },
                );
            }
        }

        // Per-slot eligibility: primary or secondary at slot, or adjacent.
        // Using primary only here since `roster_for_team` gives back full
        // `Player`; secondary is consulted alongside primary.
        let mut eligible: HashMap<Position, Vec<PlayerOption>> = HashMap::new();
        for slot in Position::all() {
            let mut bucket: Vec<PlayerOption> = roster
                .iter()
                .filter(|p| eligible_for_slot(p, slot))
                .map(|p| PlayerOption {
                    id: p.id,
                    name: clean_name(&p.name),
                    primary: p.primary_position,
                    overall: p.overall,
                    on_roster: true,
                })
                .collect();
            bucket.sort_by(|a, b| {
                b.overall
                    .cmp(&a.overall)
                    .then_with(|| a.name.cmp(&b.name))
            });
            eligible.insert(slot, bucket);
        }

        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            c.roster_index = Some(index);
            c.eligible = Some(eligible);
        });
    }
    Ok(())
}

/// Adjacency: PG↔SG, SG↔SF, SF↔PF, PF↔C — same rule as
/// `commands::build_position_aware_rotation` (`|idx(a) - idx(b)| <= 1`).
fn eligible_for_slot(p: &Player, slot: Position) -> bool {
    if p.primary_position == slot {
        return true;
    }
    if p.secondary_position == Some(slot) {
        return true;
    }
    pos_distance(p.primary_position, slot) <= 1
}

fn pos_distance(a: Position, b: Position) -> i32 {
    let idx = |p: Position| -> i32 {
        match p {
            Position::PG => 0,
            Position::SG => 1,
            Position::SF => 2,
            Position::PF => 3,
            Position::C => 4,
        }
    };
    (idx(a) - idx(b)).abs()
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    // Modal first.
    let action = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        match &mut c.modal {
            Modal::None => ModalAction::None,
            Modal::Pick { slot, picker } => match picker.handle_key(key) {
                WidgetEvent::Submitted => match picker.selected().cloned() {
                    Some(opt) => ModalAction::PickSubmit {
                        slot: *slot,
                        player_id: opt.id,
                        player_name: opt.name.clone(),
                    },
                    None => ModalAction::CloseModal,
                },
                WidgetEvent::Cancelled => ModalAction::CloseModal,
                _ => ModalAction::Pending,
            },
            Modal::ClearAll { confirm } => match confirm.handle_key(key) {
                WidgetEvent::Submitted => ModalAction::ClearAllSubmit,
                WidgetEvent::Cancelled => ModalAction::CloseModal,
                _ => ModalAction::Pending,
            },
        }
    });

    match action {
        ModalAction::None => {}
        ModalAction::Pending => return Ok(true),
        ModalAction::CloseModal => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            return Ok(true);
        }
        ModalAction::PickSubmit { slot, player_id, player_name } => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let pos_str = pos_to_str(slot);
            let res = upsert_starter(app, tui, pos_str, player_id);
            after_mutation(tui, res, &format!("{} → {}", pos_str, player_name));
            return Ok(true);
        }
        ModalAction::ClearAllSubmit => {
            CACHE.with(|c| c.borrow_mut().modal = Modal::None);
            let res = clear_all_starters(app, tui);
            after_mutation(tui, res, "cleared all starters");
            return Ok(true);
        }
    }

    // No modal — slot list nav.
    match key.code {
        KeyCode::Up => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.slot_cursor = if c.slot_cursor == 0 { 4 } else { c.slot_cursor - 1 };
            });
            Ok(true)
        }
        KeyCode::Down => {
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.slot_cursor = (c.slot_cursor + 1) % 5;
            });
            Ok(true)
        }
        KeyCode::Enter => {
            let slot = current_slot();
            open_picker(slot);
            Ok(true)
        }
        KeyCode::Char('c') => {
            let slot = current_slot();
            let pos_str = pos_to_str(slot);
            let res = clear_starter(app, tui, pos_str);
            after_mutation(tui, res, &format!("cleared {}", pos_str));
            Ok(true)
        }
        KeyCode::Char('C') => {
            CACHE.with(|c| {
                c.borrow_mut().modal = Modal::ClearAll {
                    confirm: Confirm::new("Clear all starters? Auto-builder will resume."),
                };
            });
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn current_slot() -> Position {
    let cursor = CACHE.with(|c| c.borrow().slot_cursor.min(4));
    Position::all()[cursor]
}

fn open_picker(slot: Position) {
    let bucket = CACHE.with(|c| {
        c.borrow()
            .eligible
            .as_ref()
            .and_then(|m| m.get(&slot).cloned())
            .unwrap_or_default()
    });
    let title = format!("Pick {}", pos_to_str(slot));
    let picker: Picker<PlayerOption> = Picker::new(title, bucket, |o| {
        format!("{:<24}  {}  {} OVR", o.name, o.primary, o.overall)
    });
    CACHE.with(|c| {
        c.borrow_mut().modal = Modal::Pick { slot, picker };
    });
}

fn after_mutation(tui: &mut TuiApp, res: Result<()>, success_msg: &str) {
    match res {
        Ok(()) => tui.last_msg = Some(success_msg.into()),
        Err(e) => tui.last_msg = Some(format!("error: {}", e)),
    }
    invalidate();
    crate::tui::screens::home::invalidate();
}

fn upsert_starter(
    app: &mut AppState,
    tui: &TuiApp,
    pos_str: &str,
    player_id: PlayerId,
) -> Result<()> {
    let store = app.store()?;
    store.upsert_starter(tui.user_team, pos_str, player_id)?;
    Ok(())
}

fn clear_starter(app: &mut AppState, tui: &TuiApp, pos_str: &str) -> Result<()> {
    let store = app.store()?;
    store.clear_starter(tui.user_team, pos_str)?;
    Ok(())
}

fn clear_all_starters(app: &mut AppState, tui: &TuiApp) -> Result<()> {
    let store = app.store()?;
    store.clear_all_starters(tui.user_team)?;
    Ok(())
}

fn pos_to_str(p: Position) -> &'static str {
    match p {
        Position::PG => "PG",
        Position::SG => "SG",
        Position::SF => "SF",
        Position::PF => "PF",
        Position::C => "C",
    }
}

fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Modal action enum (drop the borrow before touching `app` / `tui`).
// ---------------------------------------------------------------------------

enum ModalAction {
    None,
    Pending,
    CloseModal,
    PickSubmit {
        slot: Position,
        player_id: PlayerId,
        player_name: String,
    },
    ClearAllSubmit,
}
