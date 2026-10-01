// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn first_prepare_seeds_shared_config() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let minecraft = tmp.path().join("instance/minecraft");
    create_profile(&meta, "main").unwrap();
    std::fs::create_dir_all(minecraft.join("config/nested")).unwrap();
    std::fs::write(minecraft.join("options.txt"), "local-options").unwrap();
    std::fs::write(minecraft.join("optionsshaders.txt"), "shader-options").unwrap();
    std::fs::write(minecraft.join("config/options.txt"), "local-config").unwrap();
    std::fs::write(minecraft.join("config/nested/mod.toml"), "nested").unwrap();

    assert!(prepare(Some("main"), &meta, &minecraft).unwrap());

    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/main/options.txt")).unwrap(),
        "local-options"
    );
    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/main/optionsshaders.txt")).unwrap(),
        "shader-options"
    );
    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/main/config/options.txt")).unwrap(),
        "local-config"
    );
    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/main/config/nested/mod.toml")).unwrap(),
        "nested"
    );
}

#[test]
fn prepare_mirrors_shared_config_into_instance() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let minecraft = tmp.path().join("instance/minecraft");
    std::fs::create_dir_all(meta.join("state/profiles/main/config")).unwrap();
    std::fs::create_dir_all(minecraft.join("config")).unwrap();
    std::fs::write(
        meta.join("state/profiles/main/options.txt"),
        "shared-options",
    )
    .unwrap();
    std::fs::write(
        meta.join("state/profiles/main/config/shared.toml"),
        "shared",
    )
    .unwrap();
    std::fs::write(minecraft.join("options.txt"), "stale-options").unwrap();
    std::fs::write(minecraft.join("config/local.toml"), "stale").unwrap();

    assert!(prepare(Some("main"), &meta, &minecraft).unwrap());

    assert_eq!(
        std::fs::read_to_string(minecraft.join("options.txt")).unwrap(),
        "shared-options"
    );
    assert_eq!(
        std::fs::read_to_string(minecraft.join("config/shared.toml")).unwrap(),
        "shared"
    );
    assert!(!minecraft.join("config/local.toml").exists());
}

#[test]
fn finish_mirrors_instance_config_back_to_shared() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let minecraft = tmp.path().join("instance/minecraft");
    std::fs::create_dir_all(meta.join("state/profiles/main/config")).unwrap();
    std::fs::write(meta.join("state/profiles/main/config/old.toml"), "old").unwrap();
    std::fs::create_dir_all(minecraft.join("config")).unwrap();
    std::fs::write(minecraft.join("options.txt"), "new-options").unwrap();
    std::fs::write(minecraft.join("config/new.toml"), "new").unwrap();

    finish(Some("main"), &meta, &minecraft).unwrap();

    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/main/options.txt")).unwrap(),
        "new-options"
    );
    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/main/config/new.toml")).unwrap(),
        "new"
    );
    assert!(!meta.join("state/profiles/main/config/old.toml").exists());
}

#[test]
fn prepare_releases_lock_for_another_instance() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let minecraft = tmp.path().join("instance/minecraft");
    create_profile(&meta, "main").unwrap();
    std::fs::create_dir_all(minecraft.join("config")).unwrap();

    assert!(prepare(Some("main"), &meta, &minecraft).unwrap());
    let second = tmp.path().join("second/minecraft");
    std::fs::create_dir_all(second.join("config")).unwrap();

    assert!(prepare(Some("main"), &meta, &second).unwrap());
}

#[test]
fn active_launch_keeps_its_profile_locked_until_finish() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let first = tmp.path().join("first/minecraft");
    let second = tmp.path().join("second/minecraft");
    std::fs::create_dir_all(first.join("config")).unwrap();
    std::fs::create_dir_all(second.join("config")).unwrap();
    create_profile(&meta, "main").unwrap();

    let lock = prepare_for_launch(Some("main"), &meta, &first)
        .unwrap()
        .unwrap();
    assert!(matches!(
        prepare_for_launch(Some("main"), &meta, &second),
        Err(ConfigSyncError::ProfileInUse(_))
    ));
    assert!(matches!(
        delete_profile(&meta, "main"),
        Err(ConfigSyncError::ProfileInUse(_))
    ));
    finish_launch(lock, &first).unwrap();
    assert!(
        prepare_for_launch(Some("main"), &meta, &second)
            .unwrap()
            .is_some()
    );
}

