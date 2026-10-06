// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn migrated_layout_detects_a_newly_copied_legacy_instance() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    fs::create_dir_all(&instances).unwrap();
    initialize_new_layout(&meta).unwrap();
    assert!(!is_needed(&instances, &meta));

    fs::create_dir_all(instances.join("copied/.minecraft/saves")).unwrap();
    assert!(is_needed(&instances, &meta));
    fs::write(temp.path().join("config.toml"), b"[paths]").unwrap();
    fs::write(
        MetadataPaths::new(&meta).cache_rebuild_pending(),
        LAYOUT_VERSION.to_string(),
    )
    .unwrap();
    run(&instances, &meta, &temp.path().join("config.toml"), |_| {}).unwrap();
    assert!(instances.join("copied/minecraft/saves").exists());
}

#[test]
fn migrated_layout_detects_newly_copied_shared_data() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let config = temp.path().join("config.toml");
    fs::create_dir_all(&instances).unwrap();
    fs::write(&config, b"[paths]").unwrap();
    initialize_new_layout(&meta).unwrap();
    fs::write(
        MetadataPaths::new(&meta).cache_rebuild_pending(),
        LAYOUT_VERSION.to_string(),
    )
    .unwrap();
    fs::create_dir_all(meta.join("versions")).unwrap();
    fs::write(meta.join("versions/copied.json"), b"data").unwrap();

    assert!(is_needed(&instances, &meta));
    run(&instances, &meta, &config, |_| {}).unwrap();
    assert_eq!(
        fs::read(MetadataPaths::new(&meta).versions().join("copied.json")).unwrap(),
        b"data"
    );
}

#[test]
fn migration_merges_identical_files_and_preserves_unique_files() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let config = temp.path().join("config.toml");
    fs::write(&config, b"[paths]").unwrap();
    initialize_new_layout(&meta).unwrap();
    let metadata = MetadataPaths::new(&meta);
    let directories = [
        (meta.join("versions/26.2"), metadata.versions().join("26.2")),
        (meta.join("libraries"), metadata.libraries()),
        (meta.join("assets"), metadata.assets()),
        (meta.join("loader-profiles"), metadata.loader_profiles()),
        (meta.join("config-sync/profiles"), metadata.profiles()),
    ];
    let bytes = vec![42; 20_000];
    for (source, destination) in &directories {
        fs::create_dir_all(source).unwrap();
        fs::create_dir_all(destination).unwrap();
        fs::write(source.join("duplicate"), &bytes).unwrap();
        fs::write(destination.join("duplicate"), &bytes).unwrap();
        fs::write(source.join("legacy-only"), b"legacy").unwrap();
        fs::write(destination.join("current-only"), b"current").unwrap();
    }

    run(&instances, &meta, &config, |_| {}).unwrap();

    for (source, destination) in &directories {
        assert!(!source.exists());
        assert_eq!(fs::read(destination.join("duplicate")).unwrap(), bytes);
        assert_eq!(
            fs::read(destination.join("legacy-only")).unwrap(),
            b"legacy"
        );
        assert_eq!(
            fs::read(destination.join("current-only")).unwrap(),
            b"current"
        );
    }
    assert!(!metadata.migration_journal().exists());
    finish_cache_rebuild(&meta).unwrap();
    assert!(!is_needed(&instances, &meta));
}

#[test]
fn migration_archives_different_cache_files_without_overwriting_current_files() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let config = temp.path().join("config.toml");
    let source = meta.join("versions/26.2");
    let destination = MetadataPaths::new(&meta).versions().join("26.2");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("26.2.jar"), b"same").unwrap();
    fs::write(destination.join("26.2.jar"), b"same").unwrap();
    fs::write(source.join("26.2.json"), b"legacy").unwrap();
    fs::write(destination.join("26.2.json"), b"actual").unwrap();
    for (key, legacy, current) in shared_moves(&meta) {
        if matches!(key, "profiles" | "versions") {
            continue;
        }
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(legacy.join("conflict"), b"legacy").unwrap();
        fs::write(current.join("conflict"), b"actual").unwrap();
    }

    let backup = run(&instances, &meta, &config, |_| {}).unwrap();

    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("26.2.jar")).unwrap(), b"same");
    assert_eq!(
        fs::read(backup.join("cache-conflicts/versions/26.2/26.2.json")).unwrap(),
        b"legacy"
    );
    assert_eq!(fs::read(destination.join("26.2.json")).unwrap(), b"actual");
    for (key, legacy, current) in shared_moves(&meta) {
        if matches!(key, "profiles" | "versions") {
            continue;
        }
        assert!(!legacy.exists());
        assert_eq!(fs::read(current.join("conflict")).unwrap(), b"actual");
        assert_eq!(
            fs::read(backup.join("cache-conflicts").join(key).join("conflict")).unwrap(),
            b"legacy"
        );
    }
}

