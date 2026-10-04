// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use crate::tui::tests::harness::UiHarness;

#[test]
fn instance_switches_keep_scans_empty_lists_selections_and_reconciliation_cached() {
    let mut ui = UiHarness::new();
    ui.add_instance("Cached A");
    ui.add_instance("Cached B");
    ui.app.instances_state.list_state.selected = Some(0);
    ui.app.sync_instance_content();
    let instance = ui.app.instances_state.selected_instance().unwrap().clone();
    let mods = crate::storage::InstancePaths::new(ui.instance_path(&instance.name))
        .minecraft()
        .join("mods");
    let entry =
        |name: &str| crate::instance::scan_one_mod(&mods.join(format!("{name}.jar")), name, true);
    let stream = ui.app.mods_state.start_stream(&instance.name);
    assert!(stream.send(entry("alpha")));
    assert!(stream.send(entry("bravo")));
    ui.app.mods_state.drain_pending();
    ui.app.mods_state.list_state.selected = Some(1);
    drop(ui.app.resource_packs_state.start_stream(&instance.name));
    ui.app.resource_packs_state.drain_pending();
    ui.app.reconciliation_for = Some((instance.name.clone(), instance.created));
    ui.app.content_manifest = Some((
        instance.name.clone(),
        crate::instance::ContentManifest::default(),
    ));
    ui.app.content_update_snapshot = Some((
        instance.name.clone(),
        crate::instance::content::updates::UpdateSnapshot {
            game_version: instance.game_version.clone(),
            loader: instance.loader,
            inventory: Vec::new(),
            checked_at: 123,
            updates: Vec::new(),
            failures: Vec::new(),
        },
    ));
    ui.app.content_update_check_pending = true;

    ui.app.instances_state.list_state.selected = Some(1);
    ui.draw();
    assert!(ui.app.mods_state.entries.is_empty());
    assert!(ui.app.content_manifest.is_none());
    assert!(stream.send(entry("charlie")));
    ui.app.instances_state.list_state.selected = Some(0);
    ui.draw();
    ui.app.ensure_content_reconciliation(false);

    assert_eq!(ui.app.mods_state.entries.len(), 2);
    assert_eq!(ui.app.mods_state.list_state.selected, Some(1));
    assert!(ui.app.mods_state.drain_pending());
    assert_eq!(ui.app.mods_state.entries.len(), 3);
    assert_eq!(
        ui.app.resource_packs_state.loaded_for.as_deref(),
        Some("Cached A")
    );
    assert!(ui.app.resource_packs_state.entries.is_empty());
    assert!(!ui.app.resource_packs_state.is_scanning());
    assert!(!ui.app.resource_packs_state.loading);
    assert_eq!(
        ui.app
            .content_update_snapshot
            .as_ref()
            .unwrap()
            .1
            .checked_at,
        123
    );
    assert!(ui.app.content_update_check_pending);

    ui.app.forget_instance_content("Cached B");
    assert!(stream.send(entry("delta")));
    assert!(ui.app.content_manifest.is_some());
    ui.app.instances_state.list_state.selected = Some(1);
    ui.app.sync_instance_content();
    ui.app.forget_instance_content("Cached A");
    assert!(!stream.send(entry("discarded")));
    ui.app.instances_state.list_state.selected = Some(0);
    ui.app.sync_instance_content();
    assert!(ui.app.mods_state.entries.is_empty());
    assert!(ui.app.mods_state.loaded_for.is_none());
    assert!(ui.app.reconciliation_for.is_none());
}

#[test]
fn content_cache_does_not_survive_a_recreated_instance_or_runtime_change() {
    let mut ui = UiHarness::new();
    ui.add_instance("Changed");
    ui.app.sync_instance_content();
    for recreate in [true, false] {
        let stream = ui.app.mods_state.start_stream("Changed");
        if recreate {
            ui.app.instances_state.instances[0].created += chrono::TimeDelta::seconds(1);
        } else {
            ui.app.instances_state.instances[0].game_version = "1.21.2".to_owned();
        }
        ui.app.sync_instance_content();
        assert!(!stream.send(crate::instance::scan_one_mod(
            std::path::Path::new("stale.jar"),
            "stale",
            true
        )));
        assert!(ui.app.mods_state.loaded_for.is_none());
        assert!(ui.app.cached_instance_content.is_empty());
    }
}

#[test]
fn partial_update_snapshots_label_rows_immediately_and_keep_inactive_results() {
    use crate::instance::content::updates::{
        AvailableUpdate, PENDING_UPDATE_SNAPSHOTS, PendingUpdateSnapshot, UpdateSnapshot,
    };
    let mut ui = UiHarness::new();
    for name in ["Streaming A", "Streaming B"] {
        ui.add_instance(name);
    }
    ui.app.instances_state.list_state.selected = Some(0);
    ui.app.sync_instance_content();
    let installed = crate::instance::ProviderProject {
        provider: "modrinth".to_owned(),
        project_id: "project".to_owned(),
        version_id: "old".to_owned(),
    };
    let mut entry =
        crate::instance::scan_one_mod(std::path::Path::new("example.jar"), "example", true);
    entry.provider_project = Some(installed.clone());
    ui.app.mods_state.set_entries(vec![entry]);
    let snapshot = UpdateSnapshot {
        game_version: "1.21.1".to_owned(), loader: crate::instance::ModLoader::Fabric,
        inventory: vec![installed.clone()], checked_at: 0,
        updates: vec![AvailableUpdate {
            installed, current: None, kind: crate::instance::ContentKind::Mod,
            target: serde_json::from_value(serde_json::json!({
                "id": "new", "name": "New", "version_number": "2", "game_versions": [], "loaders": [], "files": []
            })).unwrap(),
        }], failures: Vec::new(),
    };
    {
        let mut pending = PENDING_UPDATE_SNAPSHOTS.lock().unwrap();
        pending.clear();
        for instance in &ui.app.instances_state.instances {
            pending.push(PendingUpdateSnapshot {
                instance_name: instance.name.clone(),
                instance_created: instance.created,
                snapshot: snapshot.clone(),
            });
        }
    }
    ui.app.drain_content_update_snapshots();
    assert_eq!(
        ui.app.mods_state.entries[0].title_suffix.as_deref(),
        Some("Update")
    );
    assert_eq!(
        ui.app
            .content_update_snapshot
            .as_ref()
            .unwrap()
            .1
            .checked_at,
        0
    );
    assert_eq!(PENDING_UPDATE_SNAPSHOTS.lock().unwrap().len(), 1);
    ui.app.instances_state.list_state.selected = Some(1);
    ui.app.sync_instance_content();
    ui.app.drain_content_update_snapshots();
    assert_eq!(
        ui.app.content_update_snapshot.as_ref().unwrap().0,
        "Streaming B"
    );
    assert!(PENDING_UPDATE_SNAPSHOTS.lock().unwrap().is_empty());
}

