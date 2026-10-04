// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use crate::instance::FileFingerprint;

struct NoopProgress;

#[cfg(unix)]
#[test]
fn linked_pack_contents_cannot_recurse_into_themselves() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let pack = minecraft.join("resourcepacks/unpacked");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(pack.join("pack.mcmeta"), b"pack").unwrap();
    std::os::unix::fs::symlink(&pack, pack.join("again")).unwrap();

    let files = content_files(&minecraft).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(directory_fingerprint_metadata(&pack).unwrap().size, 4);
}

#[cfg(unix)]
#[test]
fn inventory_rejects_content_directories_linked_outside_the_instance() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&minecraft).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("foreign.jar"), b"foreign").unwrap();
    std::os::unix::fs::symlink(&outside, minecraft.join("mods")).unwrap();
    let manifest_path = temp.path().join("manifest.json");
    let error = reconcile_inventory(&manifest_path, &minecraft, 24, 512, &NoopProgress)
        .err()
        .unwrap();
    assert!(error.to_string().contains("symlink"));
    assert!(!manifest_path.exists());
}

impl InventoryProgress for NoopProgress {
    fn set_sub_action(&self, _text: &str) {}

    fn set_progress(&self, _current: u64, _total: u64) {}
}

#[test]
fn saving_an_identified_inventory_reports_progress_for_the_actual_files() {
    #[derive(Default)]
    struct Counts(std::cell::RefCell<Vec<(u64, u64)>>);
    impl InventoryProgress for Counts {
        fn set_sub_action(&self, _text: &str) {}
        fn set_progress(&self, current: u64, total: u64) {
            self.0.borrow_mut().push((current, total));
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let manifest_path = temp.path().join("manifest.json");
    let mut previous = ContentManifest::default();
    for name in ["alpha.jar", "bravo.jar"] {
        let mut record = resolved_record();
        record.relative_path = PathBuf::from("mods").join(name);
        let path = minecraft.join(&record.relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, name).unwrap();
        record.fingerprint = fingerprint(&path).unwrap();
        record.provider_checks = vec!["modrinth".to_owned(), "curseforge".to_owned()];
        previous.files.push(record);
    }
    previous.save(&manifest_path).unwrap();
    let counts = Counts::default();
    let inventory = reconcile_inventory(&manifest_path, &minecraft, 24, 512, &counts).unwrap();
    assert!(inventory.queries.is_empty());
    counts.0.borrow_mut().clear();
    let saved = save_reconciled_manifest(
        &manifest_path,
        &minecraft,
        inventory.manifest,
        &inventory.previous,
        &counts,
    )
    .unwrap();
    assert_eq!(saved.files, previous.files);
    let reported = counts.0.borrow();
    assert_eq!(reported.first(), Some(&(0, 2)));
    assert!(reported.contains(&(1, 2)));
    assert_eq!(reported.last(), Some(&(2, 2)));
}

fn resolved_record() -> ContentFileRecord {
    ContentFileRecord {
        relative_path: PathBuf::from("mods/example.jar"),
        kind: ContentKind::Mod,
        enabled: true,
        fingerprint: FileFingerprint {
            size: 1,
            modified_ns: 1,
            hashes: Default::default(),
        },
        resolution: Resolution::Resolved {
            project: ProviderProject {
                provider: "modrinth".to_owned(),
                project_id: "example".to_owned(),
                version_id: "1".to_owned(),
            },
        },
        provider_aliases: Vec::new(),
        provider_checks: Vec::new(),
        required_dependencies: Vec::new(),
        automatic_dependency: false,
        cleanup_eligible: false,
    }
}

fn job(name: &str) -> ReconcileJob {
    ReconcileJob {
        instance: InstanceConfig {
            name: name.to_owned(),
            game_version: "1.21.1".to_owned(),
            loader: crate::instance::ModLoader::Fabric,
            loader_version: None,
            created: chrono::DateTime::UNIX_EPOCH,
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
        },
        instances_dir: PathBuf::new(),
        client: crate::net::HttpClient::new(),
    }
}

#[tokio::test]
async fn busy_manifest_defers_reconciliation_without_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let instances_dir = temp.path().join("instances");
    let paths = crate::storage::InstancePaths::new(instances_dir.join("Busy"));
    let path = paths.minecraft().join("mods/example.jar");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"example").unwrap();
    let mut record = resolved_record();
    record.relative_path = PathBuf::from("mods/example.jar");
    record.fingerprint = fingerprint(&path).unwrap();
    ContentManifest {
        version: 1,
        files: vec![record],
    }
    .save(&paths.content_manifest())
    .unwrap();
    // Hold the manifest lock the way an install commit does.
    let _lock = ContentLock::acquire(&paths.content_manifest()).unwrap();
    assert!(
        reconcile_inventory(
            &paths.content_manifest(),
            &paths.minecraft(),
            24,
            512,
            &NoopProgress,
        )
        .is_err()
    );

    let task = crate::feedback::progress::ProgressTask::start("test reconcile");
    let result = super::reconcile(
        job("Busy").instance,
        instances_dir,
        crate::net::HttpClient::new(),
        &task,
        |_| {},
    )
    .await;

    assert!(result.complete);
    assert!(result.error.is_none());
    assert_eq!(result.manifest.files.len(), 1);
}

