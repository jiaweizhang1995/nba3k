//! Reusable form widgets and the TUI theme palette.
//!
//! All widgets share a single contract via `FormWidget`: they own their state,
//! render into a ratatui `Frame`, and consume `KeyEvent`s into a small set of
//! `WidgetEvent` outcomes that the parent screen can react to.
//!
//! Wave-1 screens (Home, Calendar, Saves, NewGame) compose these widgets, so
//! the API surface is broader than what Wave-0 uses today; `dead_code` is
//! silenced module-wide for the duration of Wave 0.

#![allow(dead_code)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, Padding, Paragraph, Row, Table},
    Frame,
};

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

/// Color palette used by every TUI widget. Two presets:
/// - `Theme::DEFAULT` — original v1 palette (gray / yellow / cyan accents)
/// - `Theme::TV`     — high-contrast 16-color palette + extra padding, picked
///                     by `--tv` for living-room / projector use.
#[derive(Copy, Clone, Debug)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub muted: Color,
    pub highlight_bg: Color,
    pub highlight_fg: Color,
    pub border: Color,
    /// Block padding multiplier — `0` for default, `1` for TV mode (padding
    /// every block by 1 cell for readability at distance).
    pub block_padding: u16,
}

impl Theme {
    pub const DEFAULT: Theme = Theme {
        bg: Color::Reset,
        fg: Color::Reset,
        accent: Color::Yellow,
        muted: Color::DarkGray,
        highlight_bg: Color::DarkGray,
        highlight_fg: Color::Yellow,
        border: Color::Gray,
        block_padding: 0,
    };

    pub const TV: Theme = Theme {
        bg: Color::Black,
        fg: Color::White,
        accent: Color::Yellow,
        muted: Color::Gray,
        highlight_bg: Color::Blue,
        highlight_fg: Color::White,
        border: Color::White,
        block_padding: 1,
    };

    /// Default text style derived from this theme.
    pub fn text(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }

    /// Highlight style for selected list rows / menu items.
    pub fn highlight(&self) -> Style {
        Style::default()
            .bg(self.highlight_bg)
            .fg(self.highlight_fg)
            .add_modifier(Modifier::BOLD)
    }

    /// Accent style for headers / labels.
    pub fn accent_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Muted style for de-emphasized text.
    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Build a bordered block honoring the theme's border color and padding.
    pub fn block<'a>(&self, title: &'a str) -> Block<'a> {
        let mut b = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.border));
        if !title.is_empty() {
            b = b.title(title);
        }
        if self.block_padding > 0 {
            b = b.padding(Padding::uniform(self.block_padding));
        }
        b
    }

    /// Build an outer region block that shows whether the region owns focus.
    pub fn focus_block<'a>(&self, title: &'a str, active: bool) -> Block<'a> {
        let style = if active {
            self.accent_style()
        } else {
            self.muted_style()
        };
        let mut b = Block::default().borders(Borders::ALL).border_style(style);
        if !title.is_empty() {
            b = b.title(title);
        }
        if self.block_padding > 0 {
            b = b.padding(Padding::uniform(self.block_padding));
        }
        b
    }
}

// ---------------------------------------------------------------------------
// FormWidget contract
// ---------------------------------------------------------------------------

/// Outcome of `handle_key`. Screens treat `Submitted`/`Cancelled` as terminal
/// states; `Selected`/`Toggled` as in-progress signals.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WidgetEvent {
    None,
    Submitted,
    Cancelled,
    /// Picker emits the new index after navigation/select.
    Selected(usize),
    /// MultiSelect emits the toggled index.
    Toggled(usize),
}

/// Common contract for every form widget. Widgets own their state — the parent
/// screen renders them in a `Rect`, forwards key events, and reacts to the
/// returned `WidgetEvent`.
pub trait FormWidget {
    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme);
    fn handle_key(&mut self, key: KeyEvent) -> WidgetEvent;
}

// ---------------------------------------------------------------------------
// TextInput
// ---------------------------------------------------------------------------

/// Single-line text input. Backed by a `String` + cursor index (in chars, not
/// bytes — we count via `chars()` everywhere).
///
/// Example:
/// ```ignore
/// let mut name = TextInput::new("New game name").with_max_len(32);
/// // each tick:
/// name.render(f, area, &theme);
/// match name.handle_key(key) {
///     WidgetEvent::Submitted => start_game(name.value()),
///     WidgetEvent::Cancelled => exit_screen(),
///     _ => {}
/// }
/// ```
#[derive(Clone, Debug)]
pub struct TextInput {
    label: String,
    value: String,
    cursor: usize, // char index
    max_len: Option<usize>,
}