#[test]
fn update_badges_stream_while_the_content_index_is_still_being_saved() {
    use crate::instance::content::{manifest, reconcile, updates};
    use std::{collections::HashMap, sync::Arc};

    fn wait_for(mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "content work timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    let mut ui = UiHarness::new();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let _runtime = runtime.enter();
    ui.add_instance("Streaming index");
    ui.app.sync_instance_content();
    let instance = ui.app.instances_state.selected_instance().unwrap().clone();
    let paths = crate::storage::InstancePaths::new(ui.instance_path(&instance.name));
    let minecraft = paths.minecraft();
    let mut saved = crate::instance::ContentManifest::default();
    let mut entries = Vec::new();
    let mut versions = Vec::new();
    let mut pauses = HashMap::new();
    for name in ["alpha", "bravo"] {
        let relative_path = std::path::PathBuf::from("mods").join(format!("{name}.jar"));
        let path = minecraft.join(&relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, name).unwrap();
        saved.files.push(crate::instance::ContentFileRecord {
            relative_path,
            kind: crate::instance::ContentKind::Mod,
            enabled: true,
            fingerprint: manifest::fingerprint(&path).unwrap(),
            resolution: crate::instance::Resolution::Resolved {
                project: crate::instance::ProviderProject {
                    provider: "modrinth".to_owned(),
                    project_id: name.to_owned(),
                    version_id: format!("{name}-old"),
                },
            },
            provider_aliases: Vec::new(),
            provider_checks: vec!["modrinth".to_owned(), "curseforge".to_owned()],
            required_dependencies: Vec::new(),
            automatic_dependency: false,
            cleanup_eligible: false,
        });
        entries.push(crate::instance::scan_one_mod(&path, name, true));
        for (suffix, date) in [
            ("old", "2026-01-01T00:00:00Z"),
            ("new", "2026-02-01T00:00:00Z"),
        ] {
            versions.push(
                serde_json::from_value(serde_json::json!({
                    "id": format!("{name}-{suffix}"), "project_id": name,
                    "name": name, "version_number": suffix,
                    "date_published": date, "game_versions": ["1.21.1"],
                    "loaders": ["fabric"], "files": []
                }))
                .unwrap(),
            );
        }
        pauses.insert(name.to_owned(), Arc::new(tokio::sync::Semaphore::new(0)));
    }
    let mut removed = saved.files[0].clone();
    removed.relative_path = std::path::PathBuf::from("mods").join("removed.jar");
    saved.files.push(removed);
    saved.save(&paths.content_manifest()).unwrap();
    let stream = ui.app.mods_state.start_stream(&instance.name);
    let _shader_stream = ui.app.shaders_state.start_stream(&instance.name);
    updates::PENDING_UPDATE_SNAPSHOTS.lock().unwrap().clear();
    reconcile::PENDING_RECONCILIATIONS.lock().unwrap().clear();
    let instances_dir = ui.app.instance_manager.instances_dir.clone();
    let indexing_instance = instance.clone();
    let (resume_save, saving) = std::sync::mpsc::channel();
    let indexing = runtime.spawn(async move {
        let task = crate::feedback::progress::ProgressTask::start("Preparing content index");
        let result = reconcile::reconcile(
            indexing_instance,
            instances_dir,
            crate::net::HttpClient::new(),
            &task,
            move |result| {
                reconcile::publish_reconciliation(result);
                saving.recv().unwrap();
            },
        )
        .await;
        reconcile::publish_reconciliation(result);
    });
    wait_for(|| {
        !reconcile::PENDING_RECONCILIATIONS
            .lock()
            .unwrap()
            .is_empty()
    });
    assert!(!reconcile::PENDING_RECONCILIATIONS.lock().unwrap()[0].complete);
    ui.app.drain_content_reconciliation();
    assert_eq!(
        ui.app.content_update_check_pending,
        crate::config::SETTINGS.read().general.check_content_updates
    );
    assert!(!ui.app.content_update_check_ready());
    for entry in entries {
        assert!(stream.send(entry));
    }
    ui.app.mods_state.drain_pending();
    ui.app.apply_cached_content_manifest();
    assert!(ui.app.content_update_check_ready());
    assert!(ui.app.mods_state.is_scanning());
    assert!(ui.app.shaders_state.is_scanning());
    let priority = ui.app.content_update_priority();
    let manifest = ui.app.content_manifest.as_ref().unwrap().1.clone();
    ui.app.content_update_check_pending = false;
    let (started, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let registry = Arc::new(
        crate::instance::content::dependencies::tests::controlled_update_registry(
            versions,
            started,
            pauses.clone(),
        ),
    );
    let start_check = |manifest: &crate::instance::ContentManifest, previous| {
        updates::spawn_with_registry(
            instance.clone(),
            manifest.clone(),
            paths.content_updates(),
            priority.clone(),
            previous,
            registry.clone(),
        )
    };
    let mut wait_for_requests = || {
        runtime.block_on(async {
            for _ in 0..2 {
                tokio::time::timeout(Duration::from_secs(5), requests.recv())
                    .await
                    .unwrap()
                    .unwrap();
            }
        })
    };
    assert!(start_check(&manifest, None));
    assert!(!start_check(&manifest, None));
    wait_for_requests();
    pauses["alpha"].add_permits(1);
    wait_for(|| !updates::PENDING_UPDATE_SNAPSHOTS.lock().unwrap().is_empty());
    ui.app.drain_content_update_snapshots();
    assert_eq!(
        ui.app.mods_state.entries[0].title_suffix.as_deref(),
        Some("Update")
    );
    assert_eq!(ui.app.mods_state.entries[1].title_suffix, None);
    assert!(updates::is_running(
        &instance,
        &manifest,
        &paths.content_updates()
    ));
    assert_eq!(
        crate::instance::ContentManifest::load(&paths.content_manifest())
            .unwrap()
            .files
            .len(),
        3
    );
    assert!(
        reconcile::PENDING_RECONCILIATIONS
            .lock()
            .unwrap()
            .is_empty()
    );
    ui.draw();
    assert!(ui.screen().contains("Update"));
    assert_eq!(
        crate::feedback::progress::PROGRESS.lock().unwrap().progress,
        Some((1, 2))
    );

    resume_save.send(()).unwrap();
    runtime.block_on(indexing).unwrap();
    wait_for(|| {
        !reconcile::PENDING_RECONCILIATIONS
            .lock()
            .unwrap()
            .is_empty()
    });
    assert!(reconcile::PENDING_RECONCILIATIONS.lock().unwrap()[0].complete);
    assert_eq!(
        crate::instance::ContentManifest::load(&paths.content_manifest())
            .unwrap()
            .files
            .len(),
        2
    );
    ui.app.drain_content_reconciliation();
    assert!(!ui.app.content_update_check_pending);
    assert_eq!(
        ui.app.mods_state.entries[0].title_suffix.as_deref(),
        Some("Update")
    );
    assert_eq!(ui.app.mods_state.entries[1].title_suffix, None);
    assert_eq!(
        ui.app
            .content_update_snapshot
            .as_ref()
            .unwrap()
            .1
            .checked_at,
        0
    );
    pauses["bravo"].add_permits(1);
    wait_for(|| !updates::is_running(&instance, &manifest, &paths.content_updates()));
    ui.app.content_update_check_pending = true;
    ui.app.drain_content_update_snapshots();
    assert!(!ui.app.content_update_check_pending);
    assert!(
        ui.app
            .mods_state
            .entries
            .iter()
            .all(|entry| entry.title_suffix.as_deref() == Some("Update"))
    );

    ui.app
        .content_update_snapshot
        .as_mut()
        .unwrap()
        .1
        .checked_at = 0;
    reconcile::publish_reconciliation(reconcile::ReconcileResult {
        instance_name: instance.name.clone(),
        instance_created: instance.created,
        manifest: manifest.clone(),
        complete: true,
        error: None,
    });
    ui.app.drain_content_reconciliation();
    assert!(ui.app.content_update_check_pending);
    let previous = ui.app.content_update_snapshot.as_ref().unwrap().1.clone();
    assert!(start_check(&manifest, Some(previous)));
    wait_for_requests();

    let mut changed = manifest.clone();
    changed.files[1].resolution = crate::instance::Resolution::Resolved {
        project: crate::instance::ProviderProject {
            provider: "modrinth".to_owned(),
            project_id: "bravo".to_owned(),
            version_id: "bravo-new".to_owned(),
        },
    };
    changed.save(&paths.content_manifest()).unwrap();
    reconcile::publish_reconciliation(reconcile::ReconcileResult {
        instance_name: instance.name.clone(),
        instance_created: instance.created,
        manifest: changed.clone(),
        complete: true,
        error: None,
    });
    ui.app.drain_content_reconciliation();
    assert!(ui.app.content_update_check_ready());
    let previous = ui.app.content_update_snapshot.as_ref().unwrap().1.clone();
    assert!(start_check(&changed, Some(previous)));
    wait_for_requests();
    pauses["bravo"].add_permits(1);
    wait_for(|| !updates::PENDING_UPDATE_SNAPSHOTS.lock().unwrap().is_empty());
    ui.app.drain_content_update_snapshots();
    assert_eq!(
        ui.app.mods_state.entries[0].title_suffix.as_deref(),
        Some("Update")
    );
    assert_eq!(ui.app.mods_state.entries[1].title_suffix, None);
    assert!(updates::is_running(
        &instance,
        &changed,
        &paths.content_updates()
    ));
    assert_eq!(
        ui.app
            .content_update_snapshot
            .as_ref()
            .unwrap()
            .1
            .checked_at,
        0
    );
    pauses["alpha"].add_permits(1);
    wait_for(|| !updates::is_running(&instance, &changed, &paths.content_updates()));
    ui.app.drain_content_update_snapshots();
    assert!(
        ui.app
            .content_update_snapshot
            .as_ref()
            .unwrap()
            .1
            .matches_manifest(&changed)
    );
    assert!(
        updates::UpdateSnapshot::load(&paths.content_updates())
            .unwrap()
            .matches_manifest(&changed)
    );
    assert!(!ui.app.content_update_check_pending);
    let previous = ui.app.content_update_snapshot.as_ref().unwrap().1.clone();
    assert!(start_check(&changed, Some(previous)));
    wait_for_requests();
    assert!(updates::is_running(
        &instance,
        &changed,
        &paths.content_updates()
    ));
    ui.app.forget_instance_content(&instance.name);
    assert!(!updates::is_running(
        &instance,
        &changed,
        &paths.content_updates()
    ));
    assert!(updates::PENDING_UPDATE_SNAPSHOTS.lock().unwrap().is_empty());
    assert!(requests.try_recv().is_err());
}

fn key_kind(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
    KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
}

#[test]
fn dispatch_repeats_navigation_but_not_launch_delete_or_confirmation() {
    let mut ui = UiHarness::new();
    for name in ["First", "Second", "Third"] {
        ui.add_instance(name);
    }
    ui.app.instances_state.list_state.selected = Some(0);
    ui.draw();

    assert!(ui.key_event(key_kind(KeyCode::Down, KeyEventKind::Press)));
    assert_eq!(ui.app.instances_state.list_state.selected, Some(1));
    assert!(ui.key_event(key_kind(KeyCode::Down, KeyEventKind::Repeat)));
    assert_eq!(ui.app.instances_state.list_state.selected, Some(2));
    assert!(!ui.key_event(key_kind(KeyCode::Up, KeyEventKind::Release)));
    assert_eq!(ui.app.instances_state.list_state.selected, Some(2));

    for code in [
        KeyCode::Char('l'),
        KeyCode::Char('d'),
        KeyCode::Char('u'),
        KeyCode::Enter,
    ] {
        assert!(!ui.key_event(key_kind(code, KeyEventKind::Repeat)));
    }
    assert_eq!(ui.app.focused, FocusedArea::Instances);
    assert!(crate::instance::runtime::get("Third").is_none());
    assert!(widgets::popups::confirm::pending_target().is_none());

    ui.key(KeyCode::Char('d'));
    assert_eq!(ui.app.focused, FocusedArea::ConfirmDelete);
    // A search behind the confirmation cannot turn its 'y' into repeatable text.
    ui.app.instances_state.search.active = true;
    for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
        for code in [KeyCode::Enter, KeyCode::Char('y'), KeyCode::Char('n')] {
            assert!(!ui.key_event(key_kind(code, kind)));
        }
    }
    assert!(ui.instance_path("Third").exists());
    assert_eq!(ui.app.focused, FocusedArea::ConfirmDelete);
    ui.app.instances_state.search.active = false;
    ui.key(KeyCode::Char('y'));
    assert!(!ui.instance_path("Third").exists());
    assert!(!ui.key_event(key_kind(KeyCode::Char('y'), KeyEventKind::Repeat)));
    assert_eq!(ui.app.instances_state.instances.len(), 2);
}

#[test]
fn dispatch_repeats_text_and_backspace_in_search_rename_account_and_profile_inputs() {
    let mut ui = UiHarness::new();
    ui.add_instance("Text Fields");

    ui.app.instances_state.search.activate();
    ui.key_event(key_kind(KeyCode::Char('d'), KeyEventKind::Press));
    ui.key_event(key_kind(KeyCode::Char('d'), KeyEventKind::Repeat));
    assert!(!ui.key_event(key_kind(KeyCode::Char('d'), KeyEventKind::Release)));
    assert_eq!(ui.app.instances_state.search.query, "dd");
    assert!(ui.key_event(key_kind(KeyCode::Backspace, KeyEventKind::Repeat)));
    assert_eq!(ui.app.instances_state.search.query, "d");
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    assert!(ui.app.instances_state.search.active);
    ui.app.instances_state.search.deactivate();

    ui.app.instances_state.renaming = Some(String::new());
    assert!(ui.key_event(key_kind(KeyCode::Char('l'), KeyEventKind::Repeat)));
    assert_eq!(ui.app.instances_state.renaming.as_deref(), Some("l"));
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    assert!(ui.instance_path("Text Fields").exists());
    ui.app.instances_state.renaming = None;

    ui.app.focused = FocusedArea::Account;
    ui.app.account_state.add_mode = widgets::account::AddMode::OfflineNameInput(String::new());
    assert!(ui.key_event(key_kind(KeyCode::Char('d'), KeyEventKind::Repeat)));
    assert!(
        matches!(&ui.app.account_state.add_mode, widgets::account::AddMode::OfflineNameInput(name) if name == "d")
    );
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    assert_eq!(ui.app.focused, FocusedArea::Account);

    ui.app.focused = FocusedArea::Settings;
    ui.app.settings_state.add_mode = widgets::settings::AddMode::ProfileName(String::new());
    assert!(ui.key_event(key_kind(KeyCode::Char('q'), KeyEventKind::Repeat)));
    assert!(
        matches!(&ui.app.settings_state.add_mode, widgets::settings::AddMode::ProfileName(name) if name == "q")
    );
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    assert!(!ui.app.exit);
}

#[test]
fn dispatch_does_not_repeat_content_toggles_or_install_confirmation() {
    let mut ui = UiHarness::new();
    ui.add_instance("One Shot Content");
    let path = ui
        .instance_path("One Shot Content")
        .join(crate::storage::MINECRAFT_DIR_NAME)
        .join("mods/test.jar");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"mod").unwrap();
    ui.app.focused = FocusedArea::Content;
    ui.app
        .mods_state
        .set_entries(vec![crate::instance::content::entry::ContentEntry {
            file_stem: "test".to_owned(),
            name: "Test Mod".to_owned(),
            source_slug: None,
            installed_path: None,
            provider_project: None,
            world_details: None,
            title_suffix: None,
            footer_label: None,
            footer_change: None,
            description: String::new(),
            enabled: true,
            icon_bytes: None,
            provider_icon: false,
            provider_description: false,
            path: path.clone(),
            icon_lines: None,
        }]);
    ui.app.mods_state.list_state.selected = Some(0);
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    assert!(path.exists());
    ui.key(KeyCode::Enter);
    assert!(!path.exists());
    assert!(path.with_extension("jar.disabled").exists());
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    assert!(!path.exists());

    ui.app
        .mods_discovery_state
        .begin_managed_modpack_versions(
            "Install",
            crate::instance::ProviderProject {
                provider: "modrinth".to_owned(),
                project_id: "test".to_owned(),
                version_id: "installed".to_owned(),
            },
        )
        .unwrap();
    let popup = ui.app.mods_discovery_state.version_popup.as_mut().unwrap();
    popup.loading = false;
    popup.confirming = true;
    ui.app.mods_discovery_state.search.activate();
    for code in [KeyCode::Enter, KeyCode::Char('s'), KeyCode::Tab] {
        assert!(!ui.key_event(key_kind(code, KeyEventKind::Repeat)));
    }
    let popup = ui.app.mods_discovery_state.version_popup.as_ref().unwrap();
    assert!(!popup.installing);
    assert!(!popup.skip_dependencies);

    let popup = ui.app.mods_discovery_state.version_popup.as_mut().unwrap();
    popup.confirming = false;
    popup.selecting_world = true;
    popup.worlds.search.activate();
    assert!(ui.key_event(key_kind(KeyCode::Char('d'), KeyEventKind::Repeat)));
    assert_eq!(
        ui.app
            .mods_discovery_state
            .version_popup
            .as_ref()
            .unwrap()
            .worlds
            .search
            .query,
        "d"
    );
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
}

