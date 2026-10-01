// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

fn cache_java_result(
    state: &mut SettingsState,
    instance: &InstanceConfig,
    label: Option<&str>,
) -> Arc<Mutex<Option<String>>> {
    let settings = SETTINGS.read();
    let environment = crate::instance::java::merge_environment(
        &settings.defaults.environment,
        &instance.environment,
    );
    let cwd = state
        .instances_dir
        .join(&instance.name)
        .join(crate::storage::MINECRAFT_DIR_NAME);
    let java = crate::instance::java::resolve_java_path_in(
        instance.java_path.as_deref(),
        &cwd,
        &environment,
    );
    let result = Arc::new(Mutex::new(label.map(str::to_owned)));
    state.java_cache.insert(
        instance.name.clone(),
        CachedJavaLabel {
            key: Some((instance.created, java_path_key(&java, &cwd, &environment))),
            result: result.clone(),
        },
    );
    result
}

#[test]
fn narrow_settings_shows_the_selected_pane() {
    let _guard = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let mut state = SettingsState::new(tmp.path().to_path_buf(), tmp.path().join("instances"));
    let instance: InstanceConfig = serde_json::from_value(serde_json::json!({
        "name": "Test", "game_version": "1.21", "loader": "vanilla",
        "loader_version": null, "created": "2026-01-01T00:00:00Z", "java_path": "test-java"
    }))
    .unwrap();
    cache_java_result(&mut state, &instance, Some("jdk17"));
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
fn cached_settings_render_immediately_and_keep_configuration_values_current() {
    let _guard = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let mut state = SettingsState::new(temp.path().join("meta"), temp.path().join("instances"));
    state.profiles = vec!["shared".to_owned()];
    let mut first: InstanceConfig = serde_json::from_value(serde_json::json!({
        "name": "first", "game_version": "1.21", "loader": "vanilla",
        "created": "2026-01-01T00:00:00Z", "java_path": "test-java",
        "memory_max": "4G", "config_sync_profile": "shared"
    }))
    .unwrap();
    let mut second = first.clone();
    second.name = "second".to_owned();
    second.memory_max = Some("8G".to_owned());
    second.config_sync_profile = None;
    cache_java_result(&mut state, &first, Some("jdk21"));
    let pending = cache_java_result(&mut state, &second, None);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 5)).unwrap();
    let mut draw = |state: &mut SettingsState, instance: &InstanceConfig| {
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    FocusedArea::Settings,
                    state,
                    Some(instance),
                )
            })
            .unwrap();
        terminal.backend().to_string()
    };

    assert!(draw(&mut state, &second).contains("unknown"));
    assert!(draw(&mut state, &first).contains("jdk21"));
    *pending.lock().unwrap() = Some("jdk25".to_owned());
    for (instance, java, memory) in [
        (&second, "jdk25", "8G"),
        (&first, "jdk21", "4G"),
        (&second, "jdk25", "8G"),
    ] {
        let screen = draw(&mut state, instance);
        assert!(screen.contains(java), "{screen}");
        assert!(screen.contains(memory), "{screen}");
        assert!(!screen.contains("unknown"), "{screen}");
        assert_eq!(state.active_profile, instance.config_sync_profile);
    }
    first.memory_max = Some("12G".to_owned());
    first.config_sync_profile = None;
    let screen = draw(&mut state, &first);
    assert!(screen.contains("jdk21"));
    assert!(screen.contains("12G"));
    assert_eq!(state.active_profile, None);

    state.invalidate_java_cache(Some(&first.name));
    assert!(!state.java_cache.contains_key(&first.name));
    assert!(draw(&mut state, &second).contains("jdk25"));
    state.invalidate_java_cache(None);
    assert!(state.java_cache.is_empty());
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
fn instance_java_labels_are_cached_and_refresh_when_probe_inputs_change() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let java = temp.path().join("selected-java");
    std::fs::write(&java, b"#!/bin/sh\nprintf 'probe\\n' >> probes\nread -r expected < marker || exit 1\n[ \"$RMCL_TEST_JAVA_VERSION\" = \"$expected\" ] || exit 1\nprintf 'java.version = %s\\n' \"$expected\"\n").unwrap();
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
    let mut first = instance("first", "21");
    wait_for_label(&mut state, &first, "jdk21");
    let second = instance("second", "25");
    state.update_for_instance(Some(&second));
    wait_for_label(&mut state, &second, "jdk25");
    state.update_for_instance(Some(&first));
    assert_eq!(state.java_label, "jdk21");
    state.update_for_instance(None);
    state.update_for_instance(Some(&second));
    assert_eq!(state.java_label, "jdk25");
    let probes = |name: &str| {
        std::fs::read_to_string(instances.join(name).join("minecraft/probes"))
            .unwrap()
            .lines()
            .count()
    };
    assert_eq!(probes("first"), 1);
    assert_eq!(probes("second"), 1);

    let stale = state.java_cache[&first.name].result.clone();
    first = instance("first", "17");
    state.update_for_instance(Some(&first));
    *stale.lock().unwrap() = Some("stale label".to_owned());
    wait_for_label(&mut state, &first, "jdk17");
    assert_eq!(probes("first"), 2);
    state.update_for_instance(Some(&second));
    assert_eq!(state.java_label, "jdk25");

    std::fs::write(
        &java,
        b"#!/bin/sh\nprintf 'probe\\n' >> probes\nprintf 'java.version = 8\\n'\n",
    )
    .unwrap();
    wait_for_label(&mut state, &first, "jdk8");
    assert_eq!(probes("first"), 3);
    first.created += chrono::TimeDelta::seconds(1);
    wait_for_label(&mut state, &first, "jdk8");
    assert_eq!(probes("first"), 4);

    let other_java = temp.path().join("other-java");
    std::fs::write(
        &other_java,
        b"#!/bin/sh\nprintf 'probe\\n' >> probes\nprintf 'java.version = 11\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&other_java, std::fs::Permissions::from_mode(0o755)).unwrap();
    first.java_path = Some(other_java.to_string_lossy().into_owned());
    wait_for_label(&mut state, &first, "jdk11");
    assert_eq!(probes("first"), 5);
}