#[test]
fn failed_options_source_read_keeps_existing_options() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("not-a-directory");
    let target = tmp.path().join("target");
    std::fs::write(&source, b"invalid").unwrap();
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("options.txt"), b"keep").unwrap();

    assert!(mirror_options(&source, &target).is_err());
    assert_eq!(std::fs::read(target.join("options.txt")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn config_sync_preserves_symlinked_config_and_options() {
    let tmp = tempfile::tempdir().unwrap();
    let minecraft = tmp.path().join("minecraft");
    let profile = tmp.path().join("profile");
    std::fs::create_dir_all(minecraft.join("config")).unwrap();
    std::fs::write(tmp.path().join("shared.toml"), "settings").unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("shared.toml"),
        minecraft.join("config/mod.toml"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("shared.toml"),
        minecraft.join("options.txt"),
    )
    .unwrap();

    sync_to_profile(&minecraft, &profile).unwrap();
    assert!(
        std::fs::symlink_metadata(profile.join("config/mod.toml"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::symlink_metadata(profile.join("options.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    sync_from_profile(&profile, &minecraft).unwrap();
    assert!(
        std::fs::symlink_metadata(minecraft.join("config/mod.toml"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::symlink_metadata(minecraft.join("options.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn symlink_only_profile_is_not_reinitialized_or_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let minecraft = tmp.path().join("minecraft");
    let meta = tmp.path().join("meta");
    let profile = crate::storage::MetadataPaths::new(&meta)
        .profiles()
        .join("linked");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::create_dir_all(&minecraft).unwrap();
    std::fs::write(tmp.path().join("shared-options"), b"profile").unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("shared-options"),
        profile.join("options.txt"),
    )
    .unwrap();

    let lock = prepare_for_launch(Some("linked"), &meta, &minecraft).unwrap();
    assert!(lock.is_some());
    assert_eq!(
        std::fs::read(minecraft.join("options.txt")).unwrap(),
        b"profile"
    );
    assert!(
        std::fs::symlink_metadata(minecraft.join("options.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let external = tmp.path().join("external-config");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(external.join("keep.txt"), b"keep").unwrap();
    std::fs::remove_dir(minecraft.join("config")).unwrap();
    std::os::unix::fs::symlink(&external, minecraft.join("config")).unwrap();
    assert!(sync_from_profile(&profile, &minecraft).is_err());
    assert!(
        std::fs::symlink_metadata(minecraft.join("config"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(external.join("keep.txt")).unwrap(), b"keep");
}

#[test]
fn profile_rejects_path_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let err = prepare(Some("../bad"), tmp.path(), tmp.path()).unwrap_err();

    assert!(matches!(err, ConfigSyncError::InvalidProfile(_)));
}

#[test]
fn profile_rejects_builtin_names() {
    for profile in [
        "none",
        "default",
        "local",
        "instance default",
        "local default",
    ] {
        let err = validate_profile(profile).unwrap_err();
        assert!(matches!(err, ConfigSyncError::InvalidProfile(_)));
    }
}

#[test]
fn prepare_ignores_deleted_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let minecraft = tmp.path().join("instance/minecraft");
    std::fs::create_dir_all(minecraft.join("config")).unwrap();

    let lock = prepare(Some("deleted"), &meta, &minecraft).unwrap();

    assert!(!lock);
    assert!(!meta.join("state/profiles/deleted").exists());
}

#[test]
fn create_profile_trims_and_lists_profiles() {
    let tmp = tempfile::tempdir().unwrap();

    let profile = create_profile(tmp.path(), " main ").unwrap();
    let profiles = list_profiles(tmp.path()).unwrap();

    assert_eq!(profile, "main");
    assert_eq!(profiles, vec!["main"]);
}

#[test]
fn delete_profile_removes_profile_dir() {
    let tmp = tempfile::tempdir().unwrap();
    create_profile(tmp.path(), "main").unwrap();

    delete_profile(tmp.path(), "main").unwrap();

    assert!(list_profiles(tmp.path()).unwrap().is_empty());
}

#[test]
fn switch_to_profile_backs_up_local_config_and_restores_none() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let instance = tmp.path().join("instance");
    let minecraft = instance.join(crate::storage::MINECRAFT_DIR_NAME);
    std::fs::create_dir_all(minecraft.join("config")).unwrap();
    std::fs::write(minecraft.join("options.txt"), "local-options").unwrap();
    std::fs::write(minecraft.join("config/local.txt"), "local").unwrap();

    let selected = switch_profile("inst", None, Some("main"), &meta, &instance).unwrap();
    assert_eq!(selected.as_deref(), Some("main"));
    assert_eq!(
        std::fs::read_to_string(instance.join("rmcl/content/config/options.txt")).unwrap(),
        "local-options"
    );
    assert_eq!(
        std::fs::read_to_string(instance.join("rmcl/content/config/config/local.txt")).unwrap(),
        "local"
    );

    std::fs::write(minecraft.join("options.txt"), "shared-options").unwrap();
    std::fs::write(minecraft.join("config/shared.txt"), "shared").unwrap();
    let selected = switch_profile("inst", Some("main"), None, &meta, &instance).unwrap();

    assert_eq!(selected, None);
    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/main/options.txt")).unwrap(),
        "shared-options"
    );
    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/main/config/shared.txt")).unwrap(),
        "shared"
    );
    assert_eq!(
        std::fs::read_to_string(minecraft.join("options.txt")).unwrap(),
        "local-options"
    );
    assert_eq!(
        std::fs::read_to_string(minecraft.join("config/local.txt")).unwrap(),
        "local"
    );
    assert!(!minecraft.join("config/shared.txt").exists());
}

#[test]
fn switch_from_deleted_profile_restores_local_without_recreating_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let instance = tmp.path().join("instance");
    let minecraft = instance.join(crate::storage::MINECRAFT_DIR_NAME);
    std::fs::create_dir_all(minecraft.join("config")).unwrap();
    std::fs::create_dir_all(instance.join("rmcl/content/config/config")).unwrap();
    std::fs::write(minecraft.join("options.txt"), "deleted-profile-options").unwrap();
    std::fs::write(
        instance.join("rmcl/content/config/options.txt"),
        "local-options",
    )
    .unwrap();
    std::fs::write(
        instance.join("rmcl/content/config/config/local.txt"),
        "local",
    )
    .unwrap();

    let selected = switch_profile("inst", Some("deleted"), None, &meta, &instance).unwrap();

    assert_eq!(selected, None);
    assert_eq!(
        std::fs::read_to_string(minecraft.join("options.txt")).unwrap(),
        "local-options"
    );
    assert_eq!(
        std::fs::read_to_string(minecraft.join("config/local.txt")).unwrap(),
        "local"
    );
    assert!(!meta.join("state/profiles/deleted").exists());
}

#[test]
fn switch_between_profiles_saves_old_and_loads_new() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let instance = tmp.path().join("instance");
    let minecraft = instance.join(crate::storage::MINECRAFT_DIR_NAME);
    std::fs::create_dir_all(minecraft.join("config")).unwrap();
    std::fs::write(minecraft.join("options.txt"), "changed-a-options").unwrap();
    std::fs::write(minecraft.join("config/a.txt"), "changed-a").unwrap();
    create_profile(&meta, "a").unwrap();
    std::fs::create_dir_all(meta.join("state/profiles/b/config")).unwrap();
    std::fs::write(
        meta.join("state/profiles/b/options.txt"),
        "profile-b-options",
    )
    .unwrap();
    std::fs::write(meta.join("state/profiles/b/config/b.txt"), "profile-b").unwrap();

    let selected = switch_profile("inst", Some("a"), Some("b"), &meta, &instance).unwrap();

    assert_eq!(selected.as_deref(), Some("b"));
    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/a/options.txt")).unwrap(),
        "changed-a-options"
    );
    assert_eq!(
        std::fs::read_to_string(meta.join("state/profiles/a/config/a.txt")).unwrap(),
        "changed-a"
    );
    assert_eq!(
        std::fs::read_to_string(minecraft.join("options.txt")).unwrap(),
        "profile-b-options"
    );
    assert_eq!(
        std::fs::read_to_string(minecraft.join("config/b.txt")).unwrap(),
        "profile-b"
    );
    assert!(!minecraft.join("config/a.txt").exists());
}

#[test]
fn failed_profile_save_restores_previous_files() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let instance = tmp.path().join("instance");
    let minecraft = instance.join(crate::storage::MINECRAFT_DIR_NAME);
    std::fs::create_dir_all(minecraft.join("config")).unwrap();
    std::fs::create_dir_all(meta.join("state/profiles/main/config")).unwrap();
    std::fs::write(minecraft.join("options.txt"), "local-options").unwrap();
    std::fs::write(minecraft.join("config/local.txt"), "local").unwrap();
    std::fs::write(
        meta.join("state/profiles/main/options.txt"),
        "shared-options",
    )
    .unwrap();
    std::fs::write(meta.join("state/profiles/main/config/shared.txt"), "shared").unwrap();

    let error = switch_profile_and_persist("inst", None, Some("main"), &meta, &instance, |_| {
        Err(std::io::Error::other("disk full"))
    })
    .unwrap_err();

    assert!(matches!(error, ConfigSyncError::Save(_)));
    assert_eq!(
        std::fs::read_to_string(minecraft.join("options.txt")).unwrap(),
        "local-options"
    );
    assert_eq!(
        std::fs::read_to_string(minecraft.join("config/local.txt")).unwrap(),
        "local"
    );
    assert!(!minecraft.join("config/shared.txt").exists());
    assert_eq!(
        std::fs::read(meta.join("state/profiles/main/options.txt")).unwrap(),
        b"shared-options"
    );
    assert_eq!(
        std::fs::read(meta.join("state/profiles/main/config/shared.txt")).unwrap(),
        b"shared"
    );
}

#[test]
fn second_instance_uses_profile_options_saved_by_first_instance() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let first = tmp.path().join("first/minecraft");
    let second_instance = tmp.path().join("second");
    let second = second_instance.join(crate::storage::MINECRAFT_DIR_NAME);
    std::fs::create_dir_all(first.join("config")).unwrap();
    std::fs::create_dir_all(second.join("config")).unwrap();
    std::fs::write(first.join("options.txt"), "first-default").unwrap();
    std::fs::write(second.join("options.txt"), "second-local").unwrap();
    create_profile(&meta, "main").unwrap();

    assert!(prepare(Some("main"), &meta, &first).unwrap());
    std::fs::write(first.join("options.txt"), "changed-in-main").unwrap();
    finish(Some("main"), &meta, &first).unwrap();

    switch_profile("second", None, Some("main"), &meta, &second_instance).unwrap();

    assert_eq!(
        std::fs::read_to_string(second.join("options.txt")).unwrap(),
        "changed-in-main"
    );
    assert_eq!(
        std::fs::read_to_string(second_instance.join("rmcl/content/config/options.txt")).unwrap(),
        "second-local"
    );
}

#[test]
fn options_commit_failure_restores_the_entire_payload() {
    for sync in [sync_to_profile as fn(&Path, &Path) -> _, sync_from_profile] {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("source");
        let dst = tmp.path().join("target");
        std::fs::create_dir_all(src.join("config")).unwrap();
        std::fs::create_dir_all(dst.join("config")).unwrap();
        std::fs::create_dir_all(dst.join("optionsz.txt")).unwrap();
        std::fs::write(src.join("config/new"), b"new").unwrap();
        std::fs::write(src.join("options.txt"), b"new-options").unwrap();
        std::fs::write(src.join("optionsz.txt"), b"collision").unwrap();
        std::fs::write(dst.join("config/old"), b"old").unwrap();
        std::fs::write(dst.join("options.txt"), b"old-options").unwrap();
        std::fs::write(dst.join("optionslegacy.txt"), b"legacy").unwrap();
        std::fs::write(dst.join("optionsz.txt/keep"), b"keep").unwrap();
        assert!(sync(&src, &dst).is_err());
        assert_eq!(std::fs::read(dst.join("config/old")).unwrap(), b"old");
        assert!(!dst.join("config/new").exists());
        assert_eq!(
            std::fs::read(dst.join("options.txt")).unwrap(),
            b"old-options"
        );
        assert_eq!(
            std::fs::read(dst.join("optionslegacy.txt")).unwrap(),
            b"legacy"
        );
        assert_eq!(
            std::fs::read(dst.join("optionsz.txt/keep")).unwrap(),
            b"keep"
        );
        assert!(!tmp.path().join(".target.sync-backup").exists());
    }
}

#[test]
fn failed_rollback_retains_original_payload_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("source");
    let dst = tmp.path().join("target");
    std::fs::create_dir_all(src.join("config")).unwrap();
    std::fs::create_dir_all(dst.join("config")).unwrap();
    std::fs::write(src.join("config/value"), b"new").unwrap();
    std::fs::write(dst.join("config/value"), b"original").unwrap();
    std::fs::write(dst.join("options.txt"), b"original-options").unwrap();
    let error = mirror_payload(&src, &dst, true, || {
        std::fs::rename(&dst, tmp.path().join("displaced"))?;
        std::fs::write(&dst, b"blocks restoration")?;
        Err::<(), _>(ConfigSyncError::Save("injected failure".to_owned()))
    })
    .unwrap_err();
    assert!(matches!(error, ConfigSyncError::SaveRollback { .. }));
    let backup = tmp.path().join(".target.sync-backup");
    assert_eq!(
        std::fs::read(backup.join("config/value")).unwrap(),
        b"original"
    );
    assert_eq!(
        std::fs::read(backup.join("options.txt")).unwrap(),
        b"original-options"
    );
}

#[test]
fn failed_switch_io_does_not_save_selection_or_change_local_files() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let instance = tmp.path().join("instance");
    let local = minecraft_dir(&instance);
    let profile = profile_dir(&meta, "main");
    std::fs::create_dir_all(local.join("config")).unwrap();
    std::fs::create_dir_all(local.join("optionsz.txt")).unwrap();
    std::fs::create_dir_all(profile.join("config")).unwrap();
    std::fs::write(local.join("config/value"), b"local").unwrap();
    std::fs::write(local.join("options.txt"), b"local-options").unwrap();
    std::fs::write(profile.join("config/value"), b"profile").unwrap();
    std::fs::write(profile.join("optionsz.txt"), b"collision").unwrap();
    let mut saved = false;
    assert!(
        switch_profile_and_persist("inst", None, Some("main"), &meta, &instance, |_| {
            saved = true;
            Ok::<(), std::io::Error>(())
        })
        .is_err()
    );
    assert!(!saved);
    assert_eq!(std::fs::read(local.join("config/value")).unwrap(), b"local");
    assert_eq!(
        std::fs::read(local.join("options.txt")).unwrap(),
        b"local-options"
    );
}

#[test]
fn failed_publication_recovers_local_changes_before_the_next_import() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let local = tmp.path().join("instance/minecraft");
    std::fs::create_dir_all(local.join("config")).unwrap();
    std::fs::write(local.join("config/value"), b"original").unwrap();
    create_profile(&meta, "main").unwrap();
    let lock = prepare_for_launch(Some("main"), &meta, &local)
        .unwrap()
        .unwrap();
    let profile = profile_dir(&meta, "main");
    std::fs::write(local.join("config/value"), b"changed").unwrap();
    std::fs::write(local.join("optionsz.txt"), b"changed-options").unwrap();
    std::fs::create_dir(profile.join("optionsz.txt")).unwrap();
    assert!(finish_launch(lock, &local).is_err());
    assert!(unsynced_marker(&profile).exists());
    assert!(prepare_for_launch(Some("main"), &meta, &local).is_err());
    assert_eq!(
        std::fs::read(local.join("config/value")).unwrap(),
        b"changed"
    );
    std::fs::remove_dir(profile.join("optionsz.txt")).unwrap();
    let lock = prepare_for_launch(Some("main"), &meta, &local)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read(local.join("config/value")).unwrap(),
        b"changed"
    );
    assert_eq!(
        std::fs::read(profile.join("config/value")).unwrap(),
        b"changed"
    );
    assert_eq!(
        std::fs::read(profile.join("optionsz.txt")).unwrap(),
        b"changed-options"
    );
    finish_launch(lock, &local).unwrap();
    assert!(!unsynced_marker(&profile).exists());
}

#[test]
fn invalid_recovery_source_keeps_profile_and_local_payloads() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let local = tmp.path().join("instance/minecraft");
    let profile = profile_dir(&meta, "main");
    std::fs::create_dir_all(local.join("config")).unwrap();
    std::fs::create_dir_all(profile.join("config")).unwrap();
    std::fs::write(local.join("config/value"), b"local").unwrap();
    std::fs::write(profile.join("config/value"), b"shared").unwrap();
    std::fs::write(
        unsynced_marker(&profile),
        serde_json::to_vec(&tmp.path().join("missing")).unwrap(),
    )
    .unwrap();
    assert!(prepare_for_launch(Some("main"), &meta, &local).is_err());
    assert!(unsynced_marker(&profile).exists());
    assert_eq!(std::fs::read(local.join("config/value")).unwrap(), b"local");
    assert_eq!(
        std::fs::read(profile.join("config/value")).unwrap(),
        b"shared"
    );
}

#[cfg(unix)]
#[test]
fn relative_symlinks_keep_internal_and_external_referents_at_different_depths() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    let target = tmp.path().join("deep/target");
    std::fs::create_dir_all(source.join("config/nested")).unwrap();
    std::fs::write(tmp.path().join("shared"), b"external").unwrap();
    std::fs::write(source.join("options.txt"), b"options").unwrap();
    std::fs::write(source.join("config/nested/value"), b"nested").unwrap();
    std::os::unix::fs::symlink("value", source.join("config/nested/sibling")).unwrap();
    std::os::unix::fs::symlink("../options.txt", source.join("config/to-options")).unwrap();
    std::os::unix::fs::symlink("config/nested/value", source.join("optionslinked.txt")).unwrap();
    std::os::unix::fs::symlink("missing", source.join("config/dangling")).unwrap();
    std::os::unix::fs::symlink("../../shared", source.join("config/external")).unwrap();
    std::os::unix::fs::symlink("../shared", source.join("optionsexternal.txt")).unwrap();
    sync_to_profile(&source, &target).unwrap();
    std::fs::remove_dir_all(&source).unwrap();
    for (path, bytes) in [
        ("config/nested/sibling", b"nested".as_slice()),
        ("config/to-options", b"options".as_slice()),
        ("optionslinked.txt", b"nested".as_slice()),
        ("config/external", b"external".as_slice()),
        ("optionsexternal.txt", b"external".as_slice()),
    ] {
        assert_eq!(std::fs::read(target.join(path)).unwrap(), bytes, "{path}");
    }
    assert_eq!(
        std::fs::read_link(target.join("config/dangling")).unwrap(),
        Path::new("missing")
    );
}

