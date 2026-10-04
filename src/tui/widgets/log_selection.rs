// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;

use super::search::SearchState;

// Positions are (filtered line index, UTF-8 byte boundary). Mouse columns
// are converted using grapheme widths, including wide and combining glyphs.
#[derive(Debug, Default)]
pub(crate) struct LogSelection {
    anchor: Option<(usize, usize)>,
    pub range: Option<((usize, usize), (usize, usize))>,
}

impl LogSelection {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn finish(&mut self) {
        self.anchor = None;
    }

    pub fn handle_mouse(
        &mut self,
        event: MouseEvent,
        area: Rect,
        scroll: usize,
        lines: &[impl AsRef<str>],
    ) -> bool {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.clear();
                if area.is_empty()
                    || !area.contains(Position::new(event.column, event.row))
                    || lines.is_empty()
                {
                    return false;
                }
                self.anchor = Some(mouse_position(event, area, scroll, lines));
                true
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(anchor) = self.anchor else {
                    return false;
                };
                if lines.is_empty() {
                    self.clear();
                    return true;
                }
                let end = mouse_position(event, area, scroll, lines);
                self.range = (anchor != end).then_some((anchor, end));
                true
            }
            MouseEventKind::Up(MouseButton::Left) => self.anchor.take().is_some(),
            _ => false,
        }
    }

    pub fn text(&self, lines: &[impl AsRef<str>]) -> Option<String> {
        let (start, end) = self.ordered()?;
        let selected = lines.get(start.0..=end.0)?;
        let mut text = Vec::with_capacity(selected.len());
        for (offset, line) in selected.iter().enumerate() {
            let line = line.as_ref();
            let from = if offset == 0 { start.1 } else { 0 };
            let to = if offset == selected.len() - 1 {
                end.1
            } else {
                line.len()
            };
            text.push(line.get(from..to)?);
        }
        Some(text.join("\n"))
    }

    pub fn highlight_line(
        &self,
        index: usize,
        text: &str,
        search: &SearchState,
        style: Style,
    ) -> Line<'static> {
        let Some((start, end)) = self.ordered() else {
            return search.highlight_line(text, style);
        };
        if index < start.0 || index > end.0 {
            return search.highlight_line(text, style);
        }
        let from = if index == start.0 { start.1 } else { 0 };
        let to = if index == end.0 { end.1 } else { text.len() };
        if !text.is_char_boundary(from) || !text.is_char_boundary(to) {
            return search.highlight_line(text, style);
        }
        let mut offset = 0;
        let mut spans = Vec::new();
        for span in search.highlight_spans(text, style) {
            let len = span.content.len();
            let left = from.saturating_sub(offset).min(len);
            let right = to.saturating_sub(offset).min(len);
            for (range, selected) in [(0..left, false), (left..right, true), (right..len, false)] {
                if !range.is_empty() {
                    spans.push(Span::styled(
                        span.content[range].to_owned(),
                        if selected {
                            span.style.add_modifier(Modifier::REVERSED)
                        } else {
                            span.style
                        },
                    ));
                }
            }
            offset += len;
        }
        Line::from(spans)
    }

    fn ordered(&self) -> Option<((usize, usize), (usize, usize))> {
        self.range.map(|(a, b)| (a.min(b), a.max(b)))
    }
}

fn mouse_position(
    event: MouseEvent,
    area: Rect,
    scroll: usize,
    lines: &[impl AsRef<str>],
) -> (usize, usize) {
    let row = event
        .row
        .saturating_sub(area.y)
        .min(area.height.saturating_sub(1));
    let index = scroll + usize::from(row);
    if index >= lines.len() {
        return (lines.len() - 1, lines.last().unwrap().as_ref().len());
    }
    let column = usize::from(event.column.saturating_sub(area.x).min(area.width));
    let text = lines[index].as_ref();
    let mut width = 0;
    for (byte, grapheme) in text.grapheme_indices(true) {
        width += Span::raw(grapheme).width();
        if column < width {
            return (index, byte);
        }
    }
    (index, text.len())
}

#[cfg(test)]
#[path = "../tests/widgets/log_selection.rs"]
mod tests;
