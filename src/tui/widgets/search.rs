// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crossterm::event::KeyModifiers;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::config::theme::THEME;

#[derive(Debug, Default, Clone)]
pub struct SearchState {
    pub query: String,
    pub active: bool,
}

impl SearchState {
    pub fn activate(&mut self) {
        self.active = true;
    }

    pub fn deactivate(&mut self) {
        self.active = false;
        self.query.clear();
    }

    // exit search mode but keep the filter active so the user can
    // navigate the filtered results
    pub fn confirm(&mut self) {
        self.active = false;
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
    }

    pub fn backspace(&mut self, modifiers: KeyModifiers) {
        backspace(&mut self.query, modifiers);
    }

    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    pub fn matches(&self, text: &str) -> bool {
        if self.query.is_empty() {
            return true;
        }
        text.to_lowercase().contains(&self.query.to_lowercase())
    }

    pub fn highlight_spans(&self, text: &str, base_style: Style) -> Vec<Span<'static>> {
        if self.query.is_empty() {
            return vec![Span::styled(text.to_owned(), base_style)];
        }

        let query_lower = self.query.to_lowercase();
        let text_lower = text.to_lowercase();
        let mut original_ranges = Vec::with_capacity(text_lower.len());
        for (start, character) in text.char_indices() {
            let lowercase_bytes = character.to_lowercase().map(char::len_utf8).sum::<usize>();
            original_ranges.extend(std::iter::repeat_n(
                (start, start + character.len_utf8()),
                lowercase_bytes,
            ));
        }
        let mut spans = Vec::new();
        let mut last = 0;

        for (lower_start, _) in text_lower.match_indices(&query_lower) {
            let start = original_ranges[lower_start].0;
            let end = original_ranges[lower_start + query_lower.len() - 1].1;
            if start < last {
                continue;
            }
            if start > last {
                spans.push(Span::styled(text[last..start].to_owned(), base_style));
            }
            spans.push(Span::styled(
                text[start..end].to_owned(),
                base_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ));
            last = end;
        }

        if last < text.len() {
            spans.push(Span::styled(text[last..].to_owned(), base_style));
        }

        if spans.is_empty() {
            spans.push(Span::styled(text.to_owned(), base_style));
        }
        spans
    }

    pub fn highlight_line(&self, text: &str, base_style: Style) -> Line<'static> {
        Line::from(self.highlight_spans(text, base_style))
    }

    pub fn title_line(&self) -> Option<Line<'static>> {
        if !self.active && self.query.is_empty() {
            return None;
        }

        let theme = THEME.as_ref();
        let dim = Style::default().fg(theme.text_dim());
        let accent = Style::default()
            .fg(theme.text_dim())
            .add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled(" / ", dim),
            Span::styled(self.query.clone(), accent),
        ];

        if self.active {
            spans.push(Span::styled("\u{2588}", accent));
        }

        spans.push(Span::raw(" "));

        Some(Line::from(spans).right_aligned())
    }
}

pub fn backspace(text: &mut String, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::CONTROL) {
        let cursor = text.chars().count();
        delete_previous_word(text, cursor);
    } else {
        text.pop();
    }
}

pub fn delete_previous_word(text: &mut String, cursor: usize) -> usize {
    let mut chars = text.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());
    let mut start = cursor;
    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    chars.drain(start..cursor);
    *text = chars.into_iter().collect();
    start
}

#[cfg(test)]
#[path = "../tests/widgets/search.rs"]
mod tests;
