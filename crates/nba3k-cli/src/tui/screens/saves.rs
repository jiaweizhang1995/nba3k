//! Saves overlay (Ctrl+S). Shows a scrollable list of save files in the
//! current dir + `/tmp` (matches `cmd_saves_list` scan logic). User keys: `n`
//! new (push wizard), `l` load, `d` delete (with confirm), `e` export to JSON
//! (prompt for destination), Esc back.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Cell, Clear, Paragraph, Row, Table},
    Frame,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::cli::{Command, SavesAction, SavesArgs};
use crate::state::AppState;
use crate::tui::widgets::{Confirm, FormWidget, TextInput, Theme, WidgetEvent};
use crate::tui::{Screen, TuiApp};

// ---------------------------------------------------------------------------
// Per-screen cache + modal state
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SavesState {
    rows: Option<Vec<SaveRow>>,
    cursor: usize,
    /// Active modal, if any (delete-confirm or export-path prompt).
    modal: Modal,
}

#[derive(Default)]
enum Modal {
    #[default]
    None,
    Delete(Confirm, PathBuf),
    Export(TextInput, PathBuf),
}

struct SaveRow {
    path: PathBuf,
    season: Option<u16>,
    day: Option<u32>,
    team: Option<String>,
    size_kb: u64,
    mtime: Option<SystemTime>,
    mtime_label: String,
    /// Set when `Store::open` failed — we still show the row but with the error.
    error: Option<String>,
}

thread_local! {
    static STATE: RefCell<SavesState> = RefCell::new(SavesState::default());
}

/// Drop the cached scan so the next render re-walks the filesystem. Called
/// from screens that mutate the save set (NewGame on success).
pub fn invalidate() {
    STATE.with(|s| *s.borrow_mut() = SavesState::default());
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState, _tui: &TuiApp) {
    if let Err(e) = ensure_scan() {
        let p = Paragraph::new(format!("Saves overlay error: {}", e))
            .block(theme.block(" Saves "));
        f.render_widget(p, area);
        return;
    }

    // Header (1 line) | Body table (rest) | Hint footer (2 lines).
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    draw_header(f, parts[0], theme, app);
    let modal_rect = STATE.with(|s| {
        let st = s.borrow();
        draw_table(f, parts[1], theme, st.rows.as_deref().unwrap_or(&[]), st.cursor);
        draw_hints(f, parts[2], theme);
        if matches!(st.modal, Modal::None) {
            None
        } else {
            Some(modal_rect(area))
        }
    });
    if let Some(rect) = modal_rect {
        // Wipe background under the modal so the table doesn't bleed through.
        f.render_widget(Clear, rect);
        STATE.with(|s| {
            let st = s.borrow();
            match &st.modal {
                Modal::None => {}
                Modal::Delete(c, _) => c.render(f, rect, theme),
                Modal::Export(t, _) => t.render(f, rect, theme),
            }
        });
    }
}

fn draw_header(f: &mut Frame, area: Rect, theme: &Theme, app: &mut AppState) {
    let current = app
        .save_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(no save loaded)".into());
    let lines = vec![Line::from(vec![
        Span::styled("Current: ", theme.muted_style()),
        Span::styled(current, theme.text()),
    ])];
    let p = Paragraph::new(lines).block(theme.block(" Save Files "));
    f.render_widget(p, area);
}

fn draw_table(f: &mut Frame, area: Rect, theme: &Theme, rows: &[SaveRow], cursor: usize) {
    let header = Row::new(vec![
        Cell::from(Span::styled("PATH", theme.accent_style())),
        Cell::from(Span::styled("TEAM", theme.accent_style())),
        Cell::from(Span::styled("SEASON", theme.accent_style())),
        Cell::from(Span::styled("DAY", theme.accent_style())),
        Cell::from(Span::styled("SIZE", theme.accent_style())),
        Cell::from(Span::styled("MODIFIED", theme.accent_style())),
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
            let path = r
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string();
            let team = r.team.clone().unwrap_or_else(|| "—".into());
            let season = r
                .season
                .map(|s| s.to_string())
                .unwrap_or_else(|| "—".into());
            let day = r.day.map(|d| d.to_string()).unwrap_or_else(|| "—".into());
            let size = format!("{} KB", r.size_kb);
            let mtime = r.mtime_label.clone();
            if let Some(err) = &r.error {
                Row::new(vec![
                    Cell::from(Span::styled(path, style)),
                    Cell::from(Span::styled(format!("[{}]", err), style)),
                    Cell::from(Span::styled("", style)),
                    Cell::from(Span::styled("", style)),
                    Cell::from(Span::styled(size, style)),
                    Cell::from(Span::styled(mtime, style)),
                ])
            } else {
                Row::new(vec![
                    Cell::from(Span::styled(path, style)),
                    Cell::from(Span::styled(team, style)),
                    Cell::from(Span::styled(season, style)),
                    Cell::from(Span::styled(day, style)),
                    Cell::from(Span::styled(size, style)),
                    Cell::from(Span::styled(mtime, style)),
                ])
            }
        })
        .collect();

    let title = format!(" Saves ({}) ", rows.len());
    let table = Table::new(
        body,
        [
            Constraint::Min(20),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(8),
            Constraint::Length(20),
        ],
    )
    .header(header)
    .block(theme.block(&title));
    f.render_widget(table, area);
}

