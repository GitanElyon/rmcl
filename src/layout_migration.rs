// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage::{InstancePaths, LAYOUT_VERSION, MetadataPaths};

const LEGACY_MINECRAFT: &str = ".minecraft";
const LEGACY_STATE: &str = ".rmcl";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProgress {
    pub phase: String,
    pub item: String,
    pub current: u64,
    pub total: u64,
    pub item_current: Option<u64>,
    pub item_total: Option<u64>,
    pub backup_dir: Option<PathBuf>,
}

impl MigrationProgress {
    fn new(phase: impl Into<String>, item: impl Into<String>, current: u64, total: u64) -> Self {
        Self {
            phase: phase.into(),
            item: item.into(),
            current,
            total,
            item_current: None,
            item_total: None,
            backup_dir: None,
        }
    }

    fn with_item_progress(mut self, current: u64, total: u64) -> Self {
        self.item_current = Some(current);
        self.item_total = Some(total);
        self
    }

    fn with_backup(mut self, backup_dir: &Path) -> Self {
        self.backup_dir = Some(backup_dir.to_owned());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MigrationJournal {
    version: u32,
    backup_dir: PathBuf,
    completed: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LayoutMarker {
    version: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Cannot migrate {instance}: both {old} and {new} exist")]
    PathConflict {
        instance: String,
        old: String,
        new: String,
    },
    #[error(
        "Cannot safely merge migration data from {old} into {new}: conflicting contents or incompatible paths"
    )]
    MergeConflict { old: String, new: String },
    #[error("Migration backup would be created inside the data being backed up: {0}")]
    BackupOverlap(String),
    #[error(
        "Not enough free space for migration backup: need {required} bytes, have {available} bytes"
    )]
    InsufficientSpace { required: u64, available: u64 },
}

pub fn is_needed(instances_dir: &Path, meta_dir: &Path) -> bool {
    let metadata = MetadataPaths::new(meta_dir);
    metadata.cache_rebuild_pending().exists()
        || metadata.migration_journal().exists()
        || metadata.state().join("migration-v2.json").exists()
        || has_legacy_instances(instances_dir)
        || has_legacy_shared_data(meta_dir)
}

fn has_legacy_shared_data(meta_dir: &Path) -> bool {
    [
        "versions",
        "libraries",
        "assets",
        "loader-profiles",
        "config-sync",
    ]
    .iter()
    .any(|path| meta_dir.join(path).exists())
}

pub fn initialize_new_layout(meta_dir: &Path) -> Result<(), MigrationError> {
    let metadata = MetadataPaths::new(meta_dir);
    for directory in [
        metadata.profiles(),
        metadata.backups(),
        metadata.versions(),
        metadata.libraries(),
        metadata.assets(),
        metadata.loader_profiles(),
        metadata.provider_projects("modrinth"),
        metadata.provider_versions("modrinth"),
        metadata.provider_icons("modrinth"),
        metadata.temporary(),
    ] {
        fs::create_dir_all(directory)?;
    }
    write_json_atomic(
        &metadata.layout_marker(),
        &LayoutMarker {
            version: LAYOUT_VERSION,
        },
    )
}