#[test]
fn dispatch_repeats_only_the_visible_content_search() {
    use widgets::content::{ContentMode, ContentTab};

    let mut ui = UiHarness::new();
    ui.app.focused = FocusedArea::Content;
    for tab in [
        ContentTab::Mods,
        ContentTab::ResourcePacks,
        ContentTab::Shaders,
        ContentTab::Worlds,
        ContentTab::Screenshots,
        ContentTab::Logs,
    ] {
        ui.app.content_tab = tab;
        ui.key(KeyCode::Char('/'));
        assert!(
            ui.key_event(key_kind(KeyCode::Char('d'), KeyEventKind::Repeat)),
            "{tab:?}"
        );
        let query = match tab {
            ContentTab::Mods => &ui.app.mods_state.search.query,
            ContentTab::ResourcePacks => &ui.app.resource_packs_state.search.query,
            ContentTab::Shaders => &ui.app.shaders_state.search.query,
            ContentTab::Worlds => &ui.app.worlds_state.search.query,
            ContentTab::Screenshots => &ui.app.screenshots_state.search.query,
            ContentTab::Logs => &ui.app.logs_state.search.query,
            _ => unreachable!(),
        };
        assert_eq!(query, "d");
        assert!(!ui.key_event(key_kind(KeyCode::Tab, KeyEventKind::Repeat)));
        ui.key(KeyCode::Esc);
    }
    ui.app.content_tab = ContentTab::Worlds;
    ui.app.open_world_datapacks = Some(("World".to_owned(), std::path::PathBuf::from("World")));
    ui.app.world_datapacks_state.search.activate();
    assert!(ui.key_event(key_kind(KeyCode::Char('u'), KeyEventKind::Repeat)));
    assert_eq!(ui.app.world_datapacks_state.search.query, "u");
    ui.app.open_world_datapacks = None;

    ui.app.content_tab = ContentTab::Logs;
    ui.app.logs_state.viewer_focused = true;
    ui.app.logs_state.viewer_search.activate();
    assert!(ui.key_event(key_kind(KeyCode::Char('G'), KeyEventKind::Repeat)));
    assert_eq!(ui.app.logs_state.viewer_search.query, "G");

    ui.app.content_tab = ContentTab::Mods;
    ui.app.content_mode = ContentMode::Discover;
    ui.app.mods_discovery_state.search.activate();
    assert!(ui.key_event(key_kind(KeyCode::Char('v'), KeyEventKind::Repeat)));
    assert_eq!(ui.app.mods_discovery_state.search.query, "v");
    assert!(ui.app.mods_discovery_state.version_popup.is_none());
    ui.app.mods_discovery_state.search.deactivate();
    ui.app.mods_discovery_state.sort_panel_open = true;
    ui.app.mods_discovery_state.sort_panel_focused = true;
    ui.app.mods_discovery_state.filter_version_picker_open = true;
    ui.app.mods_discovery_state.filter_version_search.activate();
    assert!(ui.key_event(key_kind(KeyCode::Char('s'), KeyEventKind::Repeat)));
    assert_eq!(ui.app.mods_discovery_state.filter_version_search.query, "s");
    assert!(!ui.app.mods_discovery_state.filter_show_snapshots);
}

