// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn narrow_settings_shows_the_selected_pane() {
    let tmp = tempfile::tempdir().unwrap();
    let mut state = SettingsState::new(tmp.path().to_path_buf(), tmp.path().join("instances"));
    let instance: InstanceConfig = serde_json::from_value(serde_json::json!({
        "name": "Test", "game_version": "1.21", "loader": "vanilla",
        "loader_version": null, "created": "2026-01-01T00:00:00Z", "java_path": "test-java"
    }))
    .unwrap();
    let environment = crate::instance::java::merge_environment(
        &SETTINGS.read().defaults.environment,
        &instance.environment,
    );
    state.java_key = Some(java_path_key(
        "test-java",
        &state
            .instances_dir
            .join("Test")
            .join(crate::storage::MINECRAFT_DIR_NAME),
        &environment,
    ));
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
    let mut state = SettingsState::new(tmp.path().to_path_buf(), tmp.path().join("instances"));
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
    let mut state = SettingsState::new(tmp.path().to_path_buf(), tmp.path().join("instances"));
    state.pane = SettingsPane::Info;

    assert!(matches!(
        handle_key(&KeyEvent::from(KeyCode::Char('d')), &mut state, None),
        SettingsAction::None
    ));
}

#[cfg(unix)]
#[test]
fn instance_java_labels_use_their_cwd_and_environment_and_discard_previous_results() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let java = temp.path().join("selected-java");
    std::fs::write(&java, b"#!/bin/sh\nread -r expected < marker || exit 1\n[ \"$RMCL_TEST_JAVA_VERSION\" = \"$expected\" ] || exit 1\nprintf 'java.version = %s\\n' \"$expected\"\n").unwrap();
    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut state = SettingsState::new(temp.path().join("meta"), instances.clone());
    let instance = |name: &str, version: &str| -> InstanceConfig {
        let minecraft = instances.join(name).join("minecraft");
        std::fs::create_dir_all(&minecraft).unwrap();
        std::fs::write(minecraft.join("marker"), format!("{version}\n")).unwrap();
        serde_json::from_value(serde_json::json!({
            "name": name, "game_version": "1.21", "loader": "vanilla", "created": "2026-01-01T00:00:00Z",
            "java_path": java, "environment": {"RMCL_TEST_JAVA_VERSION": version}
        })).unwrap()
    };
    fn wait_for_label(state: &mut SettingsState, instance: &InstanceConfig, expected: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            state.update_for_instance(Some(instance));
            if state.java_label == expected {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("Expected {expected}, found {}", state.java_label);
    }
    let first = instance("first", "21");
    wait_for_label(&mut state, &first, "jdk21");
    let old_pending = state.pending_java.clone();
    let second = instance("second", "25");
    state.update_for_instance(Some(&second));
    *old_pending.lock().unwrap() = Some("stale label".to_owned());
    wait_for_label(&mut state, &second, "jdk25");
}
