// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crate::config::theme::THEME;
use crossterm::event::KeyEvent;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Scrollbar, ScrollbarOrientation},
};

pub mod account;
pub mod content;
pub mod instances;
pub mod logs_viewer;
pub mod markdown;
pub mod popups;
pub mod screenshots_grid;
pub mod search;
pub mod settings;
pub mod status;

pub fn styled_title(title: &str, highlight: bool) -> Line<'_> {
    let theme = THEME.as_ref();
    if !highlight || title.is_empty() {
        Line::from(Span::raw(title))
    } else {
        let mut chars = title.chars();
        let first = chars.next().unwrap_or_default().to_string();
        let rest: String = chars.collect();
        Line::from(vec![
            Span::styled(first, Style::default().fg(theme.accent())),
            Span::styled(rest, Style::default().fg(theme.text())),
        ])
    }
}

pub(crate) fn status_badge_style(color: Color) -> Style {
    Style::default()
        .fg(THEME.as_ref().background())
        .bg(color)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn status_badge(label: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(format!(" {} ", label.into()), status_badge_style(color))
}

pub(crate) fn scrollbar(color: Color) -> Scrollbar<'static> {
    Scrollbar::default()
        .orientation(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("\u{25b2}"))
        .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
        .thumb_symbol("\u{2551}")
        .track_symbol(Some(""))
        .end_symbol(Some("\u{25bc}"))
}

pub trait WidgetKey {
    fn handle_key(&mut self, key_event: &KeyEvent);
}