#[test]
fn coordinator_queues_instances_once_and_preserves_order() {
    let mut coordinator = ReconcileCoordinator::default();
    let one = ("one".to_owned(), chrono::DateTime::UNIX_EPOCH);
    assert!(coordinator.enqueue(job("one"), false));
    assert!(!coordinator.enqueue(job("two"), false));
    assert!(!coordinator.enqueue(job("one"), false));
    assert!(!coordinator.rerun.contains(&one));
    assert!(!coordinator.enqueue(job("one"), true));
    assert_eq!(coordinator.queue.len(), 2);
    assert!(coordinator.rerun.contains(&one));
    assert_eq!(coordinator.queue.pop_front().unwrap().instance.name, "one");
    assert_eq!(coordinator.queue.pop_front().unwrap().instance.name, "two");
}

#[test]
fn coordinator_does_not_merge_recreated_instances_with_the_same_name() {
    let mut coordinator = ReconcileCoordinator::default();
    let first = job("same");
    let mut recreated = job("same");
    recreated.instance.created += chrono::TimeDelta::seconds(1);

    assert!(coordinator.enqueue(first, false));
    assert!(!coordinator.enqueue(recreated, false));
    assert_eq!(coordinator.queue.len(), 2);
}

#[test]
fn oversized_content_is_kept_without_hashing_or_provider_query() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let resource_packs = minecraft.join("resourcepacks");
    std::fs::create_dir_all(&resource_packs).unwrap();
    let pack = resource_packs.join("large.zip");
    let file = std::fs::File::create(&pack).unwrap();
    file.set_len(2 * 1024 * 1024).unwrap();
    let manifest_path = temp.path().join("manifest.json");

    let inventory = reconcile_inventory(&manifest_path, &minecraft, 24, 1, &NoopProgress).unwrap();

    assert!(inventory.queries.is_empty());
    assert_eq!(inventory.manifest.files.len(), 1);
    assert!(inventory.manifest.files[0].fingerprint.hashes.is_empty());
    assert!(matches!(
        inventory.manifest.files[0].resolution,
        Resolution::Unmatched { .. }
    ));
}

#[test]
fn unchanged_saved_index_reuses_fingerprint_and_skips_provider_query() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let mods = minecraft.join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    std::fs::write(mods.join("example.jar"), b"example").unwrap();
    let manifest_path = temp.path().join("manifest.json");

    let mut first =
        reconcile_inventory(&manifest_path, &minecraft, 24, 512, &NoopProgress).unwrap();
    assert_eq!(first.queries.len(), 1);
    let fingerprint = first.manifest.files[0].fingerprint.clone();
    first.manifest.files[0].resolution = Resolution::Unmatched {
        checked_at: chrono::Utc::now().timestamp(),
        providers: vec!["modrinth".to_owned(), "curseforge".to_owned()],
    };
    first.manifest.save(&manifest_path).unwrap();

    let second = reconcile_inventory(&manifest_path, &minecraft, 24, 512, &NoopProgress).unwrap();
    assert!(second.queries.is_empty());
    assert_eq!(second.manifest.files[0].fingerprint, fingerprint);
}

#[test]
fn newly_configured_provider_retries_an_unmatched_record() {
    let mut record = resolved_record();

    assert!(provider_was_not_checked(&record, "curseforge"));
    assert!(!provider_was_not_checked(&record, "modrinth"));
    record.provider_checks.push("curseforge".to_owned());
    assert!(!provider_was_not_checked(&record, "curseforge"));
}

#[test]
fn reconciliation_keeps_newer_provider_ownership_for_unchanged_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let manifest_path = temp.path().join("manifest.json");
    let path = minecraft.join("mods/example.jar");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"original").unwrap();
    let mut record = resolved_record();
    record.fingerprint = fingerprint(&path).unwrap();
    let previous = ContentManifest {
        version: 1,
        files: vec![record.clone()],
    };
    let mut current = previous.clone();
    current.files[0].resolution = Resolution::Resolved {
        project: ProviderProject {
            provider: "curseforge".to_owned(),
            project_id: "chosen".to_owned(),
            version_id: "new".to_owned(),
        },
    };
    current.files[0].provider_aliases = vec![record.resolved_project().unwrap().clone()];
    current.save(&manifest_path).unwrap();
    let saved = save_reconciled_manifest(
        &manifest_path,
        &minecraft,
        previous.clone(),
        &previous,
        &NoopProgress,
    )
    .unwrap();
    assert_eq!(saved.files, current.files);
}