fn write_payload(dir: &Path, value: &str) {
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/value"), value).unwrap();
    std::fs::write(dir.join("options.txt"), value).unwrap();
}

fn saved_instance(
    manager: &crate::instance::InstanceManager,
    name: &str,
) -> crate::instance::InstanceConfig {
    let config = serde_json::from_value(serde_json::json!({
        "name": name, "game_version": "1.21.1", "loader": "vanilla", "loader_version": null,
        "created": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    write_payload(
        &minecraft_dir(&manager.instances_dir.join(name)),
        "local-default",
    );
    manager.save(&config).unwrap();
    config
}

#[test]
fn missing_launch_source_preserves_profile_and_recovery_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let local = tmp.path().join("instance/minecraft");
    write_payload(&local, "keep");
    create_profile(&meta, "main").unwrap();
    let lock = prepare_for_launch(Some("main"), &meta, &local)
        .unwrap()
        .unwrap();
    std::fs::remove_dir_all(&local).unwrap();
    assert!(finish_launch(lock, &local).is_err());
    let profile = profile_dir(&meta, "main");
    assert_eq!(
        std::fs::read(profile.join("config/value")).unwrap(),
        b"keep"
    );
    assert_eq!(std::fs::read(profile.join("options.txt")).unwrap(), b"keep");
    assert!(unsynced_marker(&profile).exists());
}

#[test]
fn missing_seed_source_does_not_initialize_the_profile() {
    let tmp = tempfile::tempdir().unwrap();
    create_profile(tmp.path(), "main").unwrap();
    let local = tmp.path().join("missing/minecraft");
    assert!(prepare_for_launch(Some("main"), tmp.path(), &local).is_err());
    let profile = profile_dir(tmp.path(), "main");
    assert!(std::fs::read_dir(&profile).unwrap().next().is_none());
    assert!(!local.exists());
}

#[test]
fn missing_switch_source_does_not_publish_or_save_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = crate::instance::InstanceManager::new(
        tmp.path().join("instances"),
        tmp.path().join("meta"),
    );
    saved_instance(&manager, "inst");
    let mut config = manager
        .save_config_sync_profile("inst", Some("main".to_owned()))
        .unwrap();
    let profile = profile_dir(&manager.meta_dir, "main");
    write_payload(&profile, "shared");
    std::fs::remove_dir_all(minecraft_dir(&manager.instances_dir.join("inst"))).unwrap();
    assert!(switch_profile_and_save(&manager, &mut config, None).is_err());
    assert_eq!(
        manager
            .load_one("inst")
            .unwrap()
            .config_sync_profile
            .as_deref(),
        Some("main")
    );
    assert_eq!(
        std::fs::read(profile.join("config/value")).unwrap(),
        b"shared"
    );
    assert_eq!(
        std::fs::read(profile.join("options.txt")).unwrap(),
        b"shared"
    );
}

#[test]
fn stale_public_config_reloads_current_profile_and_preserves_newer_fields_and_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = crate::instance::InstanceManager::new(
        tmp.path().join("instances"),
        tmp.path().join("meta"),
    );
    let mut stale = saved_instance(&manager, "inst");
    let mut current = stale.clone();
    write_payload(&profile_dir(&manager.meta_dir, "a"), "profile-a");
    write_payload(&profile_dir(&manager.meta_dir, "b"), "profile-b");
    switch_profile_and_save(&manager, &mut current, Some("a")).unwrap();
    current.memory_max = Some("8G".to_owned());
    current
        .environment
        .insert("UPDATED".to_owned(), "yes".to_owned());
    manager.save(&current).unwrap();

    let other = minecraft_dir(&manager.instances_dir.join("other"));
    write_payload(&other, "other-default");
    let lock = prepare_for_launch(Some("a"), &manager.meta_dir, &other)
        .unwrap()
        .unwrap();
    write_payload(&other, "newer-a");
    finish_launch(lock, &other).unwrap();

    switch_profile_and_save(&manager, &mut stale, Some("b")).unwrap();
    assert_eq!(stale, manager.load_one("inst").unwrap());
    assert_eq!(stale.memory_max.as_deref(), Some("8G"));
    assert_eq!(stale.environment["UPDATED"], "yes");
    assert_eq!(
        std::fs::read(profile_dir(&manager.meta_dir, "a").join("options.txt")).unwrap(),
        b"newer-a"
    );
    assert_eq!(
        std::fs::read(local_backup_dir(&manager.instances_dir.join("inst")).join("options.txt"))
            .unwrap(),
        b"local-default"
    );
    switch_profile_and_save(&manager, &mut stale, None).unwrap();
    assert_eq!(
        std::fs::read(minecraft_dir(&manager.instances_dir.join("inst")).join("options.txt"))
            .unwrap(),
        b"local-default"
    );
}