#[test]
fn local_config_conflicts_are_detected_before_legacy_directories_are_renamed() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let config = temp.path().join("config.toml");
    let state = instances.join("Copied/.rmcl");
    fs::create_dir_all(state.join("config-sync/local-config")).unwrap();
    fs::create_dir_all(state.join("content/config")).unwrap();
    fs::write(state.join("config-sync/local-config/options.txt"), b"old").unwrap();
    fs::write(state.join("content/config/options.txt"), b"current").unwrap();
    initialize_new_layout(&meta).unwrap();

    assert!(run(&instances, &meta, &config, |_| {}).is_err());
    assert!(state.exists());
    assert!(MetadataPaths::new(&meta).migration_journal().exists());
    assert!(run(&instances, &meta, &config, |_| {}).is_err());
    assert_eq!(
        fs::read(state.join("content/config/options.txt")).unwrap(),
        b"current"
    );
}

#[test]
fn repeated_migrations_create_separate_backups() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let config = temp.path().join("config.toml");
    fs::create_dir_all(instances.join("First/.minecraft")).unwrap();
    let first = run(&instances, &meta, &config, |_| {}).unwrap();
    fs::create_dir_all(instances.join("Second/.minecraft")).unwrap();
    let second = run(&instances, &meta, &config, |_| {}).unwrap();

    assert_ne!(first, second);
    assert!(second.join("instances/Second/.minecraft").is_dir());
}

#[test]
fn migration_backs_up_and_renames_instance_directories() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let config = temp.path().join("config.toml");
    let instance = instances.join("Example");
    fs::create_dir_all(instance.join(".minecraft/saves/world")).unwrap();
    fs::create_dir_all(instance.join(".rmcl/config-sync/local-config/config")).unwrap();
    fs::write(instance.join(".minecraft/saves/world/level.dat"), b"world").unwrap();
    fs::write(
        instance.join(".rmcl/config-sync/local-config/options.txt"),
        b"options",
    )
    .unwrap();
    fs::write(&config, b"[paths]").unwrap();
    fs::create_dir_all(meta.join("config-sync/profiles/main")).unwrap();
    fs::write(
        meta.join("config-sync/profiles/main/options.txt"),
        b"profile",
    )
    .unwrap();

    let mut progress = Vec::new();
    let backup = run(&instances, &meta, &config, |update| progress.push(update)).unwrap();

    assert_eq!(
        fs::read(instance.join("minecraft/saves/world/level.dat")).unwrap(),
        b"world"
    );
    assert_eq!(
        fs::read(instance.join("rmcl/content/config/options.txt")).unwrap(),
        b"options"
    );
    assert!(backup.join("instances/Example/.minecraft").exists());
    assert_eq!(
        fs::read_to_string(backup.join("config/config.toml")).unwrap(),
        "[paths]"
    );
    let upgraded_config = fs::read_to_string(&config).unwrap();
    assert!(upgraded_config.contains("check_modpack_updates = true"));
    assert!(upgraded_config.contains("resolution = [854, 480]"));
    assert!(backup.join("profiles/legacy/main/options.txt").exists());
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("backup-")
    );
    assert!(!meta.join("config-sync").exists());
    assert!(progress.iter().any(|update| {
        update.item_total.is_some_and(|total| total > 0)
            && update.item_current.is_some_and(|current| current > 0)
    }));
    assert_eq!(
        marker_version(&MetadataPaths::new(&meta).layout_marker()),
        Some(LAYOUT_VERSION)
    );
    assert!(cache_rebuild_pending(&meta));
    assert_eq!(run(&instances, &meta, &config, |_| {}).unwrap(), backup);
    finish_cache_rebuild(&meta).unwrap();
    assert!(!cache_rebuild_pending(&meta));
}

