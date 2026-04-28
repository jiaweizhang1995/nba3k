//! Settings picker screen. The shell is expected to map `LanguageChoice` to
//! its persistent i18n type (`nba3k_core::Lang`) when that API is wired.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
    Frame,
};
use std::cell::RefCell;

use crate::tui::widgets::Theme;
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    Consumed,
    Cancel,
    Commit(LanguageChoice),
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

pub fn handle_key(key: KeyEvent) -> SettingsAction {
    match key.code {
        KeyCode::Up => {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                if st.cursor > 0 {
                    st.cursor -= 1;
                }
            });
            SettingsAction::Consumed
        }
        KeyCode::Down => {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                if st.cursor + 1 < LANGUAGES.len() {
                    st.cursor += 1;
                }
            });
            SettingsAction::Consumed
        }
        KeyCode::Enter => {
            let lang = STATE.with(|s| LANGUAGES[s.borrow().cursor]);
            SettingsAction::Commit(lang)
        }
        KeyCode::Esc => SettingsAction::Cancel,
        _ => SettingsAction::None,
    }
}

fn index_of(lang: LanguageChoice) -> usize {
    LANGUAGES.iter().position(|v| *v == lang).unwrap_or(0)
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