#[test]
fn public_switch_preserves_settings_updated_while_selection_save_is_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = crate::instance::InstanceManager::new(
        tmp.path().join("instances"),
        tmp.path().join("meta"),
    );
    let config = saved_instance(&manager, "inst");
    let instance = manager.instances_dir.join("inst");
    write_payload(&profile_dir(&manager.meta_dir, "main"), "shared");
    let config_lock = manager.config_lock("inst").unwrap();
    let instances_dir = manager.instances_dir.clone();
    let meta_dir = manager.meta_dir.clone();
    let worker = std::thread::spawn(move || {
        let manager = crate::instance::InstanceManager::new(instances_dir, meta_dir);
        let mut config = config;
        switch_profile_and_save(&manager, &mut config, Some("main")).unwrap();
        config
    });
    let start = std::time::Instant::now();
    while !std::fs::read(minecraft_dir(&instance).join("options.txt"))
        .is_ok_and(|bytes| bytes == b"shared")
    {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "profile payload was not installed before persistence"
        );
        std::thread::yield_now();
    }
    let mut concurrent = manager.load_one("inst").unwrap();
    assert_eq!(concurrent.config_sync_profile, None);
    concurrent.java_path = Some("changed-during-switch".to_owned());
    // This writer owns the config lock while the switch waits to persist its selection.
    crate::storage::write_atomic(
        &instance.join("instance.json"),
        &serde_json::to_vec_pretty(&concurrent).unwrap(),
    )
    .unwrap();
    drop(config_lock);
    let saved = worker.join().unwrap();
    assert_eq!(saved.java_path.as_deref(), Some("changed-during-switch"));
    assert_eq!(saved.config_sync_profile.as_deref(), Some("main"));
    assert_eq!(saved, manager.load_one("inst").unwrap());
}