impl TextInput {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: String::new(),
            cursor: 0,
            max_len: None,
        }
    }

    pub fn with_initial(mut self, v: impl Into<String>) -> Self {
        self.value = v.into();
        self.cursor = self.value.chars().count();
        self
    }

    pub fn with_max_len(mut self, n: usize) -> Self {
        self.max_len = Some(n);
        self
    }

    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, v: impl Into<String>) {
        self.value = v.into();
        self.cursor = self.value.chars().count();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    fn at_max(&self) -> bool {
        self.max_len
            .map(|m| self.value.chars().count() >= m)
            .unwrap_or(false)
    }
}

impl FormWidget for TextInput {
    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let chars: Vec<char> = self.value.chars().collect();
        let cursor = self.cursor.min(chars.len());
        let (left, right) = chars.split_at(cursor);
        let line = Line::from(vec![
            Span::styled(format!(" {} ", self.label), theme.accent_style()),
            Span::styled(left.iter().collect::<String>(), theme.text()),
            Span::styled("█", theme.text()),
            Span::styled(right.iter().collect::<String>(), theme.text()),
        ]);
        let p = Paragraph::new(line).block(theme.block(""));
        f.render_widget(p, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> WidgetEvent {
        match key.code {
            KeyCode::Enter => WidgetEvent::Submitted,
            KeyCode::Esc => WidgetEvent::Cancelled,
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let mut chars: Vec<char> = self.value.chars().collect();
                    chars.remove(self.cursor - 1);
                    self.value = chars.into_iter().collect();
                    self.cursor -= 1;
                }
                WidgetEvent::None
            }
            KeyCode::Delete => {
                let mut chars: Vec<char> = self.value.chars().collect();
                if self.cursor < chars.len() {
                    chars.remove(self.cursor);
                    self.value = chars.into_iter().collect();
                }
                WidgetEvent::None
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                WidgetEvent::None
            }
            KeyCode::Right => {
                if self.cursor < self.value.chars().count() {
                    self.cursor += 1;
                }
                WidgetEvent::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                WidgetEvent::None
            }
            KeyCode::End => {
                self.cursor = self.value.chars().count();
                WidgetEvent::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.at_max() {
                    let mut chars: Vec<char> = self.value.chars().collect();
                    chars.insert(self.cursor, c);
                    self.value = chars.into_iter().collect();
                    self.cursor += 1;
                }
                WidgetEvent::None
            }
            _ => WidgetEvent::None,
        }
    }
}

// ---------------------------------------------------------------------------
// NumberInput
// ---------------------------------------------------------------------------

/// Numeric input. Stores the raw string buffer (so partial input like `"-"` is
/// allowed) plus optional `min`/`max` bounds. `value()` returns the parsed `i64`
/// or `None` if the buffer doesn't parse / is out of bounds.
#[derive(Clone, Debug)]
pub struct NumberInput {
    label: String,
    buf: String,
    min: Option<i64>,
    max: Option<i64>,
}

impl NumberInput {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            buf: String::new(),
            min: None,
            max: None,
        }
    }

    pub fn with_bounds(mut self, min: i64, max: i64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    pub fn with_initial(mut self, n: i64) -> Self {
        self.buf = n.to_string();
        self
    }

    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    pub fn raw(&self) -> &str {
        &self.buf
    }

    pub fn value(&self) -> Option<i64> {
        let n = self.buf.parse::<i64>().ok()?;
        if let Some(min) = self.min {
            if n < min {
                return None;
            }
        }
        if let Some(max) = self.max {
            if n > max {
                return None;
            }
        }
        Some(n)
    }
}