#[test]
fn migration_resumes_from_the_recorded_journal() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let config = temp.path().join("config.toml");
    let done = instances.join("Done");
    let pending = instances.join("Pending");
    fs::create_dir_all(done.join("minecraft")).unwrap();
    fs::create_dir_all(pending.join(".minecraft/saves")).unwrap();

    let metadata = MetadataPaths::new(&meta);
    fs::create_dir_all(metadata.state()).unwrap();
    fs::create_dir_all(metadata.backups()).unwrap();
    let backup = metadata.backups().join("backup-resume");
    fs::create_dir_all(&backup).unwrap();
    write_json_atomic(
        &metadata.migration_journal(),
        &MigrationJournal {
            version: LAYOUT_VERSION,
            backup_dir: backup.clone(),
            completed: vec!["backup".to_owned(), "instance:Done".to_owned()],
        },
    )
    .unwrap();

    assert_eq!(run(&instances, &meta, &config, |_| {}).unwrap(), backup);
    assert!(done.join("minecraft").exists());
    assert!(pending.join("minecraft/saves").exists());
    assert!(!metadata.migration_journal().exists());
}

#[test]
fn stale_partial_backup_is_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let backup = temp.path().join("backups/backup-test");
    let partial = backup.with_extension("partial");
    fs::create_dir_all(instances.join("Example")).unwrap();
    fs::write(instances.join("Example/data.txt"), b"current").unwrap();
    fs::create_dir_all(&partial).unwrap();
    fs::write(partial.join("stale.txt"), b"stale").unwrap();

    backup_user_data(
        &instances,
        &temp.path().join("missing-config.toml"),
        &temp.path().join("meta"),
        &backup,
        |_, _, _| {},
    )
    .unwrap();

    assert_eq!(
        fs::read(backup.join("instances/Example/data.txt")).unwrap(),
        b"current"
    );
    assert!(!backup.join("stale.txt").exists());
    assert!(!partial.exists());
}

#[test]
fn backup_rejects_overlap_and_insufficient_space() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    fs::create_dir_all(&instances).unwrap();

    let overlap =
        validate_backup_destination(&instances, &instances.join("backup"), 0).unwrap_err();
    assert!(matches!(overlap, MigrationError::BackupOverlap(_)));

    let outside = temp.path().join("backups/backup");
    let insufficient = validate_backup_destination(&instances, &outside, u64::MAX).unwrap_err();
    assert!(matches!(
        insufficient,
        MigrationError::InsufficientSpace { .. }
    ));
}

#[test]
fn migration_need_follows_legacy_data_marker_and_pending_rebuild() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    fs::create_dir_all(instances.join("Example/.minecraft")).unwrap();

    assert!(is_needed(&instances, &meta));
    initialize_new_layout(&meta).unwrap();
    assert!(is_needed(&instances, &meta));
    fs::remove_dir_all(instances.join("Example/.minecraft")).unwrap();
    assert!(!is_needed(&instances, &meta));
    fs::create_dir_all(meta.join("versions")).unwrap();
    fs::write(MetadataPaths::new(&meta).layout_marker(), b"invalid marker").unwrap();
    assert!(is_needed(&instances, &meta));
    initialize_new_layout(&meta).unwrap();
    fs::write(
        MetadataPaths::new(&meta).cache_rebuild_pending(),
        LAYOUT_VERSION.to_string(),
    )
    .unwrap();
    assert!(is_needed(&instances, &meta));
}

#[cfg(unix)]
#[test]
fn backup_preserves_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("target.txt"), b"target").unwrap();
    std::os::unix::fs::symlink("target.txt", source.join("link.txt")).unwrap();

    copy_dir_recursive_with_progress(&source, &destination, &mut |_, _| {}).unwrap();

    assert_eq!(
        fs::read_link(destination.join("link.txt")).unwrap(),
        PathBuf::from("target.txt")
    );
}

