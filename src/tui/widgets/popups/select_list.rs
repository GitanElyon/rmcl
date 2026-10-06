// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
    widgets::{List, ListItem, ListState, StatefulWidget},
};

use crate::config::theme::THEME;
use crate::instance::models::ModLoader;

pub(crate) const MOD_LOADERS: [ModLoader; 5] = [
    ModLoader::Vanilla,
    ModLoader::Fabric,
    ModLoader::Forge,
    ModLoader::NeoForge,
    ModLoader::Quilt,
];

pub(crate) fn render(items: Vec<ListItem<'_>>, selected: usize, area: Rect, buffer: &mut Buffer) {
    let theme = THEME.as_ref();
    let items = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            if index == selected {
                item.style(Style::default().bg(theme.stripe()))
            } else {
                item
            }
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .style(Style::default().fg(theme.text()))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(Span::styled(
            "▶ ",
            Style::default()
                .fg(theme.accent())
                .add_modifier(Modifier::BOLD),
        ));
    let mut state = ListState::default().with_selected(Some(selected));
    StatefulWidget::render(list, area, buffer, &mut state);
}

#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::*;
    use crate::tui::widgets::status_badge;

    #[test]
    fn selected_labels_use_normal_text_bold_and_keep_the_accent_marker() {
        let theme = THEME.as_ref();
        let area = Rect::new(0, 0, 30, 2);
        let mut buffer = Buffer::empty(area);
        render(
            vec![ListItem::new("26.3"), ListItem::new("26.2")],
            0,
            area,
            &mut buffer,
        );

        let selected = &buffer[(2, 0)];
        assert_eq!(selected.fg, theme.text());
        assert_eq!(selected.bg, theme.stripe());
        assert!(selected.modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(0, 0)].fg, theme.accent());
        assert_eq!(buffer[(2, 1)].fg, theme.text());
        assert!(!buffer[(2, 1)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn styled_list_preserves_badges_on_the_selected_row() {
        let theme = THEME.as_ref();
        let area = Rect::new(0, 0, 30, 1);
        let mut buffer = Buffer::empty(area);
        let badge = status_badge("Auto", theme.success());
        let items = vec![ListItem::new(Line::from(vec![Span::raw("Java  "), badge]))];

        render(items, 0, area, &mut buffer);

        let badge_cell = buffer.cell((8, 0)).unwrap();
        assert_eq!(badge_cell.bg, theme.success());
        assert_eq!(badge_cell.fg, theme.background());
    }

    #[test]
    fn version_popup_installed_badge_keeps_colors_on_selection() {
        let theme = THEME.as_ref();
        let area = Rect::new(0, 0, 30, 2);
        let mut buffer = Buffer::empty(area);
        let items = vec![
            ListItem::new(Line::from(vec![Span::raw("1.11.3")])),
            ListItem::new(Line::from(vec![
                Span::raw("1.11.2  "),
                Span::styled(
                    " Installed ",
                    Style::default()
                        .fg(theme.background())
                        .bg(theme.success())
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
        ];

        render(items, 1, area, &mut buffer);

        let badge_cell = buffer.cell((10, 1)).unwrap();
        assert_eq!(badge_cell.bg, theme.success());
        assert_eq!(badge_cell.fg, theme.background());
    }
}
