// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crossterm::event::KeyModifiers;

use super::*;

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn clicks_clear_selection_and_only_dragging_selects_characters() {
    let mut selection = LogSelection::default();
    let area = Rect::new(0, 0, 20, 4);
    let lines = ["alpha", "beta"];
    assert!(!selection.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), 4, 0),
        area,
        0,
        &lines
    ));
    assert!(selection.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 1, 0),
        area,
        0,
        &lines
    ));
    assert!(selection.range.is_none());
    assert!(selection.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), 4, 0),
        area,
        0,
        &lines
    ));
    assert_eq!(selection.text(&lines).as_deref(), Some("lph"));
    assert!(selection.handle_mouse(
        mouse(MouseEventKind::Up(MouseButton::Left), 4, 0),
        area,
        0,
        &lines
    ));
    assert!(!selection.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), 5, 0),
        area,
        0,
        &lines
    ));

    let search = SearchState {
        query: "alpha".to_owned(),
        ..Default::default()
    };
    let highlighted = selection.highlight_line(0, "alpha", &search, Style::default());
    assert_eq!(
        highlighted
            .spans
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        "lph"
    );
    assert!(
        highlighted
            .spans
            .iter()
            .all(|span| span.style.add_modifier.contains(Modifier::UNDERLINED))
    );

    selection.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 2, 0),
        area,
        0,
        &lines,
    );
    selection.handle_mouse(
        mouse(MouseEventKind::Up(MouseButton::Left), 2, 0),
        area,
        0,
        &lines,
    );
    assert!(selection.range.is_none());
    assert!(selection.text(&lines).is_none());

    selection.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 2, 0),
        area,
        0,
        &lines,
    );
    selection.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), 0, 1),
        area,
        0,
        &lines,
    );
    assert_eq!(selection.text(&lines).as_deref(), Some("pha\n"));
    selection.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), 0, 3),
        area,
        0,
        &lines,
    );
    assert_eq!(selection.text(&lines).as_deref(), Some("pha\nbeta"));
}

#[test]
fn selection_handles_reverse_multiline_drags_scroll_and_unicode_cells() {
    let mut selection = LogSelection::default();
    let area = Rect::new(10, 5, 20, 4);
    let lines = ["hidden", "alpha", "beta"];
    selection.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 12, 6),
        area,
        1,
        &lines,
    );
    selection.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), 11, 5),
        area,
        1,
        &lines,
    );
    assert_eq!(selection.text(&lines).as_deref(), Some("lpha\nbe"));

    let lines = ["a界e\u{0301}🙂z"];
    selection.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 12, 5),
        area,
        0,
        &lines,
    );
    selection.handle_mouse(
        mouse(MouseEventKind::Drag(MouseButton::Left), 16, 5),
        area,
        0,
        &lines,
    );
    assert_eq!(selection.text(&lines).as_deref(), Some("界e\u{0301}🙂"));
    let highlighted =
        selection.highlight_line(0, lines[0], &SearchState::default(), Style::default());
    assert_eq!(
        highlighted
            .spans
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        "界e\u{0301}🙂"
    );
}