fn draw_hints(f: &mut Frame, area: Rect, theme: &Theme) {
    let hint = Line::from(vec![
        Span::styled(" ↑↓ ", theme.accent_style()),
        Span::styled(" Move   ", theme.text()),
        Span::styled(" n ", theme.accent_style()),
        Span::styled(" New   ", theme.text()),
        Span::styled(" l ", theme.accent_style()),
        Span::styled(" Load   ", theme.text()),
        Span::styled(" d ", theme.accent_style()),
        Span::styled(" Delete   ", theme.text()),
        Span::styled(" e ", theme.accent_style()),
        Span::styled(" Export   ", theme.text()),
        Span::styled(" Esc ", theme.accent_style()),
        Span::styled(" Back", theme.text()),
    ]);
    let p = Paragraph::new(hint).block(theme.block(""));
    f.render_widget(p, area);
}

fn modal_rect(area: Rect) -> Rect {
    let w = 60.min(area.width.saturating_sub(4));
    let h = 7.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}

// ---------------------------------------------------------------------------
// Scan logic — mirrors `cmd_saves_list` (commands.rs L5783+) but returns
// structured rows for the table render.
// ---------------------------------------------------------------------------

fn ensure_scan() -> Result<()> {
    let need = STATE.with(|s| s.borrow().rows.is_none());
    if !need {
        return Ok(());
    }
    let rows = scan_saves();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.cursor = s.cursor.min(rows.len().saturating_sub(1));
        s.rows = Some(rows);
    });
    Ok(())
}

fn scan_saves() -> Vec<SaveRow> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        scan_db_files(&cwd, &mut paths);
    }
    let tmp = PathBuf::from("/tmp");
    if cfg!(unix) && tmp.is_dir() {
        scan_db_files(&tmp, &mut paths);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        // Default save dir from the new-game wizard
        let desktop_saves = home.join("Desktop").join("nba3k_save");
        if desktop_saves.is_dir() {
            scan_db_files(&desktop_saves, &mut paths);
        }
        // Old default location (single file at HOME root)
        scan_db_files(&home, &mut paths);
    }
    paths.sort();
    paths.dedup();

    let mut rows: Vec<SaveRow> = paths.into_iter().map(read_save_row).collect();
    // Newest first by mtime; rows without mtime sink to the bottom.
    rows.sort_by(|a, b| match (a.mtime, b.mtime) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.path.cmp(&b.path),
    });
    rows
}

fn scan_db_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("db") {
            out.push(p);
        }
    }
}

fn read_save_row(path: PathBuf) -> SaveRow {
    let meta = std::fs::metadata(&path).ok();
    let size_kb = meta.as_ref().map(|m| m.len() / 1024).unwrap_or(0);
    let mtime = meta.as_ref().and_then(|m| m.modified().ok());
    let mtime_label = mtime.map(format_mtime).unwrap_or_else(|| "—".into());

    match nba3k_store::Store::open(&path) {
        Ok(store) => {
            let state = store.load_season_state().ok().flatten();
            let team = match &state {
                Some(s) => store.team_abbrev(s.user_team).ok().flatten(),
                None => store.get_meta("user_team").ok().flatten(),
            };
            let season = state.as_ref().map(|s| s.season.0).or_else(|| {
                store
                    .get_meta("season")
                    .ok()
                    .flatten()
                    .and_then(|s| s.parse::<u16>().ok())
            });
            let day = state.as_ref().map(|s| s.day);
            SaveRow {
                path,
                season,
                day,
                team,
                size_kb,
                mtime,
                mtime_label,
                error: None,
            }
        }
        Err(e) => SaveRow {
            path,
            season: None,
            day: None,
            team: None,
            size_kb,
            mtime,
            mtime_label,
            error: Some(e.to_string()),
        },
    }
}