#[test]
fn conflicting_visible_and_hidden_directories_are_not_merged() {
    let temp = tempfile::tempdir().unwrap();
    let instance = temp.path().join("Example");
    fs::create_dir_all(instance.join(".minecraft")).unwrap();
    fs::create_dir_all(instance.join("minecraft")).unwrap();
    fs::write(instance.join(".minecraft/level.dat"), b"old world").unwrap();
    fs::write(instance.join("minecraft/level.dat"), b"new world").unwrap();
    let error = migrate_instance(&instance, "Example").unwrap_err();
    assert!(matches!(error, MigrationError::MergeConflict { .. }));
    assert_eq!(
        fs::read(instance.join(".minecraft/level.dat")).unwrap(),
        b"old world"
    );
    assert_eq!(
        fs::read(instance.join("minecraft/level.dat")).unwrap(),
        b"new world"
    );
}

#[test]
fn merge_conflicts_do_not_overwrite_existing_profiles() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("legacy");
    let destination = temp.path().join("current");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir_all(destination.join("nested")).unwrap();
    fs::write(source.join("nested/options.txt"), b"legacy").unwrap();
    fs::write(destination.join("nested/options.txt"), b"current").unwrap();

    let error = move_or_merge(&source, &destination, None).unwrap_err();

    assert!(matches!(error, MigrationError::MergeConflict { .. }));
    assert_eq!(
        fs::read(destination.join("nested/options.txt")).unwrap(),
        b"current"
    );
    assert_eq!(
        fs::read(source.join("nested/options.txt")).unwrap(),
        b"legacy"
    );
}

#[test]
fn file_comparison_checks_lengths_empty_files_and_differences_after_the_first_chunk() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let mut bytes = vec![42; 20_000];
    fs::write(&source, &bytes).unwrap();
    fs::write(&destination, &bytes).unwrap();
    assert!(files_identical(&source, &destination).unwrap());
    bytes[19_999] = 43;
    fs::write(&destination, &bytes).unwrap();
    assert!(!files_identical(&source, &destination).unwrap());
    fs::write(&destination, &bytes[..19_999]).unwrap();
    assert!(!files_identical(&source, &destination).unwrap());
    fs::write(&source, []).unwrap();
    fs::write(&destination, []).unwrap();
    assert!(files_identical(&source, &destination).unwrap());
    assert!(!files_identical(&source, &source).unwrap());
    assert!(!files_identical(&source, temp.path()).unwrap());
}

#[cfg(unix)]
#[test]
fn identical_targets_do_not_allow_removing_symlinks_or_aliased_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("file"), b"same").unwrap();
    std::os::unix::fs::symlink(source.join("file"), destination.join("file")).unwrap();

    assert!(matches!(
        validate_merge(&source, &destination, true),
        Err(MigrationError::MergeConflict { .. })
    ));
    assert!(matches!(
        move_or_merge(&source, &destination, Some(&temp.path().join("archives"))),
        Err(MigrationError::MergeConflict { .. })
    ));
    assert!(!temp.path().join("archives").exists());
    assert!(matches!(
        validate_merge(&source, &destination, false),
        Err(MigrationError::MergeConflict { .. })
    ));
    assert!(matches!(
        move_or_merge(&source, &destination, None),
        Err(MigrationError::MergeConflict { .. })
    ));
    assert_eq!(fs::read(source.join("file")).unwrap(), b"same");
    assert!(
        fs::symlink_metadata(destination.join("file"))
            .unwrap()
            .is_symlink()
    );

    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    assert!(matches!(
        move_or_merge(&alias, &source, None),
        Err(MigrationError::MergeConflict { .. })
    ));
    assert_eq!(fs::read(source.join("file")).unwrap(), b"same");
}