pub fn run(
    instances_dir: &Path,
    meta_dir: &Path,
    config_file: &Path,
    mut report: impl FnMut(MigrationProgress),
) -> Result<PathBuf, MigrationError> {
    fs::create_dir_all(instances_dir)?;
    fs::create_dir_all(meta_dir)?;
    let metadata = MetadataPaths::new(meta_dir);
    if marker_version(&metadata.layout_marker()) == Some(LAYOUT_VERSION)
        && metadata.cache_rebuild_pending().exists()
        && !metadata.migration_journal().exists()
        && !metadata.state().join("migration-v2.json").exists()
        && !has_legacy_instances(instances_dir)
        && !has_legacy_shared_data(meta_dir)
    {
        crate::config::upgrade_config_file(config_file)?;
        return Ok(latest_layout_backup(&metadata).unwrap_or_else(|| metadata.backups()));
    }
    if !is_needed(instances_dir, meta_dir) {
        crate::config::upgrade_config_file(config_file)?;
        initialize_new_layout(meta_dir)?;
        return Ok(metadata.backups());
    }
    fs::create_dir_all(metadata.state())?;
    fs::write(metadata.cache_rebuild_pending(), LAYOUT_VERSION.to_string())?;

    let mut journal = load_or_create_journal(&metadata)?;
    let instances = instance_directories(instances_dir)?;
    let total = instances.len() as u64 + 8;
    let mut current = journal.completed.len() as u64;

    if !is_complete(&journal, "backup") || !journal.backup_dir.exists() {
        let backup_dir = journal.backup_dir.clone();
        backup_user_data(
            instances_dir,
            config_file,
            meta_dir,
            &backup_dir,
            |copied, bytes, path| {
                report(
                    MigrationProgress::new(
                        "Backing up user data",
                        path.display().to_string(),
                        current,
                        total,
                    )
                    .with_item_progress(copied, bytes)
                    .with_backup(&backup_dir),
                );
            },
        )?;
        complete(&mut journal, &metadata, "backup")?;
        current = journal.completed.len() as u64;
    }

    if !is_complete(&journal, "config") {
        report(
            MigrationProgress::new(
                "Upgrading launcher config",
                config_file.display().to_string(),
                current,
                total,
            )
            .with_backup(&journal.backup_dir),
        );
        crate::config::upgrade_config_file(config_file)?;
        complete(&mut journal, &metadata, "config")?;
        current = journal.completed.len() as u64;
    }

    validate_migration_conflicts(&instances, meta_dir)?;

    for instance in &instances {
        let name = instance
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("instance")
            .to_owned();
        let key = format!("instance:{name}");
        if is_complete(&journal, &key) && !has_legacy_instance_data(instance) {
            continue;
        }
        report(
            MigrationProgress::new("Migrating instances", name.clone(), current, total)
                .with_backup(&journal.backup_dir),
        );
        migrate_instance(instance, &name)?;
        complete(&mut journal, &metadata, &key)?;
        current = journal.completed.len() as u64;
    }

    for (key, source, destination) in shared_moves(meta_dir) {
        let journal_key = format!("shared:{key}");
        if is_complete(&journal, &journal_key) && !source.exists() {
            continue;
        }
        report(
            MigrationProgress::new("Migrating shared data", key, current, total)
                .with_backup(&journal.backup_dir),
        );
        let conflicts =
            (key != "profiles").then(|| journal.backup_dir.join("cache-conflicts").join(key));
        move_or_merge(&source, &destination, conflicts.as_deref())?;
        complete(&mut journal, &metadata, &journal_key)?;
        current = journal.completed.len() as u64;
    }
    let legacy_config_sync = meta_dir.join("config-sync");
    if legacy_config_sync.exists() && fs::read_dir(&legacy_config_sync)?.next().is_none() {
        fs::remove_dir(legacy_config_sync)?;
    }

    report(
        MigrationProgress::new(
            "Finalizing migration",
            "Writing layout marker",
            total.saturating_sub(1),
            total,
        )
        .with_backup(&journal.backup_dir),
    );
    initialize_new_layout(meta_dir)?;
    for journal_path in [
        metadata.migration_journal(),
        metadata.state().join("migration-v2.json"),
    ] {
        if journal_path.exists() {
            fs::remove_file(journal_path)?;
        }
    }
    report(
        MigrationProgress::new("Migration complete", "Layout updated", total, total)
            .with_backup(&journal.backup_dir),
    );
    Ok(journal.backup_dir)
}

pub fn cache_rebuild_pending(meta_dir: &Path) -> bool {
    MetadataPaths::new(meta_dir)
        .cache_rebuild_pending()
        .exists()
}

pub fn finish_cache_rebuild(meta_dir: &Path) -> Result<(), MigrationError> {
    let marker = MetadataPaths::new(meta_dir).cache_rebuild_pending();
    if marker.exists() {
        fs::remove_file(marker)?;
    }
    Ok(())
}

