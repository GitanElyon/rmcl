// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn list_and_viewer_navigation_loads_and_scrolls_the_selected_log() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.log");
    let second = temp.path().join("second.log");
    std::fs::write(&first, "first").unwrap();
    std::fs::write(&second, "second\nline").unwrap();

    let mut state = LogsState {
        entries: vec![
            LogFileEntry {
                name: "first.log".to_owned(),
                path: first,
            },
            LogFileEntry {
                name: "second.log".to_owned(),
                path: second,
            },
        ],
        ..Default::default()
    };
    state.list_state.selected = Some(0);

    assert!(handle_key(&key(KeyCode::Down), &mut state));
    assert_eq!(state.list_state.selected, Some(1));
    assert_eq!(state.viewer_lines, ["second", "line"]);

    assert!(handle_key(&key(KeyCode::Enter), &mut state));
    assert!(state.viewer_focused);
    state.viewer_max_scroll = 4;

    assert!(handle_key(&key(KeyCode::Char('G')), &mut state));
    assert_eq!(state.viewer_scroll, 4);
    assert!(handle_key(&key(KeyCode::Char('g')), &mut state));
    assert_eq!(state.viewer_scroll, 0);

    assert!(handle_key(&key(KeyCode::Esc), &mut state));
    assert!(!state.viewer_focused);
}

#[test]
fn rescans_and_runtime_changes_keep_the_selected_log_and_viewer_in_sync() {
    let temp = tempfile::tempdir().unwrap();
    let older = LogFileEntry {
        name: "older.log".to_owned(),
        path: temp.path().join("older.log"),
    };
    let newer = LogFileEntry {
        name: "newer.log".to_owned(),
        path: temp.path().join("newer.log"),
    };
    std::fs::write(&older.path, "older content").unwrap();
    std::fs::write(&newer.path, "newer content").unwrap();
    let name = "logs-rescan-identity";
    let mut state = LogsState {
        loaded_for: Some(name.to_owned()),
        entries: vec![older.clone()],
        ..Default::default()
    };
    state.list_state.selected = Some(0);
    state.load_selected_content();
    *state.pending.lock().unwrap() = Some((name.to_owned(), vec![newer.clone(), older.clone()]));
    state.drain_pending();
    assert_eq!(state.list_state.selected, Some(1));
    assert_eq!(state.viewer_lines, ["older content"]);

    crate::instance::runtime::set_state(name, crate::instance::runtime::RunState::Running);
    state.drain_pending();
    assert_eq!(state.list_state.selected, Some(2));
    state.search.query = "older".to_owned();
    state.list_state.selected = Some(0);
    state.load_selected_content();
    assert!(!state.is_live_selected());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 4)).unwrap();
    terminal
        .draw(|frame| render_viewer(frame, frame.area(), &mut state))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("older content"));

    state.search.query.clear();
    state.list_state.selected = Some(0);
    state.load_selected_content();
    crate::instance::runtime::remove(name);
    state.drain_pending();
    assert_eq!(state.viewer_lines, ["newer content"]);
    assert_eq!(state.selected_path, Some(newer.path));
}
