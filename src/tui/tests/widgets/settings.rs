// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn narrow_settings_shows_the_selected_pane() {
    let tmp = tempfile::tempdir().unwrap();
    let mut state = SettingsState::new(tmp.path().to_path_buf());
    let instance: InstanceConfig = serde_json::from_value(serde_json::json!({
        "name": "Test", "game_version": "1.21", "loader": "vanilla",
        "loader_version": null, "created": "2026-01-01T00:00:00Z", "java_path": "test-java"
    }))
    .unwrap();
    state.java_key = Some("test-java".to_owned());
    state.java_label = "jdk17".to_owned();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(35, 8)).unwrap();

    terminal
        .draw(|frame| {
            render(
                frame,
                frame.area(),
                FocusedArea::Settings,
                &mut state,
                Some(&instance),
            )
        })
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains(LOCAL_PROFILE_LABEL));

    state.pane = SettingsPane::Info;
    terminal
        .draw(|frame| {
            render(
                frame,
                frame.area(),
                FocusedArea::Settings,
                &mut state,
                Some(&instance),
            )
        })
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Memory"));
    assert!(text.contains("jdk17"));
}

#[test]
fn removing_selected_last_profile_clamps_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut state = SettingsState::new(tmp.path().to_path_buf());
    state.profiles = vec!["first".to_string(), "second".to_string()];
    state.active_profile = Some("second".to_string());
    state.list_state.selected = Some(2);

    state.remove_profile("second");

    assert_eq!(state.profiles, vec!["first"]);
    assert_eq!(state.active_profile, None);
    assert_eq!(state.list_state.selected, Some(1));
}

#[test]
fn info_pane_does_not_bind_desktop_toggle() {
    let tmp = tempfile::tempdir().unwrap();
    let mut state = SettingsState::new(tmp.path().to_path_buf());
    state.pane = SettingsPane::Info;

    assert!(matches!(
        handle_key(&KeyEvent::from(KeyCode::Char('d')), &mut state, None),
        SettingsAction::None
    ));
}