#[test]
fn dispatch_repeats_instance_setting_text_but_not_picker_selection_or_toggles() {
    let mut ui = UiHarness::new();
    ui.add_instance("Settings Text");
    ui.key(KeyCode::Char('E'));
    for _ in 0..4 {
        ui.key_event(key_kind(KeyCode::Down, KeyEventKind::Repeat));
    }
    ui.key(KeyCode::Char('c'));
    assert!(
        ui.app
            .instance_settings
            .as_ref()
            .unwrap()
            .text_input_active()
    );
    ui.key(KeyCode::Char('d'));
    assert!(ui.key_event(key_kind(KeyCode::Char('d'), KeyEventKind::Repeat)));
    assert!(!ui.key_event(key_kind(KeyCode::Char('d'), KeyEventKind::Release)));
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    ui.key(KeyCode::Enter);
    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .java_path
            .as_deref(),
        Some("dd")
    );
    assert!(!ui.key_event(key_kind(KeyCode::Right, KeyEventKind::Repeat)));
    assert!(!ui.key_event(key_kind(KeyCode::Char('a'), KeyEventKind::Repeat)));

    ui.key(KeyCode::Esc);
    ui.key(KeyCode::Char('E'));
    ui.key(KeyCode::Down);
    ui.key(KeyCode::Enter);
    let loader = ui.app.instance_settings.as_ref().unwrap().draft.loader;
    assert!(!ui.key_event(key_kind(KeyCode::Right, KeyEventKind::Repeat)));
    assert_eq!(
        ui.app.instance_settings.as_ref().unwrap().draft.loader,
        loader
    );
}