impl FormWidget for NumberInput {
    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let line = Line::from(vec![
            Span::styled(format!(" {} ", self.label), theme.accent_style()),
            Span::styled(self.buf.clone(), theme.text()),
            Span::styled("█", theme.text()),
        ]);
        let p = Paragraph::new(line).block(theme.block(""));
        f.render_widget(p, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> WidgetEvent {
        match key.code {
            KeyCode::Enter => WidgetEvent::Submitted,
            KeyCode::Esc => WidgetEvent::Cancelled,
            KeyCode::Backspace => {
                self.buf.pop();
                WidgetEvent::None
            }
            KeyCode::Char(c) if c.is_ascii_digit() || (c == '-' && self.buf.is_empty()) => {
                self.buf.push(c);
                WidgetEvent::None
            }
            _ => WidgetEvent::None,
        }
    }
}

// ---------------------------------------------------------------------------
// Picker
// ---------------------------------------------------------------------------

/// Single-select scrollable list with substring filter. Displays items by
/// formatting them via `display_fn` (caller-supplied at construction time —
/// we store a `Vec<String>` so `T` is purely a phantom for the API).
///
/// Use `Picker::set_filter("celt")` to narrow; the visible index map is
/// rebuilt automatically.
#[derive(Clone, Debug)]
pub struct Picker<T: Clone> {
    title: String,
    items: Vec<T>,
    display: Vec<String>, // pre-rendered labels parallel to items
    filter: String,
    cursor: usize, // index into the *filtered* slice
}

impl<T: Clone> Picker<T> {
    pub fn new(title: impl Into<String>, items: Vec<T>, display: impl Fn(&T) -> String) -> Self {
        let display_strs: Vec<String> = items.iter().map(&display).collect();
        Self {
            title: title.into(),
            items,
            display: display_strs,
            filter: String::new(),
            cursor: 0,
        }
    }

    pub fn set_filter(&mut self, s: impl Into<String>) {
        self.filter = s.into().to_lowercase();
        self.cursor = 0;
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Indices into the original items vec that pass the current filter.
    fn visible(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            (0..self.items.len()).collect()
        } else {
            self.items
                .iter()
                .enumerate()
                .filter(|(i, _)| self.display[*i].to_lowercase().contains(&self.filter))
                .map(|(i, _)| i)
                .collect()
        }
    }

    /// Returns the underlying item at the cursor (post-filter), if any.
    pub fn selected(&self) -> Option<&T> {
        let v = self.visible();
        let idx = *v.get(self.cursor)?;
        self.items.get(idx)
    }

    /// Original-vec index of the cursor (post-filter), if any.
    pub fn selected_index(&self) -> Option<usize> {
        let v = self.visible();
        v.get(self.cursor).copied()
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }
}

impl<T: Clone> FormWidget for Picker<T> {
    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let v = self.visible();
        let items: Vec<ListItem> = v
            .iter()
            .enumerate()
            .map(|(i, idx)| {
                let label = self.display[*idx].clone();
                let style = if i == self.cursor {
                    theme.highlight()
                } else {
                    theme.text()
                };
                ListItem::new(Line::from(Span::styled(label, style)))
            })
            .collect();

        let title = if self.filter.is_empty() {
            format!(" {} ({}) ", self.title, self.items.len())
        } else {
            format!(
                " {} ({}/{}) — filter: {} ",
                self.title,
                v.len(),
                self.items.len(),
                self.filter
            )
        };
        let list = List::new(items).block(theme.block(&title));
        f.render_widget(list, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> WidgetEvent {
        let v = self.visible();
        match key.code {
            KeyCode::Enter => WidgetEvent::Submitted,
            KeyCode::Esc => WidgetEvent::Cancelled,
            KeyCode::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    return WidgetEvent::Selected(self.cursor);
                }
                WidgetEvent::None
            }
            KeyCode::Down => {
                if self.cursor + 1 < v.len() {
                    self.cursor += 1;
                    return WidgetEvent::Selected(self.cursor);
                }
                WidgetEvent::None
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.cursor = 0;
                WidgetEvent::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c.to_ascii_lowercase());
                self.cursor = 0;
                WidgetEvent::None
            }
            _ => WidgetEvent::None,
        }
    }
}

// ---------------------------------------------------------------------------
// MultiSelect
// ---------------------------------------------------------------------------

/// Multi-select scrollable list. Space toggles the cursor row's selected state;
/// Enter submits, Esc cancels.
#[derive(Clone, Debug)]
pub struct MultiSelect<T: Clone> {
    title: String,
    items: Vec<T>,
    display: Vec<String>,
    selected: Vec<bool>,
    cursor: usize,
}

