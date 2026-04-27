//! Placeholder "Coming in M21/M22" screen used for Roster/Rotation/Trades/
//! Draft/Finance until those features ship.

use ratatui::{layout::Rect, Frame};

use crate::tui::widgets::{centered_block, Theme};

/// Render the centered "Coming in <expected>" placeholder.
///
/// `name` is the screen title (e.g. "Roster"); `expected` names the milestone
/// the screen ships in (e.g. "M21").
pub fn render_stub(f: &mut Frame, area: Rect, theme: &Theme, name: &str, expected: &str) {
    let title = format!(" {} ", name);
    let coming = format!("Coming in {}", expected);
    let lines: &[&str] = &[
        name,
        "",
        coming.as_str(),
        "",
        "Press Esc to return to menu.",
    ];
    centered_block(f, area, theme, &title, lines);
}
