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
fn shift_enter_without_a_selected_log_is_swallowed_without_opening() {
    let mut state = LogsState::default();
    let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
    assert!(handle_key(&shift_enter, &mut state));
    assert!(!state.viewer_focused);
}

#[test]
fn viewer_pages_by_visible_height() {
    let mut state = LogsState {
        viewer_focused: true,
        viewer_max_scroll: 30,
        viewer_page: 10,
        ..Default::default()
    };
    let page = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert!(handle_key(&page(KeyCode::PageDown), &mut state));
    assert_eq!(state.viewer_scroll, 10);
    assert!(handle_key(&page(KeyCode::PageDown), &mut state));
    assert_eq!(state.viewer_scroll, 20);
    assert!(handle_key(&page(KeyCode::PageUp), &mut state));
    assert_eq!(state.viewer_scroll, 10);
    assert!(handle_key(&page(KeyCode::Home), &mut state));
    assert_eq!(state.viewer_scroll, 0);
    assert!(handle_key(&page(KeyCode::End), &mut state));
    assert_eq!(state.viewer_scroll, 30);
}

#[test]
fn yank_copies_the_selected_viewer_lines() {
    let mut state = LogsState {
        viewer_lines: vec!["first".to_owned(), "second".to_owned(), "third".to_owned()],
        selection: Some((2, 0)),
        ..Default::default()
    };
    let yank = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
    assert!(handle_key(&yank, &mut state));
    assert!(
        crate::feedback::errors::peek_all_errors()
            .iter()
            .any(|event| event.message.contains("Copied 3 line(s)"))
    );
}

#[test]
fn log_levels_share_one_color_mapping_in_viewer_and_overlay() {
    let theme = crate::config::theme::THEME.as_ref();
    for line in [
        "12:17:29:ERROR:rmcl::net: boom",
        "something failed with error",
        "[20:10:44] [Render thread/ERROR]: boom",
        "[STDERR] boom",
    ] {
        assert_eq!(line_level_style(line).fg, Some(theme.error()), "{line}");
    }
    for line in [
        "12:17:29:WARN:rmcl: careful",
        "[20:10:44] [Server thread/WARN]: careful",
    ] {
        assert_eq!(line_level_style(line).fg, Some(theme.warning()), "{line}");
    }
    assert_eq!(
        line_level_style("12:17:28:DEBUG:rmcl::net: fetching").fg,
        Some(theme.text_dim())
    );
    assert_eq!(
        line_level_style("[20:10:46] [Render thread/INFO]: done").fg,
        Some(theme.text())
    );
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

#[test]
fn unclassified_lines_have_no_level_and_are_never_filtered() {
    use crate::instance::launch::parser::LogLevel;
    assert_eq!(level_of_line("just some output"), None);
    assert_eq!(
        level_of_line("[20:10:44] [Render thread/INFO]: hello"),
        Some(LogLevel::Info)
    );
    assert_eq!(
        level_of_line("tracing trace message"),
        Some(LogLevel::Trace)
    );
    assert!(!level_hidden(&[LogLevel::Debug], "just some output"));
}

#[test]
fn level_filter_panel_toggles_resets_and_closes() {
    use crate::instance::launch::parser::LogLevel;
    let mut state = LogsState::default();
    let press = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert!(handle_key(&press(KeyCode::Char('f')), &mut state));
    assert!(state.filter_open);
    assert!(handle_key(&press(KeyCode::Enter), &mut state));
    assert_eq!(state.hidden_levels, vec![LogLevel::Error]);
    assert!(handle_key(&press(KeyCode::Char('j')), &mut state));
    assert!(handle_key(&press(KeyCode::Enter), &mut state));
    assert_eq!(state.hidden_levels, vec![LogLevel::Error, LogLevel::Warn]);
    assert!(handle_key(&press(KeyCode::Enter), &mut state));
    assert_eq!(state.hidden_levels, vec![LogLevel::Error]);
    assert!(handle_key(&press(KeyCode::Char('r')), &mut state));
    assert!(state.hidden_levels.is_empty());
    assert!(handle_key(&press(KeyCode::Esc), &mut state));
    assert!(!state.filter_open);
}

#[test]
fn level_filter_hides_matching_lines_but_keeps_unclassified_ones() {
    use crate::instance::launch::parser::LogLevel;
    fn rendered(state: &mut LogsState) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(70, 6)).unwrap();
        terminal
            .draw(|frame| render_viewer(frame, frame.area(), state))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }
    let mut state = LogsState {
        viewer_lines: vec![
            "12:17:28:DEBUG:rmcl::net: fetching versions".to_owned(),
            "[20:10:46] [Render thread/INFO]: done".to_owned(),
            "plain line without a level".to_owned(),
        ],
        ..Default::default()
    };
    assert!(rendered(&mut state).contains("fetching versions"));
    state.hidden_levels.push(LogLevel::Debug);
    let shown = rendered(&mut state);
    assert!(!shown.contains("fetching versions"));
    assert!(shown.contains("done"));
    assert!(shown.contains("plain line"));
}