#[test]
fn dispatch_repeats_wizard_name_import_input_and_discovery_search() {
    let mut ui = UiHarness::new();
    ui.key(KeyCode::Char('a'));
    ui.key(KeyCode::Char('q'));
    assert!(ui.key_event(key_kind(KeyCode::Char('q'), KeyEventKind::Repeat)));
    assert!(!ui.key_event(key_kind(KeyCode::Char('q'), KeyEventKind::Release)));
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    ui.draw();
    assert!(ui.screen().contains("qq"));
    assert!(!ui.app.exit);
    ui.key(KeyCode::Esc);

    ui.key(KeyCode::Char('m'));
    ui.key(KeyCode::Char('i'));
    ui.key(KeyCode::Char('q'));
    assert!(ui.key_event(key_kind(KeyCode::Char('q'), KeyEventKind::Repeat)));
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    ui.draw();
    assert!(ui.screen().contains("qq"));
    ui.key(KeyCode::Esc);
    ui.key(KeyCode::Char('/'));
    assert!(ui.key_event(key_kind(KeyCode::Char('v'), KeyEventKind::Repeat)));
    assert!(!ui.key_event(key_kind(KeyCode::Enter, KeyEventKind::Repeat)));
    ui.draw();
    assert!(ui.screen().contains("/ v"));
}

#[test]
fn gui_editor_handoff_process() {}

#[test]
fn unrelated_directory_events_do_not_postpone_an_edited_file_save() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("instance.json");
    std::fs::write(&path, b"old").unwrap();
    let mut watch = EditedConfigWatch::new(path.clone()).unwrap();
    let (sender, events) = std::sync::mpsc::channel();
    watch.events = events;
    watch.pending = Some(std::time::Instant::now() - Duration::from_secs(1));
    std::fs::write(path, b"new").unwrap();
    sender
        .send(Ok(notify::Event::new(notify::EventKind::Modify(
            notify::event::ModifyKind::Any,
        ))
        .add_path(temp.path().join("other.json"))))
        .unwrap();
    assert!(watch.changed());
    std::fs::write(temp.path().join("instance.json"), b"changed again").unwrap();
    sender
        .send(Ok(notify::Event::new(notify::EventKind::Modify(
            notify::event::ModifyKind::Any,
        ))
        .add_path(temp.path().join("INSTANCE.JSON"))))
        .unwrap();
    assert!(!watch.changed());
    assert!(watch.pending.is_some());
    watch.pending = Some(std::time::Instant::now() - Duration::from_secs(1));
    assert!(watch.changed());
}

fn wait_for_edited_config(ui: &mut UiHarness, changed: bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut observed = false;
    loop {
        let reloaded = ui.app.drain_edited_configs();
        observed |= ui
            .app
            .edited_config_watches
            .iter()
            .any(|watch| watch.pending.is_some());
        if changed && reloaded {
            return;
        }
        assert!(!reloaded, "unchanged bytes must not reload");
        if !changed
            && observed
            && ui
                .app
                .edited_config_watches
                .iter()
                .all(|watch| watch.pending.is_none())
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "edited file notification timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn gui_editor_saves_reload_after_handoff_and_atomic_replacement_only_when_bytes_change() {
    let mut ui = UiHarness::new();
    ui.add_instance("GUI Edited");
    let path = ui.instance_path("GUI Edited").join("instance.json");
    let mut config = ui.app.instances_state.selected_instance().unwrap().clone();
    config.memory_max = Some("4G".to_owned());
    ui.app.instance_manager.save(&config).unwrap();
    let original = std::fs::read(&path).unwrap();

    let reaper = ui
        .app
        .launch_gui_editor(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tui::event::tests::gui_editor_handoff_process"]),
            &path,
        )
        .unwrap();
    assert!(!ui.app.drain_edited_configs());
    reaper.join().unwrap();
    assert!(!ui.app.drain_edited_configs());
    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .memory_max,
        None
    );
    ui.app.watch_edited_config(&path).unwrap();
    assert_eq!(ui.app.edited_config_watches.len(), 1);

    std::fs::write(&path, &original).unwrap();
    wait_for_edited_config(&mut ui, false);
    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .memory_max,
        None
    );

    config.memory_max = Some("8G".to_owned());
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    wait_for_edited_config(&mut ui, true);
    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .memory_max
            .as_deref(),
        Some("8G")
    );

    config.memory_max = Some("12G".to_owned());
    crate::storage::write_atomic(&path, &serde_json::to_vec(&config).unwrap()).unwrap();
    wait_for_edited_config(&mut ui, true);
    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .memory_max
            .as_deref(),
        Some("12G")
    );

    config.memory_max = Some("16G".to_owned());
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    wait_for_edited_config(&mut ui, true);
    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .memory_max
            .as_deref(),
        Some("16G")
    );

    std::fs::write(&path, b"invalid instance config").unwrap();
    wait_for_edited_config(&mut ui, true);
    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .memory_max
            .as_deref(),
        Some("16G")
    );
    assert!(
        error_buffer::peek_error()
            .unwrap()
            .message
            .contains("Failed to reload edited instance")
    );
    config.memory_max = Some("20G".to_owned());
    crate::storage::write_atomic(&path, &serde_json::to_vec(&config).unwrap()).unwrap();
    wait_for_edited_config(&mut ui, true);
    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .memory_max
            .as_deref(),
        Some("20G")
    );
}