fn migrate_instance(instance: &Path, name: &str) -> Result<(), MigrationError> {
    let paths = InstancePaths::new(instance);
    rename_visible_directory(instance, LEGACY_MINECRAFT, paths.minecraft(), name)?;
    rename_visible_directory(instance, LEGACY_STATE, paths.state(), name)?;

    let old_config = paths.state().join("config-sync").join("local-config");
    move_or_merge(&old_config, &paths.local_config(), None)?;
    let old_config_root = paths.state().join("config-sync");
    if old_config_root.exists() && fs::read_dir(&old_config_root)?.next().is_none() {
        fs::remove_dir(old_config_root)?;
    }
    fs::create_dir_all(paths.content())?;
    Ok(())
}

fn rename_visible_directory(
    instance: &Path,
    legacy_name: &str,
    destination: PathBuf,
    instance_name: &str,
) -> Result<(), MigrationError> {
    let source = instance.join(legacy_name);
    if path_exists(&source)?
        && path_exists(&destination)?
        && (!fs::symlink_metadata(&source)?.is_dir()
            || !fs::symlink_metadata(&destination)?.is_dir())
    {
        return Err(MigrationError::PathConflict {
            instance: instance_name.to_owned(),
            old: source.display().to_string(),
            new: destination.display().to_string(),
        });
    }
    move_or_merge(&source, &destination, None)
}

fn backup_user_data(
    instances_dir: &Path,
    config_file: &Path,
    meta_dir: &Path,
    backup_dir: &Path,
    mut report: impl FnMut(u64, u64, &Path),
) -> Result<(), MigrationError> {
    if backup_dir.exists() {
        return Ok(());
    }
    let legacy_profiles = meta_dir.join("config-sync").join("profiles");
    let current_profiles = MetadataPaths::new(meta_dir).profiles();
    let total = tree_size(instances_dir)?
        .saturating_add(file_size(config_file)?)
        .saturating_add(tree_size(&legacy_profiles)?)
        .saturating_add(tree_size(&current_profiles)?);
    validate_backup_destination(instances_dir, backup_dir, total)?;
    let partial = backup_dir.with_extension("partial");
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    let mut copied = 0;
    report(copied, total, instances_dir);
    fs::create_dir_all(&partial)?;
    copy_dir_recursive_with_progress(
        instances_dir,
        &partial.join("instances"),
        &mut |bytes, path| {
            copied = copied.saturating_add(bytes);
            report(copied, total, path);
        },
    )?;
    if config_file.exists() {
        let config_backup = partial.join("config");
        fs::create_dir_all(&config_backup)?;
        let bytes = fs::copy(config_file, config_backup.join("config.toml"))?;
        copied = copied.saturating_add(bytes);
        report(copied, total, config_file);
    }
    if legacy_profiles.exists() {
        copy_dir_recursive_with_progress(
            &legacy_profiles,
            &partial.join("profiles/legacy"),
            &mut |bytes, path| {
                copied = copied.saturating_add(bytes);
                report(copied, total, path);
            },
        )?;
    }
    if current_profiles.exists() {
        copy_dir_recursive_with_progress(
            &current_profiles,
            &partial.join("profiles/current"),
            &mut |bytes, path| {
                copied = copied.saturating_add(bytes);
                report(copied, total, path);
            },
        )?;
    }
    fs::rename(partial, backup_dir)?;
    Ok(())
}