#[test]
fn split_instance_directories_merge_without_losing_worlds_or_local_settings() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let instance = instances.join("Example");
    let legacy = instance.join(".minecraft/saves/old-world");
    let current = instance.join("minecraft/saves/current-world");
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&current).unwrap();
    fs::write(legacy.join("level.dat"), b"old world").unwrap();
    fs::write(current.join("level.dat"), b"current world").unwrap();
    fs::create_dir_all(instance.join(".rmcl/config-sync/local-config")).unwrap();
    fs::create_dir_all(instance.join("rmcl/content/config")).unwrap();
    fs::write(
        instance.join(".rmcl/config-sync/local-config/options.txt"),
        b"options",
    )
    .unwrap();
    fs::write(instance.join("rmcl/content/config/options.txt"), b"options").unwrap();

    let backup = run(&instances, &meta, &temp.path().join("config.toml"), |_| {}).unwrap();

    assert!(!instance.join(".minecraft").exists());
    assert!(!instance.join(".rmcl").exists());
    assert_eq!(
        fs::read(instance.join("minecraft/saves/old-world/level.dat")).unwrap(),
        b"old world"
    );
    assert_eq!(
        fs::read(current.join("level.dat")).unwrap(),
        b"current world"
    );
    assert_eq!(
        fs::read(instance.join("rmcl/content/config/options.txt")).unwrap(),
        b"options"
    );
    assert!(
        backup
            .join("instances/Example/.minecraft/saves/old-world/level.dat")
            .exists()
    );
    assert!(
        backup
            .join("instances/Example/minecraft/saves/current-world/level.dat")
            .exists()
    );
}

#[test]
fn conflicting_user_data_prevents_cache_archiving_or_instance_moves() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let instance = instances.join("Example");
    fs::create_dir_all(instance.join(".rmcl/config-sync/local-config")).unwrap();
    fs::create_dir_all(instance.join("rmcl/content/config")).unwrap();
    fs::write(
        instance.join(".rmcl/config-sync/local-config/options.txt"),
        b"old settings",
    )
    .unwrap();
    fs::write(
        instance.join("rmcl/content/config/options.txt"),
        b"current settings",
    )
    .unwrap();
    fs::create_dir_all(meta.join("versions")).unwrap();
    let current = MetadataPaths::new(&meta).versions();
    fs::create_dir_all(&current).unwrap();
    fs::write(meta.join("versions/meta.json"), b"old cache").unwrap();
    fs::write(current.join("meta.json"), b"new cache").unwrap();

    assert!(matches!(
        run(&instances, &meta, &temp.path().join("config.toml"), |_| {}),
        Err(MigrationError::MergeConflict { .. })
    ));
    assert_eq!(
        fs::read(instance.join(".rmcl/config-sync/local-config/options.txt")).unwrap(),
        b"old settings"
    );
    assert_eq!(
        fs::read(instance.join("rmcl/content/config/options.txt")).unwrap(),
        b"current settings"
    );
    assert_eq!(
        fs::read(meta.join("versions/meta.json")).unwrap(),
        b"old cache"
    );
    assert_eq!(fs::read(current.join("meta.json")).unwrap(), b"new cache");
}

#[test]
fn recorded_journals_resume_without_pending_markers_or_skipping_reintroduced_cache_data() {
    for journal_name in ["migration.json", "migration-v2.json"] {
        let temp = tempfile::tempdir().unwrap();
        let instances = temp.path().join("instances");
        let meta = temp.path().join("meta");
        let config = temp.path().join("config.toml");
        fs::write(&config, b"[paths]").unwrap();
        initialize_new_layout(&meta).unwrap();
        let metadata = MetadataPaths::new(&meta);
        let backup = metadata.backups().join("backup-resume");
        fs::create_dir_all(&backup).unwrap();
        write_json_atomic(
            &metadata.state().join(journal_name),
            &MigrationJournal {
                version: LAYOUT_VERSION,
                backup_dir: backup.clone(),
                completed: vec![
                    "backup".to_owned(),
                    "config".to_owned(),
                    "shared:versions".to_owned(),
                ],
            },
        )
        .unwrap();
        if journal_name == "migration.json" {
            fs::copy(
                metadata.migration_journal(),
                metadata.state().join("migration-v2.json"),
            )
            .unwrap();
        }
        assert!(is_needed(&instances, &meta));
        fs::create_dir_all(meta.join("versions")).unwrap();
        fs::write(meta.join("versions/meta.json"), b"old cache").unwrap();
        fs::write(metadata.versions().join("meta.json"), b"new cache").unwrap();
        let archived = backup.join("cache-conflicts/versions/meta.json");
        fs::create_dir_all(archived.parent().unwrap()).unwrap();
        fs::write(&archived, b"earlier conflict").unwrap();

        assert_eq!(run(&instances, &meta, &config, |_| {}).unwrap(), backup);
        assert!(!meta.join("versions").exists());
        assert!(!metadata.migration_journal().exists());
        assert!(!metadata.state().join("migration-v2.json").exists());
        assert_eq!(
            fs::read(backup.join("cache-conflicts/versions/meta.json.1")).unwrap(),
            b"old cache"
        );
        assert_eq!(fs::read(&archived).unwrap(), b"earlier conflict");
        assert_eq!(
            fs::read(metadata.versions().join("meta.json")).unwrap(),
            b"new cache"
        );
        finish_cache_rebuild(&meta).unwrap();
        assert!(!is_needed(&instances, &meta));
    }
}