#[test]
fn second_instance_switch_recovers_failed_publication_without_overwriting_it() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let profile = profile_dir(&meta, "main");
    let first = tmp.path().join("first/minecraft");
    let second_instance = tmp.path().join("second");
    let second = minecraft_dir(&second_instance);
    write_payload(&profile, "original");
    write_payload(&first, "first-default");
    write_payload(&second, "second-default");
    switch_profile("second", None, Some("main"), &meta, &second_instance).unwrap();
    let lock = prepare_for_launch(Some("main"), &meta, &first)
        .unwrap()
        .unwrap();
    write_payload(&first, "changed");
    std::fs::write(first.join("optionsz.txt"), b"changed-options").unwrap();
    std::fs::create_dir(profile.join("optionsz.txt")).unwrap();
    assert!(finish_launch(lock, &first).is_err());
    assert!(switch_profile("second", Some("main"), None, &meta, &second_instance).is_err());
    assert!(unsynced_marker(&profile).exists());
    assert_eq!(
        std::fs::read(second.join("options.txt")).unwrap(),
        b"original"
    );
    write_payload(&first, "changed-after-failure");
    std::fs::remove_dir(profile.join("optionsz.txt")).unwrap();
    switch_profile("second", Some("main"), None, &meta, &second_instance).unwrap();
    assert_eq!(
        std::fs::read(profile.join("options.txt")).unwrap(),
        b"changed-after-failure"
    );
    assert_eq!(
        std::fs::read(first.join("options.txt")).unwrap(),
        b"changed-after-failure"
    );
    assert_eq!(
        std::fs::read(second.join("options.txt")).unwrap(),
        b"second-default"
    );
    assert!(!unsynced_marker(&profile).exists());

    let third = tmp.path().join("third/minecraft");
    write_payload(&third, "third-default");
    let lock = prepare_for_launch(Some("main"), &meta, &third)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read(third.join("options.txt")).unwrap(),
        b"changed-after-failure"
    );
    finish_launch(lock, &third).unwrap();
}