fn validate_migration_conflicts(
    instances: &[PathBuf],
    meta_dir: &Path,
) -> Result<(), MigrationError> {
    for instance in instances {
        let name = instance
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("instance");
        let paths = InstancePaths::new(instance);
        for (legacy, destination) in [
            (instance.join(LEGACY_MINECRAFT), paths.minecraft()),
            (instance.join(LEGACY_STATE), paths.state()),
        ] {
            if path_exists(&legacy)?
                && path_exists(&destination)?
                && (!fs::symlink_metadata(&legacy)?.is_dir()
                    || !fs::symlink_metadata(&destination)?.is_dir())
            {
                return Err(MigrationError::PathConflict {
                    instance: name.to_owned(),
                    old: legacy.display().to_string(),
                    new: destination.display().to_string(),
                });
            }
            validate_merge(&legacy, &destination, false)?;
        }
        for state in [instance.join(LEGACY_STATE), paths.state()] {
            for destination in [
                instance.join(LEGACY_STATE).join("content/config"),
                paths.local_config(),
            ] {
                validate_merge(&state.join("config-sync/local-config"), &destination, false)?;
            }
        }
    }
    for (key, source, destination) in shared_moves(meta_dir) {
        validate_merge(&source, &destination, key != "profiles")?;
    }
    Ok(())
}

fn shared_moves(meta_dir: &Path) -> [(&'static str, PathBuf, PathBuf); 5] {
    let metadata = MetadataPaths::new(meta_dir);
    [
        (
            "profiles",
            meta_dir.join("config-sync").join("profiles"),
            metadata.profiles(),
        ),
        ("versions", meta_dir.join("versions"), metadata.versions()),
        (
            "libraries",
            meta_dir.join("libraries"),
            metadata.libraries(),
        ),
        ("assets", meta_dir.join("assets"), metadata.assets()),
        (
            "loader-profiles",
            meta_dir.join("loader-profiles"),
            metadata.loader_profiles(),
        ),
    ]
}

pub(crate) fn validate_merge(
    source: &Path,
    destination: &Path,
    archive_cache_conflicts: bool,
) -> Result<(), MigrationError> {
    if !path_exists(source)? || !path_exists(destination)? {
        return Ok(());
    }
    validate_merge_roots(source, destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if !path_exists(&target)? {
            continue;
        }
        if entry.file_type()?.is_dir() && fs::symlink_metadata(&target)?.is_dir() {
            validate_merge(&entry.path(), &target, archive_cache_conflicts)?;
        } else if !files_identical(&entry.path(), &target)?
            && !(archive_cache_conflicts && distinct_regular_files(&entry.path(), &target)?)
        {
            return Err(merge_conflict(&entry.path(), &target));
        }
    }
    Ok(())
}

fn validate_backup_destination(
    instances_dir: &Path,
    backup_dir: &Path,
    required: u64,
) -> Result<(), MigrationError> {
    let instances = instances_dir.canonicalize()?;
    let backup_parent = backup_dir
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "backup has no parent"))?;
    fs::create_dir_all(backup_parent)?;
    let backup_parent = backup_parent.canonicalize()?;
    let backup_name = backup_dir
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "backup has no name"))?;
    let canonical_backup = backup_parent.join(backup_name);
    if canonical_backup.starts_with(&instances) {
        return Err(MigrationError::BackupOverlap(
            canonical_backup.display().to_string(),
        ));
    }
    let available = fs2::available_space(&backup_parent)?;
    let margin = required / 20;
    let required_with_margin = required
        .saturating_add(margin)
        .saturating_add(16 * 1024 * 1024);
    if available < required_with_margin {
        return Err(MigrationError::InsufficientSpace {
            required: required_with_margin,
            available,
        });
    }
    Ok(())
}