#[test]
fn cross_device_file_copy_preserves_source_on_conflict_and_permissions_on_success() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"source data").unwrap();
    fs::write(&destination, b"current data").unwrap();
    assert!(copy_file_then_remove(&source, &destination).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"source data");
    assert_eq!(fs::read(&destination).unwrap(), b"current data");
    fs::remove_file(&destination).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
    }
    copy_file_then_remove(&source, &destination).unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"source data");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn visible_instances_with_leftover_local_configs_are_migrated_even_after_a_completed_step() {
    for recorded_step in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let instances = temp.path().join("instances");
        let meta = temp.path().join("meta");
        let config = temp.path().join("config.toml");
        let instance = instances.join("Example");
        let source = instance.join("rmcl/config-sync/local-config");
        let destination = instance.join("rmcl/content/config");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("options.txt"), b"options").unwrap();
        fs::write(destination.join("options.txt"), b"options").unwrap();
        fs::write(&config, b"[paths]").unwrap();
        initialize_new_layout(&meta).unwrap();
        if recorded_step {
            let metadata = MetadataPaths::new(&meta);
            let backup = metadata.backups().join("backup-resume");
            fs::create_dir_all(&backup).unwrap();
            write_json_atomic(
                &metadata.migration_journal(),
                &MigrationJournal {
                    version: LAYOUT_VERSION,
                    backup_dir: backup,
                    completed: vec!["backup".to_owned(), "instance:Example".to_owned()],
                },
            )
            .unwrap();
        }
        assert!(is_needed(&instances, &meta));
        run(&instances, &meta, &config, |_| {}).unwrap();
        assert!(!source.exists());
        assert_eq!(
            fs::read(destination.join("options.txt")).unwrap(),
            b"options"
        );
        finish_cache_rebuild(&meta).unwrap();
        assert!(!is_needed(&instances, &meta));
    }
}

#[test]
fn missing_recorded_backup_is_recreated_before_moving_user_data() {
    let temp = tempfile::tempdir().unwrap();
    let instances = temp.path().join("instances");
    let meta = temp.path().join("meta");
    let config = temp.path().join("config.toml");
    let instance = instances.join("Example");
    fs::create_dir_all(instance.join(".minecraft/saves/world")).unwrap();
    fs::write(instance.join(".minecraft/saves/world/level.dat"), b"world").unwrap();
    let metadata = MetadataPaths::new(&meta);
    let backup = metadata.backups().join("backup-resume");
    write_json_atomic(
        &metadata.migration_journal(),
        &MigrationJournal {
            version: LAYOUT_VERSION,
            backup_dir: backup.clone(),
            completed: vec!["backup".to_owned()],
        },
    )
    .unwrap();

    assert_eq!(run(&instances, &meta, &config, |_| {}).unwrap(), backup);
    assert_eq!(
        fs::read(backup.join("instances/Example/.minecraft/saves/world/level.dat")).unwrap(),
        b"world"
    );
    assert_eq!(
        fs::read(instance.join("minecraft/saves/world/level.dat")).unwrap(),
        b"world"
    );
}