#[test]
fn successful_publication_keeps_stale_copies_from_overwriting_newer_settings() {
    for edited in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let meta = tmp.path().join("meta");
        let profile = profile_dir(&meta, "main");
        let first = tmp.path().join("first/minecraft");
        let second_instance = tmp.path().join("second");
        let second = minecraft_dir(&second_instance);
        write_payload(&profile, "original");
        write_payload(&first, "first-default");
        write_payload(&second, "second-default");
        switch_profile("second", None, Some("main"), &meta, &second_instance).unwrap();
        let lock = prepare_for_launch(Some("main"), &meta, &first)
            .unwrap()
            .unwrap();
        write_payload(&first, "new-shared");
        finish_launch(lock, &first).unwrap();
        if edited {
            write_payload(&second, "local-edits");
        }
        let baseline = std::fs::read(baseline_path(&second).unwrap()).unwrap();
        let mut saved = false;
        let result = switch_profile_and_persist(
            "second",
            Some("main"),
            None,
            &meta,
            &second_instance,
            |_| {
                saved = true;
                Ok::<(), std::io::Error>(())
            },
        );
        assert_eq!(
            std::fs::read(profile.join("options.txt")).unwrap(),
            b"new-shared"
        );
        if edited {
            assert!(matches!(result, Err(ConfigSyncError::Conflict { .. })));
            assert!(!saved);
            assert_eq!(
                std::fs::read(second.join("options.txt")).unwrap(),
                b"local-edits"
            );
            assert_eq!(
                std::fs::read(baseline_path(&second).unwrap()).unwrap(),
                baseline
            );
            assert!(matches!(
                prepare_for_launch(Some("main"), &meta, &second),
                Err(ConfigSyncError::Conflict { .. })
            ));
            assert_eq!(
                std::fs::read(second.join("options.txt")).unwrap(),
                b"local-edits"
            );
        } else {
            result.unwrap();
            assert!(saved);
            assert_eq!(
                std::fs::read(second.join("options.txt")).unwrap(),
                b"second-default"
            );
        }
    }
}

#[test]
fn local_edits_publish_when_the_imported_profile_has_not_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let instance = tmp.path().join("instance");
    let local = minecraft_dir(&instance);
    write_payload(&local, "default");
    write_payload(&profile_dir(&meta, "a"), "profile-a");
    write_payload(&profile_dir(&meta, "b"), "profile-b");
    switch_profile("inst", None, Some("a"), &meta, &instance).unwrap();
    write_payload(&local, "local-edits");
    switch_profile("inst", Some("a"), Some("b"), &meta, &instance).unwrap();
    assert_eq!(
        std::fs::read(profile_dir(&meta, "a").join("options.txt")).unwrap(),
        b"local-edits"
    );
    assert_eq!(
        std::fs::read(local.join("options.txt")).unwrap(),
        b"profile-b"
    );
}

#[test]
fn untracked_selected_payload_cannot_overwrite_a_different_existing_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let instance = tmp.path().join("instance");
    let meta = tmp.path().join("meta");
    write_payload(&minecraft_dir(&instance), "untracked-local");
    let profile = profile_dir(&meta, "main");
    write_payload(&profile, "shared");
    assert!(matches!(
        switch_profile("inst", Some("main"), None, &meta, &instance),
        Err(ConfigSyncError::Conflict { .. })
    ));
    assert_eq!(
        std::fs::read(profile.join("options.txt")).unwrap(),
        b"shared"
    );
    assert_eq!(
        std::fs::read(minecraft_dir(&instance).join("options.txt")).unwrap(),
        b"untracked-local"
    );
}