impl<T: Clone> MultiSelect<T> {
    pub fn new(title: impl Into<String>, items: Vec<T>, display: impl Fn(&T) -> String) -> Self {
        let display_strs: Vec<String> = items.iter().map(&display).collect();
        let n = items.len();
        Self {
            title: title.into(),
            items,
            display: display_strs,
            selected: vec![false; n],
            cursor: 0,
        }
    }

    pub fn selected_indices(&self) -> Vec<usize> {
        self.selected
            .iter()
            .enumerate()
            .filter_map(|(i, b)| if *b { Some(i) } else { None })
            .collect()
    }

    pub fn selected_items(&self) -> Vec<&T> {
        self.selected_indices()
            .into_iter()
            .filter_map(|i| self.items.get(i))
            .collect()
    }
}

impl<T: Clone> FormWidget for MultiSelect<T> {
    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let items: Vec<ListItem> = self
            .display
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let mark = if self.selected[i] { "[x]" } else { "[ ]" };
                let style = if i == self.cursor {
                    theme.highlight()
                } else {
                    theme.text()
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{} {}", mark, label),
                    style,
                )))
            })
            .collect();
        let title = format!(
            " {} ({}/{}) ",
            self.title,
            self.selected.iter().filter(|b| **b).count(),
            self.items.len()
        );
        let list = List::new(items).block(theme.block(&title));
        f.render_widget(list, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> WidgetEvent {
        match key.code {
            KeyCode::Enter => WidgetEvent::Submitted,
            KeyCode::Esc => WidgetEvent::Cancelled,
            KeyCode::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                WidgetEvent::None
            }
            KeyCode::Down => {
                if self.cursor + 1 < self.items.len() {
                    self.cursor += 1;
                }
                WidgetEvent::None
            }
            KeyCode::Char(' ') => {
                if let Some(slot) = self.selected.get_mut(self.cursor) {
                    *slot = !*slot;
                    return WidgetEvent::Toggled(self.cursor);
                }
                WidgetEvent::None
            }
            _ => WidgetEvent::None,
        }
    }
}

// ---------------------------------------------------------------------------
// Confirm
// ---------------------------------------------------------------------------

/// Confirmation modal. `Enter` → Submitted; `Esc` → Cancelled.
#[derive(Clone, Debug)]
pub struct Confirm {
    prompt: String,
    /// Default answer shown highlighted when nothing is pressed.
    default_yes: bool,
}

impl Confirm {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            default_yes: false,
        }
    }

    pub fn default_yes(mut self) -> Self {
        self.default_yes = true;
        self
    }
}