#[cfg(unix)]
#[test]
fn launcher_config_save_process() {
    use std::os::unix::fs::PermissionsExt;

    if std::env::var_os("RMCL_EDITOR_CONFIG_TEST").is_none() {
        return;
    }
    let mut ui = UiHarness::new();
    let config_path = crate::config::get_config_path().join("config.toml");
    let theme_path = crate::config::get_config_path().join("theme.toml");
    let mut theme = crate::config::theme::ThemeConfig::default();
    std::fs::write(&theme_path, toml::to_string(&theme).unwrap()).unwrap();
    ui.app.watch_edited_config(&config_path).unwrap();
    ui.app.watch_edited_config(&theme_path).unwrap();
    assert!(!ui.app.drain_edited_configs());

    let mut config = crate::config::SETTINGS.read().clone();
    ui.add_instance("Inherited Java");
    std::fs::create_dir_all(ui.instance_path("Inherited Java").join("minecraft")).unwrap();
    let java = ui
        .app
        .instance_manager
        .instances_dir
        .parent()
        .unwrap()
        .join("selected-java");
    std::fs::write(
        &java,
        b"#!/bin/sh\nprintf 'java.version = %s\\n' \"$RMCL_TEST_JAVA_VERSION\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
    config.paths.java_path = Some(java.to_string_lossy().into_owned());
    config
        .defaults
        .environment
        .insert("RMCL_TEST_JAVA_VERSION".to_owned(), "21".to_owned());
    config.defaults.memory_max = "6G".to_owned();
    crate::config::SETTINGS
        .save_launcher_settings(config.clone())
        .unwrap();
    ui.app.settings_state.pane = widgets::settings::SettingsPane::Info;
    fn wait_for_java(ui: &mut UiHarness, java: &str, memory: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            ui.draw();
            let screen = ui.screen();
            if screen.contains(java) {
                assert!(screen.contains(memory), "{screen}");
                return;
            }
            assert!(std::time::Instant::now() < deadline, "{screen}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    wait_for_java(&mut ui, "jdk21", "6G");
    wait_for_edited_config(&mut ui, true);

    config.ui.error_auto_dismiss_ms += 1;
    let timeout = config.ui.error_auto_dismiss_ms;
    config
        .defaults
        .environment
        .insert("RMCL_TEST_JAVA_VERSION".to_owned(), "25".to_owned());
    config.defaults.memory_max = "10G".to_owned();
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    wait_for_edited_config(&mut ui, true);
    assert_eq!(
        crate::config::SETTINGS.read().ui.error_auto_dismiss_ms,
        timeout
    );
    wait_for_java(&mut ui, "jdk25", "10G");

    let other_java = java.with_file_name("other-java");
    std::fs::write(&other_java, b"#!/bin/sh\nprintf 'java.version = 17\\n'\n").unwrap();
    std::fs::set_permissions(&other_java, std::fs::Permissions::from_mode(0o755)).unwrap();
    config.paths.java_path = Some(other_java.to_string_lossy().into_owned());
    crate::config::SETTINGS
        .save_launcher_settings(config)
        .unwrap();
    wait_for_java(&mut ui, "jdk17", "10G");
    wait_for_edited_config(&mut ui, true);

    theme.theme = "dracula".to_owned();
    theme.border_style = crate::config::theme::BorderStyle::Thick;
    crate::storage::write_atomic(&theme_path, toml::to_string(&theme).unwrap().as_bytes()).unwrap();
    wait_for_edited_config(&mut ui, true);
    assert_eq!(
        crate::config::theme::current_theme_config().theme,
        "dracula"
    );
    assert_eq!(
        crate::config::theme::BORDER_STYLE.current(),
        crate::config::theme::BorderStyle::Thick
    );
}

#[cfg(unix)]
#[test]
fn watched_launcher_config_and_theme_saves_update_cached_settings() {
    let ui = UiHarness::new();
    let root = ui.app.instance_manager.instances_dir.parent().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tui::event::tests::launcher_config_save_process",
            "--nocapture",
        ])
        .env("RMCL_EDITOR_CONFIG_TEST", "1")
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn edited_instance_config_reloads_into_the_ui() {
    let mut ui = UiHarness::new();
    ui.add_instance("Edited");
    let mut config = ui.app.instances_state.selected_instance().unwrap().clone();
    config.memory_max = Some("8G".to_owned());
    ui.app.instance_manager.save(&config).unwrap();

    ui.app
        .reload_edited_config(&ui.instance_path("Edited").join("instance.json"));

    assert_eq!(
        ui.app
            .instances_state
            .selected_instance()
            .unwrap()
            .memory_max
            .as_deref(),
        Some("8G")
    );
}

#[test]
fn runtime_update_merges_only_confirmed_fields_into_latest_config() {
    let mut ui = UiHarness::new();
    ui.add_instance("Merge Runtime");
    let previous = ui.app.instances_state.selected_instance().unwrap().clone();
    let mut updated = previous.clone();
    updated.game_version = "1.21.2".to_owned();
    updated.memory_max = Some("8G".to_owned());
    let mut current = previous.clone();
    current.config_sync_profile = Some("shared".to_owned());
    current.preferred_account = Some("newer-account".to_owned());

    let merged = merge_instance_settings(&previous, &updated, current);

    assert_eq!(merged.game_version, "1.21.2");
    assert_eq!(merged.memory_max.as_deref(), Some("8G"));
    assert_eq!(merged.config_sync_profile.as_deref(), Some("shared"));
    assert_eq!(merged.preferred_account.as_deref(), Some("newer-account"));
}

#[test]
fn completed_background_instance_is_drained_into_the_ui() {
    let mut ui = UiHarness::new();
    ui.add_instance("Existing");
    ui.add_instance("Pending");
    let mut config = ui.app.instances_state.instances.pop().unwrap();
    ui.app.mods_state.loaded_for = Some("Pending".to_owned());
    ui.app.reconciliation_for = Some(("Pending".to_owned(), config.created));
    ui.app.content_manifest = Some((
        "Pending".to_owned(),
        crate::instance::ContentManifest::default(),
    ));
    config.created += chrono::TimeDelta::seconds(1);
    ui.app.instances_state.list_state.selected = Some(0);
    PENDING_INSTANCES.lock().unwrap().push(config);

    ui.app.drain_pending_instances();

    assert_eq!(
        ui.app.instances_state.selected_instance().unwrap().name,
        "Pending"
    );
    assert!(PENDING_INSTANCES.lock().unwrap().is_empty());
    assert!(ui.app.reconciliation_for.is_none());
    assert!(ui.app.content_manifest.is_none());
    assert!(ui.app.mods_state.loaded_for.is_none());

    ui.draw();
    assert_eq!(ui.app.mods_state.loaded_for.as_deref(), Some("Pending"));
}

#[test]
fn unrelated_background_result_does_not_complete_runtime_update() {
    let mut ui = UiHarness::new();
    ui.add_instance("Pending Runtime");
    ui.app
        .pending_instance_settings_updates
        .insert("Pending Runtime".to_owned());
    let config = ui.app.instances_state.selected_instance().unwrap().clone();
    PENDING_INSTANCES.lock().unwrap().push(config);

    ui.app.drain_pending_instances();

    assert!(
        ui.app
            .pending_instance_settings_updates
            .contains("Pending Runtime")
    );
}

#[test]
fn pending_runtime_update_blocks_instance_rename() {
    let mut ui = UiHarness::new();
    ui.add_instance("Pending Rename");
    ui.app
        .pending_instance_settings_updates
        .insert("Pending Rename".to_owned());

    ui.key(crossterm::event::KeyCode::Char('r'));

    assert!(ui.app.instances_state.renaming.is_none());
    assert_eq!(
        crate::feedback::errors::peek_error().map(|error| error.message),
        Some(RUNTIME_UPDATE_PENDING_MESSAGE.to_owned())
    );
}

#[test]
fn runtime_settings_update_keeps_the_editor_open_and_handles_results() {
    let mut ui = UiHarness::new();
    ui.add_instance("Runtime Settings");
    ui.key(crossterm::event::KeyCode::Char('E'));
    let state = ui.app.instance_settings.as_mut().unwrap();
    state.draft.game_version = "1.21.2".to_owned();
    state.mark_runtime_update_pending();
    ui.app
        .pending_instance_settings_updates
        .insert("Runtime Settings".to_owned());

    ui.key(crossterm::event::KeyCode::Esc);
    assert!(ui.app.instance_settings.is_none());
    ui.key(crossterm::event::KeyCode::Char('E'));
    assert!(
        ui.app
            .instance_settings
            .as_ref()
            .unwrap()
            .runtime_update_pending_for("Runtime Settings")
    );

    let mut updated = ui.app.instances_state.selected_instance().unwrap().clone();
    updated.game_version = "1.21.2".to_owned();
    ui.app.instance_manager.save(&updated).unwrap();
    COMPLETED_INSTANCE_SETTINGS_UPDATES
        .lock()
        .unwrap()
        .push(updated);
    ui.app.drain_completed_instance_settings_updates();

    let state = ui.app.instance_settings.as_ref().unwrap();
    assert_eq!(state.draft.game_version, "1.21.2");
    assert!(!state.runtime_update_pending_for("Runtime Settings"));
    assert!(ui.app.pending_instance_settings_updates.is_empty());

    let state = ui.app.instance_settings.as_mut().unwrap();
    state.draft.game_version = "1.21.3".to_owned();
    state.mark_runtime_update_pending();
    ui.app
        .pending_instance_settings_updates
        .insert("Runtime Settings".to_owned());
    FAILED_INSTANCE_SETTINGS_UPDATES
        .lock()
        .unwrap()
        .push("Runtime Settings".to_owned());
    ui.app.drain_failed_instance_settings_updates();

    let state = ui.app.instance_settings.as_ref().unwrap();
    assert_eq!(state.draft.game_version, "1.21.2");
    assert!(!state.runtime_update_pending_for("Runtime Settings"));
    assert!(ui.app.pending_instance_settings_updates.is_empty());
}

#[test]
fn structural_settings_update_repairs_runtime_before_persisting() {
    use sha1::{Digest, Sha1};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sha1(bytes: &[u8]) -> String {
        format!("{:x}", Sha1::digest(bytes))
    }

    let _guard = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let server = MockServer::start().await;
        let client_jar = b"client";
        let library_jar = b"library";
        for (endpoint, body) in [
            ("/client.jar", client_jar.as_slice()),
            ("/library.jar", library_jar.as_slice()),
        ] {
            Mock::given(method("GET"))
                .and(path(endpoint))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
                .expect(1)
                .mount(&server)
                .await;
        }
        let asset_index = serde_json::to_vec(&serde_json::json!({"objects": {}})).unwrap();
        Mock::given(method("GET"))
            .and(path("/assets.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(asset_index.clone()))
            .expect(1)
            .mount(&server)
            .await;

        let temp = tempfile::tempdir().unwrap();
        let instances_dir = temp.path().join("instances");
        let meta_dir = temp.path().join("meta");
        let instance_dir = instances_dir.join("Migrating");
        std::fs::create_dir_all(&instance_dir).unwrap();
        let manager = InstanceManager::new(&instances_dir, &meta_dir);
        let mut previous = crate::instance::InstanceConfig {
            name: "Migrating".to_owned(),
            game_version: "1.20.1".to_owned(),
            loader: crate::instance::ModLoader::Vanilla,
            loader_version: None,
            created: chrono::Utc::now(),
            last_played: None,
            java_path: None,
            memory_max: None,
            memory_min: None,
            jvm_args: Vec::new(),
            environment: Default::default(),
            window_mode: Default::default(),
            inherit_window_mode: false,
            resolution: None,
            inherit_resolution: false,
            preferred_account: None,
            pre_launch_command: Default::default(),
            post_exit_command: Default::default(),
            glfw_path: None,
            config_sync_profile: None,
            modpack_source: None,
        };
        manager.save(&previous).unwrap();

        let metadata = crate::storage::MetadataPaths::new(&meta_dir);
        let version_dir = metadata.versions().join("1.21.2");
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::write(
            version_dir.join("meta.json"),
            serde_json::to_vec(&serde_json::json!({
                "id": "1.21.2",
                "mainClass": "net.minecraft.client.main.Main",
                "arguments": { "game": [], "jvm": [] },
                "assetIndex": {
                    "id": "1.21.2",
                    "url": format!("{}/assets.json", server.uri()),
                    "sha1": sha1(&asset_index)
                },
                "downloads": {
                    "client": {
                        "url": format!("{}/client.jar", server.uri()),
                        "sha1": sha1(client_jar),
                        "size": client_jar.len()
                    }
                },
                "libraries": [{
                    "name": "example:test:1",
                    "downloads": {
                        "artifact": {
                            "url": format!("{}/library.jar", server.uri()),
                            "path": "example/test/1/test-1.jar",
                            "sha1": sha1(library_jar),
                            "size": library_jar.len()
                        }
                    }
                }],
                "javaVersion": { "majorVersion": 21 }
            }))
            .unwrap(),
        )
        .unwrap();

        let mut updated = previous.clone();
        updated.game_version = "1.21.2".to_owned();
        updated.memory_max = Some("4G".to_owned());
        let applied = apply_instance_settings_update(&manager, &previous, updated)
            .await
            .unwrap();

        assert_eq!(applied.game_version, "1.21.2");
        previous = manager.load_one("Migrating").unwrap();
        assert_eq!(previous.game_version, "1.21.2");
        assert_eq!(previous.memory_max.as_deref(), Some("4G"));
        assert_eq!(
            std::fs::read(version_dir.join("1.21.2.jar")).unwrap(),
            client_jar
        );
        assert_eq!(
            std::fs::read(metadata.libraries().join("example/test/1/test-1.jar")).unwrap(),
            library_jar
        );
        assert!(metadata.assets().join("indexes/1.21.2.json").exists());
        crate::feedback::progress::clear();
    });
}

#[test]
fn failed_structural_settings_update_keeps_previous_config() {
    let _guard = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let instances_dir = temp.path().join("instances");
        let meta_dir = temp.path().join("meta");
        std::fs::create_dir_all(instances_dir.join("Stable")).unwrap();
        let manager = InstanceManager::new(&instances_dir, &meta_dir);
        let previous = crate::instance::InstanceConfig {
            name: "Stable".to_owned(),
            game_version: "1.20.1".to_owned(),
            loader: crate::instance::ModLoader::Vanilla,
            loader_version: None,
            created: chrono::Utc::now(),
            last_played: None,
            java_path: None,
            memory_max: None,
            memory_min: None,
            jvm_args: Vec::new(),
            environment: Default::default(),
            window_mode: Default::default(),
            inherit_window_mode: false,
            resolution: None,
            inherit_resolution: false,
            preferred_account: None,
            pre_launch_command: Default::default(),
            post_exit_command: Default::default(),
            glfw_path: None,
            config_sync_profile: None,
            modpack_source: None,
        };
        manager.save(&previous).unwrap();

        let metadata = crate::storage::MetadataPaths::new(&meta_dir);
        let version_dir = metadata.versions().join("missing-runtime");
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::write(version_dir.join("meta.json"), b"not json").unwrap();
        let mut updated = previous.clone();
        updated.game_version = "missing-runtime".to_owned();

        assert!(
            apply_instance_settings_update(&manager, &previous, updated)
                .await
                .is_err()
        );
        assert_eq!(manager.load_one("Stable").unwrap().game_version, "1.20.1");
        crate::feedback::progress::clear();
    });
}

