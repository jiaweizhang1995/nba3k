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
}

impl Default for SettingsState {
    fn default() -> Self {
        Self { cursor: 0 }
    }
}

const LANGUAGES: [LanguageChoice; 2] = [LanguageChoice::En, LanguageChoice::Zh];

thread_local! {
    static STATE: RefCell<SettingsState> = RefCell::new(SettingsState::default());
}

pub fn reset(current: LanguageChoice) {
    STATE.with(|s| {
        s.borrow_mut().cursor = index_of(current);
    });
}

pub fn render(f: &mut Frame, area: Rect, theme: &Theme, lang: Lang, current: LanguageChoice) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.cursor >= LANGUAGES.len() {
            st.cursor = index_of(current);
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
        let items: Vec<ListItem> = LANGUAGES
            .iter()
            .enumerate()
            .map(|(idx, lang)| {
                let selected = idx == st.cursor;
                let active = *lang == current;
                let style = if selected {
                    theme.highlight()
                } else if active {
                    theme.accent_style()
                } else {
                    theme.text()
                };
                let marker = if selected {
                    "> "
                } else if active {
                    "* "
                } else {
                    "  "
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{}{}", marker, lang.label()),
                    style,
                )))
            })
            .collect();
        let picker = List::new(items).block(theme.block(t(lang, T::SettingsLanguage)));
        f.render_widget(picker, centered(parts[1], 34, 4));
    });

    let hint = Line::from(vec![
        Span::styled(" Up/Down ", theme.accent_style()),
        Span::styled(format!("{}   ", t(lang, T::CommonMove)), theme.text()),
        Span::styled(" Enter ", theme.accent_style()),
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
                if st.cursor + 1 < LANGUAGES.len() {
                    st.cursor += 1;
                }
            });
            Ok(true)
        }
        KeyCode::Enter => {
            let choice = STATE.with(|s| LANGUAGES[s.borrow().cursor]);
            commit_lang(app, tui, choice);
            Ok(true)
        }
        KeyCode::Esc => Ok(false),
        _ => Ok(false),
    }
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