impl FormWidget for Confirm {
    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(self.prompt.clone(), theme.accent_style()))
                .alignment(Alignment::Center),
            Line::from(""),
            Line::from("    Enter Confirm    Esc Cancel    ").alignment(Alignment::Center),
        ];
        let p = Paragraph::new(lines).block(theme.block(" Confirm "));
        f.render_widget(p, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> WidgetEvent {
        match key.code {
            KeyCode::Enter => WidgetEvent::Submitted,
            KeyCode::Esc => WidgetEvent::Cancelled,
            _ => WidgetEvent::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn text_input_cursor_navigation_inserts_and_deletes_at_cursor() {
        let mut input = TextInput::new("Save").with_initial("abcd");

        input.handle_key(key(KeyCode::Left));
        input.handle_key(key(KeyCode::Left));
        input.handle_key(key(KeyCode::Char('X')));
        assert_eq!(input.value(), "abXcd");
        assert_eq!(input.cursor, 3);

        input.handle_key(key(KeyCode::Delete));
        assert_eq!(input.value(), "abXd");
        assert_eq!(input.cursor, 3);

        input.handle_key(key(KeyCode::Home));
        input.handle_key(key(KeyCode::Char('>')));
        assert_eq!(input.value(), ">abXd");
        assert_eq!(input.cursor, 1);

        input.handle_key(key(KeyCode::End));
        input.handle_key(key(KeyCode::Backspace));
        assert_eq!(input.value(), ">abX");
        assert_eq!(input.cursor, 4);
    }

    #[test]
    fn confirm_accepts_only_enter_and_esc() {
        let mut confirm = Confirm::new("Quit?");

        assert_eq!(
            confirm.handle_key(key(KeyCode::Char('y'))),
            WidgetEvent::None
        );
        assert_eq!(
            confirm.handle_key(key(KeyCode::Char('n'))),
            WidgetEvent::None
        );
        assert_eq!(
            confirm.handle_key(key(KeyCode::Enter)),
            WidgetEvent::Submitted
        );
        assert_eq!(
            confirm.handle_key(key(KeyCode::Esc)),
            WidgetEvent::Cancelled
        );
    }
}

// ---------------------------------------------------------------------------
// ActionBar
// ---------------------------------------------------------------------------

/// Bottom-bar list of `(key, label)` hints. Right-aligned, themed.
///
/// Example:
/// ```ignore
/// let bar = ActionBar::new(&[("↑↓","Navigate"), ("Enter","Open"), ("Esc","Back")]);
/// bar.render(f, footer_area, &theme);
/// ```
pub struct ActionBar<'a> {
    hints: &'a [(&'a str, &'a str)],
    /// Optional left-aligned status (e.g. last sim message).
    status: Option<&'a str>,
}

impl<'a> ActionBar<'a> {
    pub fn new(hints: &'a [(&'a str, &'a str)]) -> Self {
        Self {
            hints,
            status: None,
        }
    }

    pub fn with_status(mut self, status: &'a str) -> Self {
        self.status = Some(status);
        self
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        // Render as a 2-cell layout: status (flex) | hints (right-aligned).
        let layout = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(self.hints_len() as u16),
            ])
            .split(area);

        // Outer block (drawn under both halves).
        let block = theme.block("");
        f.render_widget(block, area);

        // Inner areas (1-cell margin so border isn't overwritten).
        let inner_left = inset(layout[0]);
        let inner_right = inset(layout[1]);

        if let Some(s) = self.status {
            let p = Paragraph::new(Line::from(Span::styled(s.to_string(), theme.muted_style())))
                .alignment(Alignment::Left);
            f.render_widget(p, inner_left);
        }

        let mut spans: Vec<Span> = Vec::new();
        for (i, (k, label)) in self.hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ", theme.text()));
            }
            spans.push(Span::styled(format!(" {} ", k), theme.accent_style()));
            spans.push(Span::styled(format!(" {}", label), theme.text()));
        }
        let p = Paragraph::new(Line::from(spans)).alignment(Alignment::Right);
        f.render_widget(p, inner_right);
    }

    fn hints_len(&self) -> usize {
        // Approximate width: " key  label" * n  + separators.
        let mut n = 0usize;
        for (i, (k, label)) in self.hints.iter().enumerate() {
            if i > 0 {
                n += 2;
            }
            n += k.chars().count() + 3 + label.chars().count() + 1;
        }
        n + 2 // borders
    }
}

fn inset(r: Rect) -> Rect {
    Rect {
        x: r.x.saturating_add(1),
        y: r.y.saturating_add(1),
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(2),
    }
}

// ---------------------------------------------------------------------------
// Helper renderers reused by screens
// ---------------------------------------------------------------------------

/// Helper to centered-render a short message inside a bordered block.
pub fn centered_block(f: &mut Frame, area: Rect, theme: &Theme, title: &str, lines: &[&str]) {
    let block = theme.block(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let v_pad = inner.height.saturating_sub(lines.len() as u16) / 2;
    let mut out: Vec<Line> = Vec::with_capacity(v_pad as usize + lines.len());
    for _ in 0..v_pad {
        out.push(Line::from(""));
    }
    for (i, l) in lines.iter().enumerate() {
        let style = if i == 0 {
            theme.accent_style()
        } else {
            theme.text()
        };
        out.push(Line::from(Span::styled(l.to_string(), style)).alignment(Alignment::Center));
    }
    let p = Paragraph::new(out).alignment(Alignment::Center);
    f.render_widget(p, inner);
}

/// Convenience: build a 2-column key/value table for "info" panels (e.g.
/// `Status:  RegularSeason`).
pub fn kv_table<'a>(rows: &'a [(&'a str, String)], theme: &Theme, title: &'a str) -> Table<'a> {
    let body: Vec<Row> = rows
        .iter()
        .map(|(k, v)| {
            Row::new(vec![
                Cell::from(Span::styled(*k, theme.muted_style())),
                Cell::from(Span::styled(v.clone(), theme.text())),
            ])
        })
        .collect();
    Table::new(body, [Constraint::Length(14), Constraint::Min(0)]).block(theme.block(title))
}