fn tree_size(path: &Path) -> io::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            total = total.saturating_add(tree_size(&entry.path())?);
        } else if file_type.is_file() {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

fn file_size(path: &Path) -> io::Result<u64> {
    if path.exists() {
        Ok(fs::metadata(path)?.len())
    } else {
        Ok(0)
    }
}

pub(crate) fn move_or_merge(
    source: &Path,
    destination: &Path,
    cache_conflicts: Option<&Path>,
) -> Result<(), MigrationError> {
    if !path_exists(source)? {
        return Ok(());
    }
    if !path_exists(destination)? {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::rename(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {}
            Err(error) => return Err(error.into()),
        }
    }
    merge_dir_without_overwrite(source, destination, cache_conflicts)?;
    fs::remove_dir(source)?;
    Ok(())
}

fn merge_dir_without_overwrite(
    source: &Path,
    destination: &Path,
    cache_conflicts: Option<&Path>,
) -> Result<(), MigrationError> {
    let new_directory = !path_exists(destination)?;
    fs::create_dir_all(destination)?;
    validate_merge_roots(source, destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        let target_exists = path_exists(&target)?;
        if file_type.is_dir() && (!target_exists || fs::symlink_metadata(&target)?.is_dir()) {
            move_or_merge(
                &entry.path(),
                &target,
                cache_conflicts
                    .map(|path| path.join(entry.file_name()))
                    .as_deref(),
            )?;
        } else if !target_exists {
            move_file(&entry.path(), &target)?;
        } else if files_identical(&entry.path(), &target)? {
            fs::remove_file(entry.path())?;
        } else if let Some(conflicts) = cache_conflicts
            && distinct_regular_files(&entry.path(), &target)?
        {
            fs::create_dir_all(conflicts)?;
            let mut archived = conflicts.join(entry.file_name());
            let mut suffix = 0;
            while path_exists(&archived)? && !files_identical(&entry.path(), &archived)? {
                suffix += 1;
                let mut name = entry.file_name();
                name.push(format!(".{suffix}"));
                archived = conflicts.join(name);
            }
            if path_exists(&archived)? {
                fs::remove_file(entry.path())?;
            } else {
                move_file(&entry.path(), &archived)?;
            }
            tracing::warn!(
                "Kept current cache {}; archived conflicting legacy copy at {}",
                target.display(),
                archived.display()
            );
        } else {
            return Err(merge_conflict(&entry.path(), &target));
        }
    }
    if new_directory {
        fs::set_permissions(destination, fs::metadata(source)?.permissions())?;
    }
    Ok(())
}

fn path_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn merge_conflict(source: &Path, destination: &Path) -> MigrationError {
    MigrationError::MergeConflict {
        old: source.display().to_string(),
        new: destination.display().to_string(),
    }
}

fn validate_merge_roots(source: &Path, destination: &Path) -> Result<(), MigrationError> {
    if !fs::symlink_metadata(source)?.is_dir() || !fs::symlink_metadata(destination)?.is_dir() {
        return Err(merge_conflict(source, destination));
    }
    let source_path = source.canonicalize()?;
    let destination_path = destination.canonicalize()?;
    if source_path.starts_with(&destination_path) || destination_path.starts_with(&source_path) {
        return Err(merge_conflict(source, destination));
    }
    Ok(())
}

fn move_file(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            copy_file_then_remove(source, destination)
        }
        Err(error) => Err(error),
    }
}

fn copy_file_then_remove(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.is_symlink() {
        crate::storage::copy_symlink(source, destination)?;
    } else if metadata.is_file() {
        let mut input = fs::File::open(source)?;
        let mut output = tempfile::NamedTempFile::new_in(
            destination
                .parent()
                .ok_or_else(|| io::Error::other("migration destination has no parent"))?,
        )?;
        io::copy(&mut input, &mut output)?;
        output.as_file().set_permissions(metadata.permissions())?;
        output.as_file().sync_all()?;
        output
            .persist_noclobber(destination)
            .map_err(|error| error.error)?;
    } else {
        return Err(io::Error::other(
            "migration source is not a regular file or symlink",
        ));
    }
    fs::remove_file(source)
}

fn distinct_regular_files(source: &Path, destination: &Path) -> io::Result<bool> {
    Ok(fs::symlink_metadata(source)?.is_file()
        && fs::symlink_metadata(destination)?.is_file()
        && source.canonicalize()? != destination.canonicalize()?)
}