#[test]
fn reconciliation_revalidates_hashes_after_provider_matching() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let manifest_path = temp.path().join("manifest.json");
    let path = minecraft.join("mods/example.jar");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"original").unwrap();
    let mut inventory =
        reconcile_inventory(&manifest_path, &minecraft, 24, 512, &NoopProgress).unwrap();
    inventory.manifest.files[0].resolution = resolved_record().resolution;
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, b"external").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let saved = save_reconciled_manifest(
        &manifest_path,
        &minecraft,
        inventory.manifest,
        &inventory.previous,
        &NoopProgress,
    )
    .unwrap();
    assert!(saved.files.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), b"external");
}

#[test]
fn an_incomplete_directory_scan_does_not_publish_a_partial_inventory() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let manifest_path = temp.path().join("manifest.json");
    let pack = minecraft.join("resourcepacks/example.zip");
    std::fs::create_dir_all(pack.parent().unwrap()).unwrap();
    std::fs::write(&pack, b"pack").unwrap();
    let first = reconcile_inventory(&manifest_path, &minecraft, 24, 512, &NoopProgress).unwrap();
    first.manifest.save(&manifest_path).unwrap();
    let saved_bytes = std::fs::read(&manifest_path).unwrap();
    std::fs::write(minecraft.join("mods"), b"not a directory").unwrap();
    assert!(reconcile_inventory(&manifest_path, &minecraft, 24, 512, &NoopProgress).is_err());
    assert_eq!(std::fs::read(manifest_path).unwrap(), saved_bytes);
}

#[test]
fn reconciliation_saves_provider_aliases_for_an_existing_resolution() {
    let temp = tempfile::tempdir().unwrap();
    let manifest_path = temp.path().join("manifest.json");
    let minecraft = temp.path().join("minecraft");
    let mut record = resolved_record();
    let path = minecraft.join(&record.relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"original").unwrap();
    record.fingerprint = fingerprint(&path).unwrap();
    let current = ContentManifest {
        version: 1,
        files: vec![record.clone()],
    };
    current.save(&manifest_path).unwrap();

    let mut enriched = record;
    enriched.provider_aliases.push(ProviderProject {
        provider: "curseforge".to_owned(),
        project_id: "42".to_owned(),
        version_id: "84".to_owned(),
    });
    enriched.provider_checks = vec!["modrinth".to_owned(), "curseforge".to_owned()];
    let saved = save_reconciled_manifest(
        &manifest_path,
        &minecraft,
        ContentManifest {
            version: 1,
            files: vec![enriched.clone()],
        },
        &current,
        &NoopProgress,
    )
    .unwrap();

    assert_eq!(saved.files, vec![enriched]);
    let stale = saved.clone();
    let mut managed = saved;
    managed.files[0].automatic_dependency = true;
    managed.files[0].cleanup_eligible = true;
    managed.save(&manifest_path).unwrap();
    let merged = save_reconciled_manifest(
        &manifest_path,
        &minecraft,
        stale.clone(),
        &stale,
        &NoopProgress,
    )
    .unwrap();
    assert!(merged.files[0].automatic_dependency);
    std::fs::write(&path, b"replacement").unwrap();
    let mut newer = merged.clone();
    newer.files[0].fingerprint = fingerprint(&path).unwrap();
    newer.save(&manifest_path).unwrap();
    assert_eq!(
        save_reconciled_manifest(
            &manifest_path,
            &minecraft,
            stale.clone(),
            &stale,
            &NoopProgress
        )
        .unwrap()
        .files,
        newer.files
    );
    std::fs::remove_file(path).unwrap();
    assert!(
        save_reconciled_manifest(
            &manifest_path,
            &minecraft,
            stale.clone(),
            &stale,
            &NoopProgress
        )
        .unwrap()
        .files
        .is_empty()
    );
}

#[test]
fn directory_packs_are_indexed_without_provider_queries() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let pack = minecraft.join("resourcepacks/example");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(pack.join("pack.mcmeta"), b"{}").unwrap();

    let inventory = reconcile_inventory(
        &temp.path().join("manifest.json"),
        &minecraft,
        24,
        512,
        &NoopProgress,
    )
    .unwrap();

    assert!(inventory.queries.is_empty());
    assert_eq!(
        inventory.manifest.files[0].relative_path,
        PathBuf::from("resourcepacks/example")
    );
}

#[test]
fn datapacks_are_indexed_under_their_world() {
    let temp = tempfile::tempdir().unwrap();
    let minecraft = temp.path().join("minecraft");
    let datapacks = minecraft.join("saves/world/datapacks");
    std::fs::create_dir_all(datapacks.join("folder-pack")).unwrap();
    std::fs::write(datapacks.join("zipped-pack.zip"), b"zip").unwrap();

    let inventory = reconcile_inventory(
        &temp.path().join("manifest.json"),
        &minecraft,
        24,
        512,
        &NoopProgress,
    )
    .unwrap();

    assert_eq!(inventory.manifest.files.len(), 2);
    assert!(inventory.manifest.files.iter().all(|record| {
        record.kind == ContentKind::DataPack
            && record.relative_path.starts_with("saves/world/datapacks")
    }));
    assert_eq!(inventory.queries.len(), 1);
}