#[test]
fn next_launch_refreshes_an_unchanged_stale_import() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let profile = profile_dir(&meta, "main");
    let first = tmp.path().join("first/minecraft");
    let second_instance = tmp.path().join("second");
    let second = minecraft_dir(&second_instance);
    write_payload(&profile, "original");
    write_payload(&first, "first-default");
    write_payload(&second, "second-default");
    switch_profile("second", None, Some("main"), &meta, &second_instance).unwrap();
    let lock = prepare_for_launch(Some("main"), &meta, &first)
        .unwrap()
        .unwrap();
    write_payload(&first, "new-shared");
    finish_launch(lock, &first).unwrap();
    let lock = prepare_for_launch(Some("main"), &meta, &second)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read(second.join("options.txt")).unwrap(),
        b"new-shared"
    );
    finish_launch(lock, &second).unwrap();
    assert_eq!(
        std::fs::read(profile.join("options.txt")).unwrap(),
        b"new-shared"
    );
}

#[test]
fn finishing_an_untracked_second_source_cannot_overwrite_a_recovered_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let profile = profile_dir(&meta, "main");
    let first = tmp.path().join("first/minecraft");
    let second = tmp.path().join("second/minecraft");
    write_payload(&profile, "original");
    write_payload(&first, "first-default");
    write_payload(&second, "original");
    let lock = prepare_for_launch(Some("main"), &meta, &first)
        .unwrap()
        .unwrap();
    write_payload(&first, "new-shared");
    std::fs::write(first.join("optionsz.txt"), b"new-options").unwrap();
    std::fs::create_dir(profile.join("optionsz.txt")).unwrap();
    assert!(finish_launch(lock, &first).is_err());
    std::fs::remove_dir(profile.join("optionsz.txt")).unwrap();
    assert!(matches!(
        finish(Some("main"), &meta, &second),
        Err(ConfigSyncError::Conflict { .. })
    ));
    assert_eq!(
        std::fs::read(profile.join("options.txt")).unwrap(),
        b"new-shared"
    );
    assert_eq!(
        std::fs::read(first.join("options.txt")).unwrap(),
        b"new-shared"
    );
    assert_eq!(
        std::fs::read(second.join("options.txt")).unwrap(),
        b"original"
    );
}

#[test]
fn baseline_io_failure_rolls_back_import_without_saving_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = crate::instance::InstanceManager::new(
        tmp.path().join("instances"),
        tmp.path().join("meta"),
    );
    let mut config = saved_instance(&manager, "inst");
    let local = minecraft_dir(&manager.instances_dir.join("inst"));
    write_payload(&profile_dir(&manager.meta_dir, "main"), "shared");
    let baseline = baseline_path(&local).unwrap();
    std::fs::create_dir_all(&baseline).unwrap();
    std::fs::write(baseline.join("keep"), b"keep").unwrap();
    assert!(switch_profile_and_save(&manager, &mut config, Some("main")).is_err());
    assert_eq!(manager.load_one("inst").unwrap().config_sync_profile, None);
    assert_eq!(
        std::fs::read(local.join("options.txt")).unwrap(),
        b"local-default"
    );
    assert_eq!(
        std::fs::read(local.join("config/value")).unwrap(),
        b"local-default"
    );
    assert_eq!(std::fs::read(baseline.join("keep")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn failed_baseline_write_rolls_back_publication_and_remains_recoverable() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let local = tmp.path().join("instance/minecraft");
    write_payload(&local, "original");
    create_profile(&meta, "main").unwrap();
    let lock = prepare_for_launch(Some("main"), &meta, &local)
        .unwrap()
        .unwrap();
    let profile = profile_dir(&meta, "main");
    let baseline = baseline_path(&local).unwrap();
    let original_baseline = std::fs::read(&baseline).unwrap();
    let state = baseline.parent().unwrap();
    let permissions = std::fs::metadata(state).unwrap().permissions();
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o500)).unwrap();
    let probe = state.join("write-probe");
    if std::fs::File::create(&probe).is_ok() {
        // Privileged runners can bypass the write denial needed for this check.
        std::fs::set_permissions(state, permissions).unwrap();
        std::fs::remove_file(probe).unwrap();
        return;
    }

    write_payload(&local, "changed");
    std::fs::write(
        local.parent().unwrap().join("shared-options"),
        b"new-link-data",
    )
    .unwrap();
    std::fs::remove_file(local.join("options.txt")).unwrap();
    std::os::unix::fs::symlink("../shared-options", local.join("options.txt")).unwrap();
    let result = finish_launch(lock, &local);
    std::fs::set_permissions(state, permissions).unwrap();

    assert!(matches!(result, Err(ConfigSyncError::Io(_))));
    assert_eq!(
        std::fs::read(profile.join("config/value")).unwrap(),
        b"original"
    );
    assert_eq!(
        std::fs::read(profile.join("options.txt")).unwrap(),
        b"original"
    );
    assert!(
        std::fs::symlink_metadata(profile.join("options.txt"))
            .unwrap()
            .is_file()
    );
    assert_eq!(std::fs::read(&baseline).unwrap(), original_baseline);
    assert!(unsynced_marker(&profile).exists());
    assert_eq!(
        std::fs::read_link(local.join("options.txt")).unwrap(),
        Path::new("../shared-options")
    );

    let lock = prepare_for_launch(Some("main"), &meta, &local)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read(profile.join("config/value")).unwrap(),
        b"changed"
    );
    assert_eq!(
        std::fs::read(profile.join("options.txt")).unwrap(),
        b"new-link-data"
    );
    assert_eq!(
        std::fs::read(local.join("options.txt")).unwrap(),
        b"new-link-data"
    );
    finish_launch(lock, &local).unwrap();
    assert!(!unsynced_marker(&profile).exists());
}

#[test]
fn prepare_rechecks_a_profile_deleted_before_the_locked_payload_phase() {
    let tmp = tempfile::tempdir().unwrap();
    create_profile(tmp.path(), "main").unwrap();
    let profile = profile_dir(tmp.path(), "main");
    let lock = acquire_lock(&profile).unwrap();
    std::fs::remove_dir(&profile).unwrap();
    let local = tmp.path().join("instance/minecraft");
    write_payload(&local, "keep");
    assert!(prepare_locked(lock, &local).unwrap().is_none());
    assert!(!profile.exists());
    assert_eq!(std::fs::read(local.join("options.txt")).unwrap(), b"keep");
}