#[test]
fn editor_kind_is_detected_from_the_executable_name() {
    assert!(editor_runs_in_terminal("/usr/bin/nvim"));
    assert!(editor_runs_in_terminal("nano"));
    assert!(!editor_runs_in_terminal("/usr/bin/code"));
    assert_eq!(
        editor_parts("nvim -u NONE").unwrap(),
        ["nvim", "-u", "NONE"]
    );
}

#[test]
fn editor_commands_preserve_quoted_paths_arguments_and_empty_arguments() {
    assert_eq!(
        editor_parts(r#""/path with spaces/editor" --wait "two words" """#).unwrap(),
        ["/path with spaces/editor", "--wait", "two words", ""]
    );
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("editor with spaces");
    std::fs::write(&executable, b"executable").unwrap();
    assert_eq!(
        editor_parts(executable.to_str().unwrap()).unwrap(),
        [executable.to_string_lossy().into_owned()]
    );
    #[cfg(unix)]
    assert!(editor_parts("editor 'unterminated").is_err());
    #[cfg(windows)]
    assert_eq!(
        editor_parts(r#""C:\Program Files\editor.exe" --wait "C:\folder with spaces\file.txt""#)
            .unwrap(),
        [
            r"C:\Program Files\editor.exe",
            "--wait",
            r"C:\folder with spaces\file.txt"
        ]
    );
}

#[test]
fn overlay_count_tracks_independent_popup_layers() {
    let mut ui = UiHarness::new();
    assert_eq!(ui.app.overlay_count(), 0);

    ui.app.instances_state.show_import_popup = true;
    ui.app.account_state.add_mode = widgets::account::AddMode::ChooseType;
    assert_eq!(ui.app.overlay_count(), 2);

    ui.app.account_state.add_mode = widgets::account::AddMode::None;
    assert_eq!(ui.app.overlay_count(), 1);
}

#[test]
fn terminal_image_markers_exclude_normal_text_and_toggle() {
    use std::num::NonZeroU16;

    let mut buffer = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(4, 7, 3, 1));
    buffer[(4, 7)]
        .set_symbol("\x1b_Gimage\x1b\\")
        .set_diff_option(ratatui::buffer::CellDiffOption::ForcedWidth(
            NonZeroU16::new(1).unwrap(),
        ));
    buffer[(5, 7)].set_symbol("text").set_diff_option(
        ratatui::buffer::CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap()),
    );
    buffer[(6, 7)].set_symbol("\x1b[0m");

    mark_terminal_images(&mut buffer, false);
    assert_eq!(buffer[(4, 7)].symbol(), "\x1b_Gimage\x1b\\\u{200b}");
    assert_eq!(buffer[(5, 7)].symbol(), "text");
    assert_eq!(buffer[(6, 7)].symbol(), "\x1b[0m");

    buffer[(4, 7)].set_symbol("\x1b_Gimage\x1b\\");
    mark_terminal_images(&mut buffer, true);
    assert_eq!(buffer[(4, 7)].symbol(), "\x1b_Gimage\x1b\\\u{200b}\u{200b}");
}

#[test]
fn terminal_image_cells_change_when_an_overlay_opens_or_closes() {
    assert!(terminal_image_cells_changed(
        &[true, false, false],
        &[true, true, false]
    ));
    assert!(terminal_image_cells_changed(
        &[true, true, false],
        &[true, false, false]
    ));
    assert!(!terminal_image_cells_changed(&[], &[true]));
    assert!(!terminal_image_cells_changed(
        &[true, false, true],
        &[true, false, true],
    ));
}