fn files_identical(source: &Path, destination: &Path) -> io::Result<bool> {
    let source_metadata = fs::symlink_metadata(source)?;
    let destination_metadata = fs::symlink_metadata(destination)?;
    if !source_metadata.is_file()
        || !destination_metadata.is_file()
        || source_metadata.len() != destination_metadata.len()
        || source.canonicalize()? == destination.canonicalize()?
    {
        return Ok(false);
    }
    let mut source = fs::File::open(source)?;
    let mut destination = fs::File::open(destination)?;
    let mut source_bytes = [0; 8192];
    let mut destination_bytes = [0; 8192];
    loop {
        let count = source.read(&mut source_bytes)?;
        destination.read_exact(&mut destination_bytes[..count])?;
        if source_bytes[..count] != destination_bytes[..count] {
            return Ok(false);
        }
        if count == 0 {
            return Ok(destination.read(&mut destination_bytes)? == 0);
        }
    }
}

fn copy_dir_recursive_with_progress(
    source: &Path,
    destination: &Path,
    report: &mut impl FnMut(u64, &Path),
) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_symlink() {
            crate::storage::copy_symlink(&entry.path(), &target)?;
        } else if file_type.is_dir() {
            copy_dir_recursive_with_progress(&entry.path(), &target, report)?;
        } else {
            let path = entry.path();
            let copied = fs::copy(&path, target)?;
            report(copied, &path);
        }
    }
    Ok(())
}

fn load_or_create_journal(metadata: &MetadataPaths) -> Result<MigrationJournal, MigrationError> {
    let legacy_journal = metadata.state().join("migration-v2.json");
    if metadata.migration_journal().exists() {
        return Ok(serde_json::from_slice(&fs::read(
            metadata.migration_journal(),
        )?)?);
    }
    if legacy_journal.exists() {
        let journal = serde_json::from_slice(&fs::read(&legacy_journal)?)?;
        write_json_atomic(&metadata.migration_journal(), &journal)?;
        fs::remove_file(legacy_journal)?;
        return Ok(journal);
    }
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%f").to_string();
    let journal = MigrationJournal {
        version: LAYOUT_VERSION,
        backup_dir: metadata.backups().join(format!("backup-{timestamp}")),
        completed: Vec::new(),
    };
    write_json_atomic(&metadata.migration_journal(), &journal)?;
    Ok(journal)
}

fn latest_layout_backup(metadata: &MetadataPaths) -> Option<PathBuf> {
    let mut backups = fs::read_dir(metadata.backups())
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            (file_type.is_dir() && (name.starts_with("backup-") || name.starts_with("layout-v2-")))
                .then(|| entry.path())
        })
        .collect::<Vec<_>>();
    backups.sort();
    backups.pop()
}

fn complete(
    journal: &mut MigrationJournal,
    metadata: &MetadataPaths,
    key: &str,
) -> Result<(), MigrationError> {
    if is_complete(journal, key) {
        return Ok(());
    }
    journal.completed.push(key.to_owned());
    write_json_atomic(&metadata.migration_journal(), journal)
}

fn is_complete(journal: &MigrationJournal, key: &str) -> bool {
    journal.completed.iter().any(|completed| completed == key)
}

fn instance_directories(instances_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut instances = fs::read_dir(instances_dir)?
        .flatten()
        .filter_map(|entry| entry.file_type().ok()?.is_dir().then(|| entry.path()))
        .collect::<Vec<_>>();
    instances.sort();
    Ok(instances)
}

fn has_legacy_instances(instances_dir: &Path) -> bool {
    instance_directories(instances_dir).is_ok_and(|instances| {
        instances
            .iter()
            .any(|instance| has_legacy_instance_data(instance))
    })
}

fn has_legacy_instance_data(instance: &Path) -> bool {
    [
        instance.join(LEGACY_MINECRAFT),
        instance.join(LEGACY_STATE),
        InstancePaths::new(instance)
            .state()
            .join("config-sync/local-config"),
    ]
    .iter()
    .any(|path| path.exists())
}

fn marker_version(path: &Path) -> Option<u32> {
    serde_json::from_slice::<LayoutMarker>(&fs::read(path).ok()?)
        .ok()
        .map(|marker| marker.version)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), MigrationError> {
    crate::storage::write_atomic(path, &serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/layout_migration.rs"]
mod tests;