#[test]
fn profile_names_are_portable_components() {
    for name in [
        "main.", "main ", "NUL", "Con.cfg", "COM1", "COM¹", "LPT9", "C:victim", "a/b", "a\\b",
    ] {
        assert!(
            matches!(
                validate_profile(name),
                Err(ConfigSyncError::InvalidProfile(_))
            ),
            "{name}"
        );
    }
    let tmp = tempfile::tempdir().unwrap();
    for name in ["main.", "NUL", "C:victim"] {
        assert!(create_profile(tmp.path(), name).is_err());
    }
    assert!(list_profiles(tmp.path()).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn dangling_payload_links_remain_internal_but_non_payload_referents_stay_anchored() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    let profile = tmp.path().join("profile");
    let replay = tmp.path().join("deep/instance/minecraft");
    std::fs::create_dir(&source).unwrap();
    std::os::unix::fs::symlink("optionsbase.txt", source.join("options.txt")).unwrap();
    std::os::unix::fs::symlink("config/value", source.join("optionsconfig.txt")).unwrap();
    std::os::unix::fs::symlink("custom.txt", source.join("optionsexternal.txt")).unwrap();
    std::fs::create_dir(source.join("optionsdir.txt")).unwrap();
    std::fs::write(source.join("optionsdir.txt/value"), b"external-dir").unwrap();
    std::os::unix::fs::symlink("optionsdir.txt/value", source.join("optionsdirectory.txt"))
        .unwrap();
    sync_to_profile(&source, &profile).unwrap();
    sync_from_profile(&profile, &replay).unwrap();
    assert_eq!(
        std::fs::read_link(replay.join("options.txt")).unwrap(),
        Path::new("optionsbase.txt")
    );
    assert_eq!(
        std::fs::read_link(replay.join("optionsconfig.txt")).unwrap(),
        Path::new("config/value")
    );
    assert_eq!(
        std::fs::read_link(replay.join("optionsexternal.txt")).unwrap(),
        source.join("custom.txt")
    );
    assert_eq!(
        std::fs::read(replay.join("optionsdirectory.txt")).unwrap(),
        b"external-dir"
    );
    std::fs::remove_dir_all(&source).unwrap();
    std::fs::write(replay.join("optionsbase.txt"), b"new-options").unwrap();
    std::fs::write(replay.join("config/value"), b"new-config").unwrap();
    assert_eq!(
        std::fs::read(replay.join("options.txt")).unwrap(),
        b"new-options"
    );
    assert_eq!(
        std::fs::read(replay.join("optionsconfig.txt")).unwrap(),
        b"new-config"
    );
}

#[cfg(unix)]
#[test]
fn failed_selection_save_restores_the_original_option_symlink_and_baseline() {
    use std::os::unix::fs::MetadataExt;
    let tmp = tempfile::tempdir().unwrap();
    let instance = tmp.path().join("instance");
    let meta = tmp.path().join("meta");
    let local = minecraft_dir(&instance);
    write_payload(&local, "default");
    write_payload(&profile_dir(&meta, "a"), "profile-a");
    write_payload(&profile_dir(&meta, "b"), "profile-b");
    switch_profile("inst", None, Some("a"), &meta, &instance).unwrap();
    std::fs::write(instance.join("shared-options"), b"original-link-data").unwrap();
    std::fs::remove_file(local.join("options.txt")).unwrap();
    std::os::unix::fs::symlink("../shared-options", local.join("options.txt")).unwrap();
    let inode = std::fs::symlink_metadata(local.join("options.txt"))
        .unwrap()
        .ino();
    let error = switch_profile_and_persist("inst", Some("a"), Some("b"), &meta, &instance, |_| {
        assert_eq!(
            std::fs::read(local.join("options.txt")).unwrap(),
            b"profile-b"
        );
        Err(std::io::Error::other("selection save failed"))
    })
    .unwrap_err();
    assert!(matches!(error, ConfigSyncError::Save(_)));
    assert_eq!(
        std::fs::read_link(local.join("options.txt")).unwrap(),
        Path::new("../shared-options")
    );
    assert_eq!(
        std::fs::symlink_metadata(local.join("options.txt"))
            .unwrap()
            .ino(),
        inode
    );
    assert_eq!(
        std::fs::read(instance.join("shared-options")).unwrap(),
        b"original-link-data"
    );
    let baseline = read_baseline(&local).unwrap().unwrap();
    assert_eq!(
        baseline.profile,
        std::fs::canonicalize(profile_dir(&meta, "a")).unwrap()
    );
    assert_eq!(baseline.local, payload_hash(&local).unwrap());
    // The failed switch must not turn the restored A payload into an untracked export.
    switch_profile("inst", Some("a"), None, &meta, &instance).unwrap();
    assert_eq!(
        std::fs::read(local.join("options.txt")).unwrap(),
        b"default"
    );
}

#[cfg(unix)]
#[test]
fn profile_root_symlink_aliases_are_not_listed_or_used() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta");
    let instance = tmp.path().join("instance");
    let local = minecraft_dir(&instance);
    write_payload(&local, "local");
    create_profile(&meta, "real").unwrap();
    let real = profile_dir(&meta, "real");
    write_payload(&real, "shared");
    let alias = profile_dir(&meta, "alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    assert_eq!(list_profiles(&meta).unwrap(), vec!["real"]);
    assert!(prepare_for_launch(Some("alias"), &meta, &local).is_err());
    assert!(switch_profile("inst", None, Some("alias"), &meta, &instance).is_err());
    assert!(switch_profile("inst", Some("alias"), Some("alias"), &meta, &instance).is_err());
    assert!(create_profile(&meta, "alias").is_err());
    assert!(delete_profile(&meta, "alias").is_err());
    assert!(
        std::fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(real.join("options.txt")).unwrap(), b"shared");
    assert_eq!(std::fs::read(local.join("options.txt")).unwrap(), b"local");
}