fn format_mtime(ts: SystemTime) -> String {
    use chrono::{DateTime, Local};
    let dt: DateTime<Local> = ts.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    // Modal-first: capture keys for the delete-confirm / export-path prompt.
    let modal_action = STATE.with(|s| {
        let mut s = s.borrow_mut();
        match &mut s.modal {
            Modal::None => ModalAction::None,
            Modal::Delete(c, p) => match c.handle_key(key) {
                WidgetEvent::Submitted => ModalAction::ConfirmDelete(p.clone()),
                WidgetEvent::Cancelled => ModalAction::Cancel,
                _ => ModalAction::Pending,
            },
            Modal::Export(t, p) => match t.handle_key(key) {
                WidgetEvent::Submitted => {
                    let dest = t.value().to_string();
                    ModalAction::ConfirmExport(p.clone(), dest)
                }
                WidgetEvent::Cancelled => ModalAction::Cancel,
                _ => ModalAction::Pending,
            },
        }
    });

    match modal_action {
        ModalAction::None => {}
        ModalAction::Pending => return Ok(true),
        ModalAction::Cancel => {
            STATE.with(|s| s.borrow_mut().modal = Modal::None);
            return Ok(true);
        }
        ModalAction::ConfirmDelete(path) => {
            STATE.with(|s| s.borrow_mut().modal = Modal::None);
            let res = crate::tui::with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Saves(SavesArgs {
                        action: SavesAction::Delete { path: path.clone(), yes: true },
                    }),
                )
            });
            match res {
                Ok(()) => {
                    tui.last_msg = Some(format!("deleted {}", path.display()));
                    invalidate();
                }
                Err(e) => {
                    tui.last_msg = Some(format!("delete failed: {}", e));
                }
            }
            return Ok(true);
        }
        ModalAction::ConfirmExport(path, dest_str) => {
            STATE.with(|s| s.borrow_mut().modal = Modal::None);
            if dest_str.is_empty() {
                tui.last_msg = Some("export cancelled (no destination)".into());
                return Ok(true);
            }
            let dest = PathBuf::from(dest_str);
            let res = crate::tui::with_silenced_io(|| {
                crate::commands::dispatch(
                    app,
                    Command::Saves(SavesArgs {
                        action: SavesAction::Export { path: path.clone(), to: dest.clone() },
                    }),
                )
            });
            match res {
                Ok(()) => tui.last_msg = Some(format!("exported {} → {}", path.display(), dest.display())),
                Err(e) => tui.last_msg = Some(format!("export failed: {}", e)),
            }
            return Ok(true);
        }
    }

    // No modal — top-level overlay keys.
    match key.code {
        KeyCode::Up => {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.cursor > 0 {
                    s.cursor -= 1;
                }
            });
            Ok(true)
        }
        KeyCode::Down => {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                let max = s.rows.as_ref().map(|v| v.len()).unwrap_or(0).saturating_sub(1);
                if s.cursor < max {
                    s.cursor += 1;
                }
            });
            Ok(true)
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            tui.current = Screen::NewGame;
            crate::tui::screens::new_game::reset();
            Ok(true)
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            let path = STATE.with(|s| {
                let s = s.borrow();
                s.rows
                    .as_ref()
                    .and_then(|r| r.get(s.cursor))
                    .filter(|r| r.error.is_none())
                    .map(|r| r.path.clone())
            });
            let Some(path) = path else {
                tui.last_msg = Some("nothing to load".into());
                return Ok(true);
            };
            // `switch_save` opens the file and re-reads season_state into
            // `save_ctx` in one shot — no need to dispatch `Command::Load`
            // separately.
            let res = crate::tui::with_silenced_io(|| tui.switch_save(app, path.clone()));
            match res {
                Ok(()) => {
                    tui.last_msg = Some(format!("loaded {}", path.display()));
                    crate::tui::screens::home::invalidate();
                    tui.invalidate_caches();
                    tui.current = Screen::Menu;
                }
                Err(e) => tui.last_msg = Some(format!("load failed: {}", e)),
            }
            Ok(true)
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            let path = STATE.with(|s| {
                let s = s.borrow();
                s.rows
                    .as_ref()
                    .and_then(|r| r.get(s.cursor))
                    .map(|r| r.path.clone())
            });
            let Some(path) = path else { return Ok(true) };
            let prompt = format!(
                "Delete {}? This is irreversible. y/n",
                path.display()
            );
            STATE.with(|s| {
                s.borrow_mut().modal = Modal::Delete(Confirm::new(prompt), path);
            });
            Ok(true)
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            let path = STATE.with(|s| {
                let s = s.borrow();
                s.rows
                    .as_ref()
                    .and_then(|r| r.get(s.cursor))
                    .filter(|r| r.error.is_none())
                    .map(|r| r.path.clone())
            });
            let Some(path) = path else { return Ok(true) };
            let default_dest = path.with_extension("json").display().to_string();
            STATE.with(|s| {
                s.borrow_mut().modal = Modal::Export(
                    TextInput::new("Export to:").with_initial(default_dest),
                    path,
                );
            });
            Ok(true)
        }
        _ => Ok(false),
    }
}

enum ModalAction {
    None,
    Pending,
    Cancel,
    ConfirmDelete(PathBuf),
    ConfirmExport(PathBuf, String),
}
