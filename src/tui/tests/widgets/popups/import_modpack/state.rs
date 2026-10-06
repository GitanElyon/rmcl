// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::instance::{
    ModLoader,
    import::{ImportSummary, PackFormat},
};
use crate::tests::TEST_LOCK;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn dispatch_repeats_import_version_search_but_not_import_confirmation() {
    use crate::tui::{app::FocusedArea, tests::harness::UiHarness};
    use crossterm::event::KeyEventKind;

    let mut ui = UiHarness::new();
    ui.app.focused = FocusedArea::ImportPopup;
    ui.app.instances_state.show_import_popup = true;
    IMPORT_STATE.lock().unwrap().step = ImportStep::Version;
    IMPORT_STATE.lock().unwrap().version_search.activate();
    assert!(ui.key_event(KeyEvent::new_with_kind(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
        KeyEventKind::Repeat
    )));
    assert_eq!(IMPORT_STATE.lock().unwrap().version_search.query, "i");
    assert!(!ui.key_event(KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Repeat
    )));
    {
        let mut state = IMPORT_STATE.lock().unwrap();
        state.step = ImportStep::Confirm;
        state.summary = Some(summary());
    }
    assert!(!ui.key_event(KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        KeyEventKind::Repeat
    )));
    assert!(take_result().is_none());
    assert!(ui.app.instances_state.show_import_popup);
    ui.key(KeyCode::Enter);
    assert_eq!(take_result().unwrap().summary.name, "Test Pack");
}

fn summary() -> ImportSummary {
    ImportSummary {
        name: "Test Pack".to_owned(),
        pack_version: "1.0.0".to_owned(),
        game_version: "1.21.1".to_owned(),
        loader: ModLoader::Fabric,
        loader_version: Some("0.16.14".to_owned()),
        mod_count: 2,
        override_count: 1,
        format: PackFormat::Mrpack,
        archive_path: PathBuf::from("test.mrpack"),
        source: None,
    }
}

#[test]
fn stale_import_results_cannot_replace_the_current_request() {
    let state = Arc::new(Mutex::new(ImportWizardState {
        step: ImportStep::Fetching,
        request_id: 2,
        ..ImportWizardState::default()
    }));

    assert!(!update_current_request(&state, 1, |state| {
        state.step = ImportStep::Confirm;
    }));
    assert_eq!(state.lock().unwrap().step, ImportStep::Fetching);
    assert!(update_current_request(&state, 2, |state| {
        state.step = ImportStep::Confirm;
    }));
    assert_eq!(state.lock().unwrap().step, ImportStep::Confirm);
}

#[test]
fn empty_input_is_ignored_and_escape_returns_to_discovery() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    *DISCOVERY_STATE.lock().unwrap() = crate::tui::widgets::content::DiscoveryState::new_modpacks();
    let mut instances = instances::State {
        show_import_popup: true,
        ..instances::State::default()
    };
    *IMPORT_STATE.lock().unwrap() = ImportWizardState {
        step: ImportStep::Input,
        ..ImportWizardState::default()
    };

    handle_key(&key(KeyCode::Enter), &mut instances);
    assert_eq!(IMPORT_STATE.lock().unwrap().step, ImportStep::Input);

    handle_key(&key(KeyCode::Esc), &mut instances);
    assert_eq!(IMPORT_STATE.lock().unwrap().step, ImportStep::Discover);
    assert!(instances.show_import_popup);

    handle_key(&key(KeyCode::Esc), &mut instances);
    assert!(!instances.show_import_popup);
}

#[test]
fn confirm_returns_the_import_summary() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut instances = instances::State {
        show_import_popup: true,
        ..instances::State::default()
    };
    {
        let mut state = IMPORT_STATE.lock().unwrap();
        *state = ImportWizardState {
            step: ImportStep::Confirm,
            summary: Some(summary()),
            ..ImportWizardState::default()
        };
    }
    *IMPORT_RESULT.lock().unwrap() = None;

    handle_key(&key(KeyCode::Enter), &mut instances);
    let result = take_result().expect("import result");
    assert_eq!(result.summary.name, "Test Pack");
    assert!(!instances.show_import_popup);
}

#[test]
fn discovered_modpack_version_skips_content_install_confirmation() {
    let project = crate::net::modrinth::DiscoveryProject {
        id: "pack-id".to_owned(),
        slug: "test-pack".to_owned(),
        title: "Test Pack".to_owned(),
        description: "A pack".to_owned(),
        downloads: 1,
        icon_url: None,
        icon_bytes: None,
    };
    let mut discovery = crate::tui::widgets::content::DiscoveryState::new_modpacks();
    discovery.list.entries.push(
        crate::tui::widgets::content::discovery::provider_project_entry(
            project,
            "modrinth",
            "test-pack".to_owned(),
            None,
        ),
    );
    discovery.list.list_state.select(Some(0));
    let versions = discovery.begin_versions().unwrap();
    crate::tui::widgets::content::DiscoveryState::push_action_result(
        &versions.pending,
        crate::tui::widgets::content::discovery::DiscoveryActionResult::Versions {
            request_id: versions.request_id,
            project_id: versions.project_id,
            result: Ok(crate::tui::widgets::content::discovery::ProjectVersions {
                minecraft_versions: vec!["1.21.1".to_owned()],
                versions: vec![VersionInfo {
                    id: "version-id".to_owned(),
                    project_id: "pack-id".to_owned(),
                    name: "1.0".to_owned(),
                    version_number: "1.0".to_owned(),
                    game_versions: vec!["1.21.1".to_owned()],
                    loaders: vec!["fabric".to_owned()],
                    version_type: crate::net::modrinth::VersionType::Release,
                    dependencies: Vec::new(),
                    date_published: String::new(),
                    files: Vec::new(),
                }],
            }),
        },
    );
    discovery.drain_pending();
    assert!(discovery.select_minecraft_version());

    let selected = take_discovered_version(&mut discovery).expect("selected version");

    assert_eq!(selected.version.id, "version-id");
    assert!(discovery.version_popup.is_none());
}

#[test]
fn discovered_modpack_becomes_visible_after_its_icon_is_decoded() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        *DISCOVERY_STATE.lock().unwrap() =
            crate::tui::widgets::content::DiscoveryState::new_modpacks();

        let request = DISCOVERY_STATE.lock().unwrap().begin_modpack_search();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let project = crate::net::modrinth::DiscoveryProject {
            id: "pack-id".to_owned(),
            slug: "test-pack".to_owned(),
            title: "Test Pack".to_owned(),
            description: "A pack".to_owned(),
            downloads: 1,
            icon_url: None,
            icon_bytes: Some(png.into_inner()),
        };
        assert!(request.stream.upsert(
            crate::tui::widgets::content::discovery::provider_project_entry(
                project,
                "modrinth",
                "testpack".to_owned(),
                None,
            )
        ));
        crate::tui::widgets::content::DiscoveryState::push_result(
            &request.pending,
            request.generation,
            request.offset,
            Ok(
                crate::tui::widgets::content::discovery::DiscoveryPageResult {
                    received: 1,
                    total_hits: 1,
                    ..Default::default()
                },
            ),
        );

        let picker = ratatui_image::picker::Picker::halfblocks();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                drain(&picker);
                if !DISCOVERY_STATE
                    .lock()
                    .unwrap()
                    .list
                    .filtered_indices()
                    .is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("modpack icon render completed");
    });
}
