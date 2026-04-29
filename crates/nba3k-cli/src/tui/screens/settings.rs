use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
    Frame,
};
use std::cell::RefCell;

use crate::state::AppState;
use crate::tui::widgets::Theme;
use crate::tui::TuiApp;
use nba3k_core::{t, Lang, T};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LanguageChoice {
    En,
    Zh,
}

impl LanguageChoice {
    pub fn value(self) -> &'static str {
        match self {
            LanguageChoice::En => "en",
            LanguageChoice::Zh => "zh",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LanguageChoice::En => "English",
            LanguageChoice::Zh => "中文",
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct SettingsState {
    cursor: usize,
    lang_cursor: usize,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            cursor: 0,
            lang_cursor: 0,
        }
    }
}

const LANGUAGES: [LanguageChoice; 2] = [LanguageChoice::En, LanguageChoice::Zh];

thread_local! {
    static STATE: RefCell<SettingsState> = RefCell::new(SettingsState::default());
}

pub fn reset(current: LanguageChoice) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.cursor = 0;
        st.lang_cursor = index_of(current);
    });
}

pub fn render(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    lang: Lang,
    current: LanguageChoice,
    god_mode: bool,
) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.cursor > 1 {
            st.cursor = 0;
        }
        if st.lang_cursor >= LANGUAGES.len() {
            st.lang_cursor = index_of(current);
        }
    });

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(3),
        ])
        .split(area);

    let title = Paragraph::new(Line::from(Span::styled(
        t(lang, T::SettingsTitle),
        theme.accent_style(),
    )))
    .alignment(Alignment::Center)
    .block(theme.block(""));
    f.render_widget(title, parts[0]);

    STATE.with(|s| {
        let st = s.borrow();
        let lang_choice = LANGUAGES[st.lang_cursor];
        let language_style = if st.cursor == 0 {
            theme.highlight()
        } else if lang_choice == current {
            theme.accent_style()
        } else {
            theme.text()
        };
        let god_style = if st.cursor == 1 {
            theme.highlight()
        } else {
            theme.text()
        };
        let items = vec![
            ListItem::new(Line::from(Span::styled(
                format!(
                    "{}{:<12} [{}]",
                    if st.cursor == 0 { "> " } else { "  " },
                    t(lang, T::SettingsLanguage),
                    lang_choice.label()
                ),
                language_style,
            ))),
            ListItem::new(Line::from(Span::styled(
                format!(
                    "{}{:<12} [{}]",
                    if st.cursor == 1 { "> " } else { "  " },
                    t(lang, T::SettingsGodMode),
                    if god_mode {
                        t(lang, T::SettingsOn)
                    } else {
                        t(lang, T::SettingsOff)
                    }
                ),
                god_style,
            ))),
        ];
        let picker = List::new(items).block(theme.block(t(lang, T::SettingsTitle)));
        f.render_widget(picker, centered(parts[1], 42, 6));
    });

    let hint = Line::from(vec![
        Span::styled(" Up/Down ", theme.accent_style()),
        Span::styled(format!("{}   ", t(lang, T::CommonMove)), theme.text()),
        Span::styled(" Space/Enter ", theme.accent_style()),
        Span::styled(format!("{}   ", t(lang, T::CommonConfirm)), theme.text()),
        Span::styled(" Esc ", theme.accent_style()),
        Span::styled(t(lang, T::CommonBack), theme.text()),
    ]);
    f.render_widget(Paragraph::new(hint).block(theme.block("")), parts[2]);
}

pub fn handle_key(app: &mut AppState, tui: &mut TuiApp, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Up => {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                if st.cursor > 0 {
                    st.cursor -= 1;
                }
            });
            Ok(true)
        }
        KeyCode::Down => {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                if st.cursor < 1 {
                    st.cursor += 1;
                }
            });
            Ok(true)
        }
        KeyCode::Left | KeyCode::Right => {
            let commit = STATE.with(|s| {
                let mut st = s.borrow_mut();
                if st.cursor == 0 {
                    st.lang_cursor = 1 - st.lang_cursor;
                    Some(LANGUAGES[st.lang_cursor])
                } else {
                    None
                }
            });
            if let Some(choice) = commit {
                commit_lang(app, tui, choice);
            }
            Ok(true)
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let action = STATE.with(|s| {
                let mut st = s.borrow_mut();
                if st.cursor == 0 {
                    st.lang_cursor = 1 - st.lang_cursor;
                    SettingsAction::Language(LANGUAGES[st.lang_cursor])
                } else {
                    SettingsAction::GodMode(!tui.god_mode)
                }
            });
            match action {
                SettingsAction::Language(choice) => commit_lang(app, tui, choice),
                SettingsAction::GodMode(enabled) => commit_god_mode(app, tui, enabled),
            }
            Ok(true)
        }
        KeyCode::Esc => Ok(false),
        _ => Ok(false),
    }
}

enum SettingsAction {
    Language(LanguageChoice),
    GodMode(bool),
}

fn commit_lang(app: &mut AppState, tui: &mut TuiApp, choice: LanguageChoice) {
    let lang = lang_from_choice(choice);
    tui.apply_language(lang);
    let value = tui.lang.as_setting();
    if let Ok(store) = app.store() {
        let _ = store.write_setting("language", value);
    }
    let _ = crate::config::write_lang(value);
    tui.last_msg = Some(t(tui.lang, T::SettingsSaved).to_string());
}

fn commit_god_mode(app: &mut AppState, tui: &mut TuiApp, enabled: bool) {
    tui.god_mode = enabled;
    app.force_god = enabled;
    let value = if enabled { "on" } else { "off" };
    if let Ok(store) = app.store() {
        let _ = store.write_setting("god_mode", value);
    }
    let _ = crate::config::write_god_mode(enabled);
    tui.last_msg = Some(t(tui.lang, T::SettingsSaved).to_string());
}

fn index_of(lang: LanguageChoice) -> usize {
    LANGUAGES.iter().position(|v| *v == lang).unwrap_or(0)
}

fn lang_from_choice(choice: LanguageChoice) -> Lang {
    match choice {
        LanguageChoice::En => Lang::En,
        LanguageChoice::Zh => Lang::Zh,
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}
