// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::instance::content::manifest::ContentManifest;
use crate::instance::{InstanceConfig, InstanceManager, ProviderProject};
use crate::net::modrinth::VersionInfo;
use crate::storage::InstancePaths;

use super::ImportSummary;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackState {
    pub source: ProviderProject,
    #[serde(
        serialize_with = "serialize_owned_paths",
        deserialize_with = "deserialize_owned_paths"
    )]
    pub files: Vec<PathBuf>,
}

impl PackState {
    pub fn load(paths: &InstancePaths) -> Option<Self> {
        let path = paths.modpack_state();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(
                    "Could not read pack ownership '{}': {error}",
                    path.display()
                );
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::warn!("Invalid pack ownership '{}': {error}", path.display());
                None
            }
        }
    }

    pub fn save(&self, paths: &InstancePaths) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        crate::storage::write_atomic(&paths.modpack_state(), &bytes)
            .map_err(|error| error.to_string())
    }
}

fn serialize_owned_paths<S: serde::Serializer>(
    paths: &[PathBuf],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let paths = paths
        .iter()
        .map(|path| {
            path.to_str()
                .ok_or_else(|| "Pack-owned path is not UTF-8".to_owned())
                .and_then(super::portable_pack_path)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(serde::ser::Error::custom)?;
    paths.serialize(serializer)
}

fn deserialize_owned_paths<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<PathBuf>, D::Error> {
    Vec::<String>::deserialize(deserializer)?
        .into_iter()
        .map(|path| {
            super::portable_pack_path(&path)
                .map(PathBuf::from)
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

struct RefreshStaging {
    // Field order closes the lease before TempDir cleanup, also on cancellation.
    _lease: std::fs::File,
    directory: tempfile::TempDir,
}

impl RefreshStaging {
    fn new(instances_dir: &Path) -> std::io::Result<Self> {
        // Recovery cannot see a new directory before its lease is installed.
        let _creation_lock = lock_refresh_recovery(instances_dir)?;
        let directory = tempfile::Builder::new()
            .prefix(".rmcl-refresh-")
            .tempdir_in(instances_dir)?;
        let lease = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join(".lease"))?;
        lease.lock()?;
        Ok(Self {
            _lease: lease,
            directory,
        })
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }
}

fn lock_refresh_recovery(instances_dir: &Path) -> std::io::Result<std::fs::File> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(instances_dir.join(".rmcl-refresh.lock"))?;
    lock.lock()?;
    Ok(lock)
}

pub struct RefreshPlan {
    pub instance: InstanceConfig,
    pub summary: ImportSummary,
    pub current_version: String,
    pub target_version: String,
    pub conflicts: Vec<PathBuf>,
    staging: RefreshStaging,
    staged_instance: PathBuf,
    old_owned: HashSet<PathBuf>,
    new_owned: HashSet<PathBuf>,
    previous_source: Option<ProviderProject>,
    meta_dir: PathBuf,
}

pub async fn prepare(
    manager: &InstanceManager,
    instance: &InstanceConfig,
    target: VersionInfo,
) -> Result<RefreshPlan, String> {
    crate::instance::manager::validate_name(&instance.name).map_err(|error| error.to_string())?;
    if crate::instance::runtime::is_active(&instance.name) {
        return Err("Stop the instance before changing its modpack".to_owned());
    }
    let source = instance
        .modpack_source
        .clone()
        .ok_or_else(|| "This instance is not linked to a modpack provider".to_owned())?;
    let registry = crate::instance::content::provider::ProviderRegistry::configured(
        crate::net::HttpClient::new(),
    );
    let provider = registry
        .get(&source.provider)
        .ok_or_else(|| format!("{} content provider is unavailable", source.provider))?;
    let current = provider
        .version(&source.version_id)
        .await
        .map_err(|error| error.to_string())?;
    let live = manager.instances_dir.join(&instance.name);
    let needed = directory_size(&live)
        .saturating_add(
            target
                .files
                .iter()
                .fold(0u64, |size, file| size.saturating_add(file.size)),
        )
        .saturating_add(64 * 1024 * 1024);
    let available = fs2::available_space(&manager.instances_dir).map_err(|e| e.to_string())?;
    if available < needed {
        return Err(format!(
            "Not enough free space to stage this modpack change (need about {}, available {})",
            format_bytes(needed),
            format_bytes(available)
        ));
    }

    let staging = RefreshStaging::new(&manager.instances_dir).map_err(|error| error.to_string())?;
    let archives = staging.path().join("archives");
    let summary = super::download_provider_summary(&source, &target, &archives.join("target"))
        .await
        .map_err(|error| error.to_string())?;
    let old_owned =
        match PackState::load(&InstancePaths::new(&live)).filter(|state| state.source == source) {
            Some(state) => state.files.into_iter().collect(),
            None => reconstruct_owned_files(&source, &current, &archives.join("current")).await?,
        };

    let staging_manager = InstanceManager::new(staging.path().join("instances"), &manager.meta_dir);
    let staged_config = super::execute_import(&summary, &staging_manager)
        .await
        .map_err(|error| error.to_string())?;
    let imported = staging_manager.instances_dir.join(&staged_config.name);
    let new_owned = PackState::load(&InstancePaths::new(&imported))
        .ok_or_else(|| "The staged pack did not record its owned files".to_owned())?
        .files
        .into_iter()
        .collect::<HashSet<_>>();
    let staged_instance = staging_manager.instances_dir.join(&instance.name);
    if imported != staged_instance {
        let aliases_source = crate::instance::manager::paths_alias(&imported, &staged_instance)
            .map_err(|error| error.to_string())?;
        crate::instance::manager::rename_path(&imported, &staged_instance, aliases_source)
            .map_err(|error| error.to_string())?;
    }

    let mut config = instance.clone();
    config.game_version = summary.game_version.clone();
    config.loader = summary.loader;
    config.loader_version = summary.loader_version.clone();
    config.modpack_source = summary.source.clone();
    staging_manager
        .save(&config)
        .map_err(|error| error.to_string())?;

    let conflicts = user_file_collisions(
        &InstancePaths::new(&live).minecraft(),
        &old_owned,
        &new_owned,
    )?;
    Ok(RefreshPlan {
        instance: config,
        current_version: current.version_number,
        target_version: target.version_number.clone(),
        summary,
        conflicts,
        staging,
        staged_instance,
        old_owned,
        new_owned,
        previous_source: instance.modpack_source.clone(),
        meta_dir: manager.meta_dir.clone(),
    })
}

pub fn apply(
    plan: RefreshPlan,
    replace_conflicts: &HashSet<PathBuf>,
) -> Result<InstanceConfig, String> {
    crate::instance::manager::validate_name(&plan.instance.name)
        .map_err(|error| error.to_string())?;
    if crate::instance::runtime::is_active(&plan.instance.name) {
        return Err("Stop the instance before changing its modpack".to_owned());
    }
    let live = plan
        .staging
        .path()
        .parent()
        .ok_or_else(|| "Invalid refresh staging path".to_owned())?
        .join(&plan.instance.name);
    let _instance_lock = crate::instance::runtime::lock_instance(
        live.parent().ok_or("Invalid instance root")?,
        &plan.instance.name,
    )
    .map_err(|error| error.to_string())?;
    let manager = InstanceManager::new(
        live.parent().ok_or("Invalid instance root")?,
        &plan.meta_dir,
    );
    let _config_lock = manager
        .config_lock(&plan.instance.name)
        .map_err(|error| error.to_string())?;
    let mut config = manager
        .load_one(&plan.instance.name)
        .map_err(|error| error.to_string())?;
    if config.created != plan.instance.created || config.modpack_source != plan.previous_source {
        return Err("The instance or its modpack changed while this update was being reviewed; prepare the update again".to_owned());
    }
    config.game_version.clone_from(&plan.instance.game_version);
    config.loader = plan.instance.loader;
    config
        .loader_version
        .clone_from(&plan.instance.loader_version);
    config
        .modpack_source
        .clone_from(&plan.instance.modpack_source);
    crate::storage::write_atomic(
        &plan.staged_instance.join("instance.json"),
        &serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let old_minecraft = InstancePaths::new(&live).minecraft();
    let staged_minecraft = InstancePaths::new(&plan.staged_instance).minecraft();
    let old_owned = preserve_disabled_pack_files(
        &live,
        &plan.staged_instance,
        &plan.old_owned,
        &plan.new_owned,
    )?;
    preserve_user_files(
        &old_minecraft,
        &staged_minecraft,
        &old_minecraft,
        &old_owned,
        &plan.new_owned,
        replace_conflicts,
    )?;
    preserve_refresh_state(
        &live,
        &plan.staged_instance,
        &old_owned,
        &plan.new_owned,
        replace_conflicts,
    )?;

    let backup = live.with_file_name(format!(".{}.rmcl-backup", plan.instance.name));
    match std::fs::symlink_metadata(&backup) {
        Ok(_) => {
            return Err(format!(
                "A previous update backup still exists at '{}'",
                backup.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    std::fs::rename(&live, &backup).map_err(|error| error.to_string())?;
    if let Err(error) = std::fs::rename(&plan.staged_instance, &live) {
        if let Err(rollback) = std::fs::rename(&backup, &live) {
            return Err(format!(
                "Could not activate the staged update: {error}; could not restore '{}' from '{}': {rollback}",
                live.display(),
                backup.display()
            ));
        }
        return Err(format!("Could not activate the staged update: {error}"));
    }
    if let Err(error) = std::fs::remove_dir_all(&backup) {
        tracing::warn!("Could not remove successful modpack update backup: {error}");
    }
    Ok(config)
}

async fn reconstruct_owned_files(
    source: &ProviderProject,
    version: &VersionInfo,
    temporary_dir: &Path,
) -> Result<HashSet<PathBuf>, String> {
    let summary = super::download_provider_summary(source, version, temporary_dir)
        .await
        .map_err(|error| error.to_string())?;
    Ok(super::owned_files(&summary).await?.into_iter().collect())
}

pub fn recover_interrupted(instances_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(instances_dir) else {
        return;
    };
    let _recovery_lock = match lock_refresh_recovery(instances_dir) {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!(
                "Could not lock refresh recovery '{}': {error}",
                instances_dir.display()
            );
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if name.starts_with(".rmcl-refresh-") && !name.ends_with(".rmcl-backup") {
            let lease_path = path.join(".lease");
            match std::fs::symlink_metadata(&lease_path) {
                Ok(metadata) if !metadata.is_file() => {
                    tracing::warn!("Invalid refresh lease '{}'", lease_path.display());
                    continue;
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    tracing::warn!(
                        "Could not inspect refresh lease '{}': {error}",
                        lease_path.display()
                    );
                    continue;
                }
                _ => {}
            }
            let lease = match std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lease_path)
            {
                Ok(lease) => lease,
                Err(error) => {
                    tracing::warn!(
                        "Could not open refresh lease '{}': {error}",
                        lease_path.display()
                    );
                    continue;
                }
            };
            match lease.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => continue,
                Err(error) => {
                    tracing::warn!(
                        "Could not lock refresh lease '{}': {error}",
                        lease_path.display()
                    );
                    continue;
                }
            }
            drop(lease);
            if let Err(error) = std::fs::remove_dir_all(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(
                    "Could not remove abandoned refresh staging '{}': {error}",
                    path.display()
                );
            }
            continue;
        }
        let Some(instance_name) = name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".rmcl-backup"))
        else {
            continue;
        };
        let _instance_lock =
            match crate::instance::runtime::lock_instance(instances_dir, instance_name) {
                Ok(lock) => lock,
                Err(crate::instance::manager::InstanceError::InstanceRunning(_)) => continue,
                Err(error) => {
                    tracing::warn!("Could not lock interrupted update '{instance_name}': {error}");
                    continue;
                }
            };
        let live = instances_dir.join(instance_name);
        if live.join("instance.json").is_file() {
            if let Err(error) = std::fs::remove_dir_all(&path) {
                tracing::warn!(
                    "Could not remove interrupted update backup '{}': {error}",
                    path.display()
                );
            }
        } else if let Err(error) = std::fs::rename(&path, &live) {
            tracing::warn!(
                "Could not restore interrupted modpack update backup '{}': {error}",
                path.display()
            );
        }
    }
}

fn user_file_collisions(
    minecraft: &Path,
    old_owned: &HashSet<PathBuf>,
    new_owned: &HashSet<PathBuf>,
) -> Result<Vec<PathBuf>, String> {
    let mut collisions = Vec::new();
    collect_files(minecraft, minecraft, &mut |relative, _| {
        if !old_owned.contains(relative) && new_owned.contains(relative) {
            collisions.push(relative.to_owned());
        }
        Ok(())
    })?;
    collisions.sort();
    Ok(collisions)
}

fn preserve_disabled_pack_files(
    live: &Path,
    staged: &Path,
    old_owned: &HashSet<PathBuf>,
    new_owned: &HashSet<PathBuf>,
) -> Result<HashSet<PathBuf>, String> {
    let old = InstancePaths::new(live);
    let new = InstancePaths::new(staged);
    let mut owned = old_owned.clone();
    let mut manifest = ContentManifest::load(&new.content_manifest()).map_err(|e| e.to_string())?;
    for path in old_owned {
        let Some(filename) = path.file_name() else {
            continue;
        };
        let mut disabled_name = filename.to_os_string();
        disabled_name.push(".disabled");
        let disabled = path.with_file_name(disabled_name);
        if !old.minecraft().join(path).exists() && old.minecraft().join(&disabled).is_file() {
            owned.insert(disabled.clone());
            if new_owned.contains(path) && new.minecraft().join(path).is_file() {
                std::fs::rename(new.minecraft().join(path), new.minecraft().join(&disabled))
                    .map_err(|e| e.to_string())?;
                manifest.rename_record(path, &disabled, false);
            }
        }
    }
    manifest
        .save(&new.content_manifest())
        .map_err(|e| e.to_string())?;
    Ok(owned)
}

fn preserve_refresh_state(
    live: &Path,
    staged: &Path,
    old_owned: &HashSet<PathBuf>,
    new_owned: &HashSet<PathBuf>,
    replace_conflicts: &HashSet<PathBuf>,
) -> Result<(), String> {
    let old = InstancePaths::new(live);
    let new = InstancePaths::new(staged);
    let baseline = old.state().join("config-sync.json");
    match std::fs::read(&baseline) {
        Ok(bytes) => crate::storage::write_atomic(&new.state().join("config-sync.json"), &bytes)
            .map_err(|error| error.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not preserve config sync baseline '{}': {error}",
                baseline.display()
            ));
        }
    }
    let local_config = old.local_config();
    if local_config.exists() {
        std::fs::create_dir_all(new.local_config()).map_err(|e| e.to_string())?;
        preserve_user_files(
            &local_config,
            &new.local_config(),
            &local_config,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )?;
    }

    let old_manifest = ContentManifest::load(&old.content_manifest()).map_err(|e| e.to_string())?;
    let mut new_manifest =
        ContentManifest::load(&new.content_manifest()).map_err(|e| e.to_string())?;
    for record in &old_manifest.files {
        let path = &record.relative_path;
        if !old_owned.contains(path)
            && !(new_owned.contains(path) && replace_conflicts.contains(path))
            && std::fs::symlink_metadata(old.minecraft().join(path)).is_ok()
        {
            new_manifest.upsert(record.clone());
        }
    }
    for path in new_owned {
        if !replace_conflicts.contains(path)
            && !old_owned.contains(path)
            && std::fs::symlink_metadata(old.minecraft().join(path)).is_ok()
            && old_manifest.record(path).is_none()
        {
            new_manifest.remove(path);
        }
    }
    new_manifest
        .save(&new.content_manifest())
        .map_err(|e| e.to_string())?;
    let mut state = PackState::load(&new)
        .ok_or_else(|| "The staged pack did not record its owned files".to_owned())?;
    state.files.retain(|path| {
        old_owned.contains(path)
            || replace_conflicts.contains(path)
            || std::fs::symlink_metadata(old.minecraft().join(path)).is_err()
    });
    state.save(&new)
}

fn preserve_user_files(
    source: &Path,
    destination: &Path,
    root: &Path,
    old_owned: &HashSet<PathBuf>,
    new_owned: &HashSet<PathBuf>,
    replace_conflicts: &HashSet<PathBuf>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if old_owned.contains(relative)
            || new_owned.contains(relative) && replace_conflicts.contains(relative)
        {
            continue;
        }
        if metadata.file_type().is_symlink() {
            let target = destination.join(entry.file_name());
            if let Ok(existing) = std::fs::symlink_metadata(&target) {
                if existing.is_dir() {
                    return Err(format!(
                        "Cannot replace directory '{}' with a symbolic link",
                        target.display()
                    ));
                }
                std::fs::remove_file(&target).map_err(|error| error.to_string())?;
            }
            crate::storage::copy_symlink(&path, &target).map_err(|error| error.to_string())?;
        } else if metadata.is_dir() {
            let target = destination.join(entry.file_name());
            std::fs::create_dir_all(&target).map_err(|error| error.to_string())?;
            preserve_user_files(
                &path,
                &target,
                root,
                old_owned,
                new_owned,
                replace_conflicts,
            )?;
        } else {
            let target = destination.join(entry.file_name());
            std::fs::copy(&path, target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn collect_files(
    directory: &Path,
    root: &Path,
    visit: &mut impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.is_dir() {
            collect_files(&path, root, visit)?;
        } else if metadata.is_file() || metadata.file_type().is_symlink() {
            visit(
                path.strip_prefix(root).map_err(|error| error.to_string())?,
                &path,
            )?;
        }
    }
    Ok(())
}

fn directory_size(path: &Path) -> u64 {
    let mut size = 0u64;
    let _ = collect_files(path, path, &mut |_, file| {
        size = size.saturating_add(
            std::fs::symlink_metadata(file)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        );
        Ok(())
    });
    size
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024 * 1024 * 1024) as f64)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024 * 1024) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::instance::content::manifest::{ContentFileRecord, ContentKind, FileFingerprint};

    #[test]
    fn refresh_rechecks_whether_the_instance_started_after_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let name = "refresh-became-active";
        let staging = RefreshStaging::new(temp.path()).unwrap();
        let stage_root = staging.path().to_owned();
        let live = temp.path().join(name);
        let staged_instance = stage_root.join("instances").join(name);
        std::fs::create_dir_all(live.join("minecraft")).unwrap();
        std::fs::create_dir_all(staged_instance.join("minecraft")).unwrap();
        let instance = serde_json::from_value(serde_json::json!({
            "name": name, "game_version": "1.21", "loader": "vanilla",
            "loader_version": null, "created": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let plan = RefreshPlan {
            instance,
            summary: ImportSummary {
                name: name.to_owned(),
                pack_version: "2".to_owned(),
                game_version: "1.21".to_owned(),
                loader: crate::instance::ModLoader::Vanilla,
                loader_version: None,
                mod_count: 0,
                override_count: 0,
                format: super::super::PackFormat::Mrpack,
                archive_path: stage_root.join("pack.mrpack"),
                source: None,
            },
            current_version: "1".to_owned(),
            target_version: "2".to_owned(),
            conflicts: Vec::new(),
            staging,
            staged_instance,
            old_owned: HashSet::new(),
            new_owned: HashSet::new(),
            previous_source: None,
            meta_dir: temp.path().join("meta"),
        };
        crate::instance::runtime::set_state(name, crate::instance::runtime::RunState::Running);
        let result = apply(plan, &HashSet::new());
        crate::instance::runtime::remove(name);

        assert!(result.unwrap_err().contains("Stop the instance"));
        assert!(live.join("minecraft").is_dir());
    }

    #[test]
    fn refresh_rejects_a_recreated_instance_without_moving_its_files() {
        let temp = tempfile::tempdir().unwrap();
        let name = "recreated";
        let manager = InstanceManager::new(temp.path(), temp.path().join("meta"));
        let staging = RefreshStaging::new(temp.path()).unwrap();
        let staged_instance = staging.path().join("instances").join(name);
        let live = manager.instances_dir.join(name);
        std::fs::create_dir_all(live.join("minecraft")).unwrap();
        std::fs::create_dir_all(staged_instance.join("minecraft")).unwrap();
        std::fs::write(live.join("minecraft/sentinel"), b"new instance").unwrap();
        let instance: InstanceConfig = serde_json::from_value(serde_json::json!({
            "name": name, "game_version": "1.21", "loader": "vanilla", "created": "2026-01-01T00:00:00Z"
        })).unwrap();
        let mut current = instance.clone();
        current.created += chrono::TimeDelta::seconds(1);
        manager.save(&current).unwrap();
        let plan = RefreshPlan {
            instance,
            summary: ImportSummary {
                name: name.to_owned(),
                pack_version: "2".to_owned(),
                game_version: "1.21".to_owned(),
                loader: crate::instance::ModLoader::Vanilla,
                loader_version: None,
                mod_count: 0,
                override_count: 0,
                format: super::super::PackFormat::Mrpack,
                archive_path: staging.path().join("pack.mrpack"),
                source: None,
            },
            current_version: "1".to_owned(),
            target_version: "2".to_owned(),
            conflicts: Vec::new(),
            staged_instance,
            staging,
            old_owned: HashSet::new(),
            new_owned: HashSet::new(),
            previous_source: None,
            meta_dir: manager.meta_dir.clone(),
        };
        assert!(
            apply(plan, &HashSet::new())
                .unwrap_err()
                .contains("changed while")
        );
        assert_eq!(
            std::fs::read(live.join("minecraft/sentinel")).unwrap(),
            b"new instance"
        );
        assert_eq!(manager.load_one(name).unwrap().created, current.created);
    }

    fn record(path: &str) -> ContentFileRecord {
        ContentFileRecord {
            relative_path: path.into(),
            kind: ContentKind::Mod,
            enabled: true,
            fingerprint: FileFingerprint {
                size: 1,
                modified_ns: 1,
                hashes: Default::default(),
            },
            resolution: Default::default(),
            provider_aliases: Vec::new(),
            provider_checks: Vec::new(),
            required_dependencies: Vec::new(),
            automatic_dependency: false,
            cleanup_eligible: false,
        }
    }

    #[test]
    fn refreshing_a_disabled_pack_file_keeps_only_the_updated_disabled_file() {
        let temp = tempfile::tempdir().unwrap();
        let old = InstancePaths::new(temp.path().join("old"));
        let new = InstancePaths::new(temp.path().join("new"));
        std::fs::create_dir_all(old.minecraft().join("mods")).unwrap();
        std::fs::create_dir_all(new.minecraft().join("mods")).unwrap();
        std::fs::write(old.minecraft().join("mods/pack.jar.disabled"), b"old").unwrap();
        std::fs::write(new.minecraft().join("mods/pack.jar"), b"updated").unwrap();
        let mut manifest = ContentManifest::default();
        manifest.upsert(record("mods/pack.jar"));
        manifest.save(&new.content_manifest()).unwrap();
        let owned = HashSet::from([PathBuf::from("mods/pack.jar")]);

        let previous =
            preserve_disabled_pack_files(old.root(), new.root(), &owned, &owned).unwrap();
        preserve_user_files(
            &old.minecraft(),
            &new.minecraft(),
            &old.minecraft(),
            &previous,
            &owned,
            &HashSet::new(),
        )
        .unwrap();

        assert_eq!(
            std::fs::read(new.minecraft().join("mods/pack.jar.disabled")).unwrap(),
            b"updated"
        );
        assert!(!new.minecraft().join("mods/pack.jar").exists());
        assert!(
            !ContentManifest::load(&new.content_manifest())
                .unwrap()
                .record(Path::new("mods/pack.jar.disabled"))
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn refresh_keeps_local_config_and_user_content_records() {
        let temp = tempfile::tempdir().unwrap();
        let old = InstancePaths::new(temp.path().join("old"));
        let new = InstancePaths::new(temp.path().join("new"));
        std::fs::create_dir_all(old.local_config()).unwrap();
        std::fs::write(old.local_config().join("options.txt"), b"local").unwrap();
        std::fs::create_dir_all(old.minecraft().join("mods")).unwrap();
        std::fs::write(old.minecraft().join("mods/user.jar"), b"user").unwrap();
        std::fs::write(old.minecraft().join("mods/replace.jar"), b"user").unwrap();
        std::fs::create_dir_all(new.minecraft().join("mods")).unwrap();
        let mut previous = ContentManifest::default();
        previous.upsert(record("mods/user.jar"));
        previous.upsert(record("mods/replace.jar"));
        previous.save(&old.content_manifest()).unwrap();
        let mut staged = ContentManifest::default();
        staged.upsert(record("mods/pack.jar"));
        staged.save(&new.content_manifest()).unwrap();
        PackState {
            source: ProviderProject {
                provider: "modrinth".to_owned(),
                project_id: "pack".to_owned(),
                version_id: "2".to_owned(),
            },
            files: vec![
                "mods/pack.jar".into(),
                "mods/user.jar".into(),
                "mods/replace.jar".into(),
            ],
        }
        .save(&new)
        .unwrap();

        preserve_refresh_state(
            old.root(),
            new.root(),
            &HashSet::new(),
            &HashSet::from([
                PathBuf::from("mods/pack.jar"),
                PathBuf::from("mods/user.jar"),
                PathBuf::from("mods/replace.jar"),
            ]),
            &HashSet::from([PathBuf::from("mods/replace.jar")]),
        )
        .unwrap();

        assert_eq!(
            std::fs::read(new.local_config().join("options.txt")).unwrap(),
            b"local"
        );
        let manifest = ContentManifest::load(&new.content_manifest()).unwrap();
        assert!(manifest.record(Path::new("mods/user.jar")).is_some());
        assert!(manifest.record(Path::new("mods/pack.jar")).is_some());
        assert!(manifest.record(Path::new("mods/replace.jar")).is_none());
        assert_eq!(
            PackState::load(&new).unwrap().files,
            vec![
                PathBuf::from("mods/pack.jar"),
                PathBuf::from("mods/replace.jar")
            ]
        );
    }

    #[test]
    fn config_sync_baseline_survives_refresh_and_preserves_publication_conflicts() {
        use crate::instance::config_sync::{self, ConfigSyncError};

        for (pack_changed, shared_changed) in [(false, true), (true, false), (true, true)] {
            let temp = tempfile::tempdir().unwrap();
            let manager =
                InstanceManager::new(temp.path().join("instances"), temp.path().join("meta"));
            let name = "synced-pack";
            let live = InstancePaths::new(manager.instances_dir.join(name));
            std::fs::create_dir_all(live.minecraft().join("config")).unwrap();
            std::fs::write(live.minecraft().join("config/pack.toml"), b"local-default").unwrap();
            std::fs::write(live.minecraft().join("options.txt"), b"default-options").unwrap();
            let source = ProviderProject {
                provider: "modrinth".to_owned(),
                project_id: "pack".to_owned(),
                version_id: "1".to_owned(),
            };
            let mut current: InstanceConfig = serde_json::from_value(serde_json::json!({
                "name": name, "game_version": "1.21", "loader": "vanilla",
                "created": "2026-01-01T00:00:00Z", "modpack_source": source
            }))
            .unwrap();
            manager.save(&current).unwrap();

            let staging = RefreshStaging::new(&manager.instances_dir).unwrap();
            let staged = InstancePaths::new(staging.path().join("instances").join(name));
            std::fs::create_dir_all(staged.minecraft().join("config")).unwrap();
            let refreshed = if pack_changed {
                "refreshed-pack"
            } else {
                "old-pack"
            };
            std::fs::write(staged.minecraft().join("config/pack.toml"), refreshed).unwrap();
            let target_source = ProviderProject {
                version_id: "2".to_owned(),
                ..source.clone()
            };
            let owned = HashSet::from([PathBuf::from("config/pack.toml")]);
            PackState {
                source: target_source.clone(),
                files: owned.iter().cloned().collect(),
            }
            .save(&staged)
            .unwrap();
            let mut instance = current.clone();
            instance.modpack_source = Some(target_source.clone());
            let plan = RefreshPlan {
                instance,
                summary: ImportSummary {
                    name: name.to_owned(),
                    pack_version: "2".to_owned(),
                    game_version: "1.21".to_owned(),
                    loader: crate::instance::ModLoader::Vanilla,
                    loader_version: None,
                    mod_count: 0,
                    override_count: 1,
                    format: super::super::PackFormat::Mrpack,
                    archive_path: staging.path().join("pack.mrpack"),
                    source: Some(target_source),
                },
                current_version: "1".to_owned(),
                target_version: "2".to_owned(),
                conflicts: Vec::new(),
                staged_instance: staged.root().to_owned(),
                staging,
                old_owned: owned.clone(),
                new_owned: owned,
                previous_source: Some(source),
                meta_dir: manager.meta_dir.clone(),
            };

            // Selection and baseline are created while the refresh is being reviewed.
            config_sync::create_profile(&manager.meta_dir, "shared").unwrap();
            let profile = crate::storage::MetadataPaths::new(&manager.meta_dir)
                .profiles()
                .join("shared");
            std::fs::create_dir_all(profile.join("config")).unwrap();
            std::fs::write(profile.join("config/pack.toml"), b"old-pack").unwrap();
            std::fs::write(profile.join("options.txt"), b"shared-options").unwrap();
            config_sync::switch_profile_and_save(&manager, &mut current, Some("shared")).unwrap();
            let baseline = std::fs::read(live.state().join("config-sync.json")).unwrap();
            if shared_changed {
                let other = manager.instances_dir.join("other/minecraft");
                std::fs::create_dir_all(&other).unwrap();
                let lock =
                    config_sync::prepare_for_launch(Some("shared"), &manager.meta_dir, &other)
                        .unwrap()
                        .unwrap();
                std::fs::write(other.join("config/pack.toml"), b"new-shared").unwrap();
                config_sync::finish_launch(lock, &other).unwrap();
            }

            let mut applied = apply(plan, &HashSet::new()).unwrap();
            assert_eq!(applied.config_sync_profile.as_deref(), Some("shared"));
            assert_eq!(
                std::fs::read(live.state().join("config-sync.json")).unwrap(),
                baseline
            );
            assert_eq!(
                std::fs::read_to_string(live.minecraft().join("config/pack.toml")).unwrap(),
                refreshed
            );
            let result = config_sync::switch_profile_and_save(&manager, &mut applied, None);
            if pack_changed && shared_changed {
                assert!(matches!(result, Err(ConfigSyncError::Conflict { .. })));
                assert_eq!(
                    manager
                        .load_one(name)
                        .unwrap()
                        .config_sync_profile
                        .as_deref(),
                    Some("shared")
                );
                assert_eq!(
                    std::fs::read(live.minecraft().join("config/pack.toml")).unwrap(),
                    b"refreshed-pack"
                );
                assert_eq!(
                    std::fs::read(live.state().join("config-sync.json")).unwrap(),
                    baseline
                );
            } else {
                result.unwrap();
                assert_eq!(manager.load_one(name).unwrap().config_sync_profile, None);
                assert_eq!(
                    std::fs::read(live.minecraft().join("config/pack.toml")).unwrap(),
                    b"local-default"
                );
                assert_eq!(
                    std::fs::read(live.minecraft().join("options.txt")).unwrap(),
                    b"default-options"
                );
            }
            assert_eq!(
                std::fs::read_to_string(profile.join("config/pack.toml")).unwrap(),
                if shared_changed {
                    "new-shared"
                } else {
                    "refreshed-pack"
                }
            );
            assert_eq!(
                std::fs::read(profile.join("options.txt")).unwrap(),
                b"shared-options"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refresh_preserves_user_symlinks_without_following_them() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::os::unix::fs::symlink("outside", old.join("linked.jar")).unwrap();
        std::os::unix::fs::symlink(&old, old.join("loop")).unwrap();
        let collisions = user_file_collisions(
            &old,
            &HashSet::new(),
            &HashSet::from([PathBuf::from("linked.jar")]),
        )
        .unwrap();
        assert_eq!(collisions, vec![PathBuf::from("linked.jar")]);
        preserve_user_files(
            &old,
            &new,
            &old,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_link(new.join("linked.jar")).unwrap(),
            PathBuf::from("outside")
        );
        assert_eq!(std::fs::read_link(new.join("loop")).unwrap(), old);
    }

    #[test]
    fn preservation_replaces_pack_files_and_keeps_user_files() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        std::fs::create_dir_all(old.join("mods")).unwrap();
        std::fs::create_dir_all(new.join("mods")).unwrap();
        std::fs::write(old.join("mods/pack.jar"), "old").unwrap();
        std::fs::write(old.join("mods/user.jar"), "user").unwrap();
        std::fs::write(new.join("mods/pack.jar"), "new").unwrap();
        std::fs::write(new.join("mods/user.jar"), "pack collision").unwrap();
        let owned = HashSet::from([PathBuf::from("mods/pack.jar")]);
        let target_owned = HashSet::from([
            PathBuf::from("mods/pack.jar"),
            PathBuf::from("mods/user.jar"),
        ]);

        preserve_user_files(&old, &new, &old, &owned, &target_owned, &HashSet::new()).unwrap();

        assert_eq!(
            std::fs::read_to_string(new.join("mods/pack.jar")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(new.join("mods/user.jar")).unwrap(),
            "user"
        );

        std::fs::write(new.join("mods/user.jar"), "pack collision").unwrap();
        preserve_user_files(
            &old,
            &new,
            &old,
            &owned,
            &target_owned,
            &HashSet::from([PathBuf::from("mods/user.jar")]),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(new.join("mods/user.jar")).unwrap(),
            "pack collision"
        );
    }

    #[test]
    fn interrupted_swap_restores_the_backup_and_removes_staging() {
        let temp = tempfile::tempdir().unwrap();
        let backup = temp.path().join(".Pack.rmcl-backup");
        let staging = temp.path().join(".rmcl-refresh-1");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("instance.json"), "{}").unwrap();
        std::fs::create_dir_all(&staging).unwrap();

        recover_interrupted(temp.path());

        assert!(temp.path().join("Pack/instance.json").is_file());
        assert!(!backup.exists());
        assert!(!staging.exists());
    }

    #[test]
    fn windows_ownership_fixture_reserializes_portably_and_refresh_removes_obsolete_files() {
        let temp = tempfile::tempdir().unwrap();
        let name = "archives";
        let live = InstancePaths::new(temp.path().join(name));
        std::fs::create_dir_all(live.minecraft().join("mods")).unwrap();
        std::fs::create_dir_all(live.minecraft().join("config")).unwrap();
        for (path, bytes) in [
            ("mods/updated.jar", "old"),
            ("mods/obsolete.jar", "obsolete"),
            ("mods/user.jar", "user"),
            ("mods/conflict.jar", "user conflict"),
            ("config/pack.toml", "old config"),
        ] {
            std::fs::write(live.minecraft().join(path), bytes).unwrap();
        }
        let raw_windows = br#"{"source":{"provider":"modrinth","project_id":"pack","version_id":"1"},"files":["mods\\updated.jar","mods\\obsolete.jar","config\\pack.toml"]}"#;
        crate::storage::write_atomic(&live.modpack_state(), raw_windows).unwrap();
        let previous = PackState::load(&live).unwrap();
        assert_eq!(
            previous.files,
            vec![
                PathBuf::from("mods/updated.jar"),
                PathBuf::from("mods/obsolete.jar"),
                PathBuf::from("config/pack.toml")
            ]
        );
        previous.save(&live).unwrap();
        let serialized: serde_json::Value =
            serde_json::from_slice(&std::fs::read(live.modpack_state()).unwrap()).unwrap();
        assert_eq!(
            serialized["files"],
            serde_json::json!(["mods/updated.jar", "mods/obsolete.jar", "config/pack.toml"])
        );

        let staging = RefreshStaging::new(temp.path()).unwrap();
        let stage_path = staging.path().to_owned();
        std::fs::create_dir_all(stage_path.join("archives/target")).unwrap();
        std::fs::write(stage_path.join("archives/target/pack.zip"), b"archive").unwrap();
        let staged = InstancePaths::new(stage_path.join("instances").join(name));
        std::fs::create_dir_all(staged.minecraft().join("mods")).unwrap();
        std::fs::create_dir_all(staged.minecraft().join("config")).unwrap();
        for (path, bytes) in [
            ("mods/updated.jar", "updated"),
            ("mods/conflict.jar", "pack conflict"),
            ("config/pack.toml", "updated config"),
        ] {
            std::fs::write(staged.minecraft().join(path), bytes).unwrap();
        }
        let source = ProviderProject {
            version_id: "2".to_owned(),
            ..previous.source.clone()
        };
        PackState {
            source: source.clone(),
            files: vec![
                PathBuf::from("mods").join("updated.jar"),
                PathBuf::from("mods").join("conflict.jar"),
                PathBuf::from("config").join("pack.toml"),
            ],
        }
        .save(&staged)
        .unwrap();
        let old_owned = previous.files.into_iter().collect();
        let new_owned = PackState::load(&staged)
            .unwrap()
            .files
            .into_iter()
            .collect();
        let conflicts = user_file_collisions(&live.minecraft(), &old_owned, &new_owned).unwrap();
        assert_eq!(conflicts, vec![PathBuf::from("mods/conflict.jar")]);
        let mut instance: InstanceConfig = serde_json::from_value(serde_json::json!({
            "name": name, "game_version": "1.21", "loader": "vanilla", "created": "2026-01-01T00:00:00Z"
        })).unwrap();
        let manager = InstanceManager::new(temp.path(), temp.path().join("meta"));
        let mut current = instance.clone();
        current.game_version = "1.20.1".to_owned();
        current.modpack_source = Some(previous.source.clone());
        manager.save(&current).unwrap();
        instance.modpack_source = Some(source);
        crate::storage::write_atomic(
            &staged.root().join("instance.json"),
            &serde_json::to_vec(&instance).unwrap(),
        )
        .unwrap();
        let plan = RefreshPlan {
            instance,
            summary: ImportSummary {
                name: name.to_owned(),
                pack_version: "2".to_owned(),
                game_version: "1.21".to_owned(),
                loader: crate::instance::ModLoader::Vanilla,
                loader_version: None,
                mod_count: 1,
                override_count: 1,
                format: super::super::PackFormat::Mrpack,
                archive_path: stage_path.join("archives/target/pack.zip"),
                source: None,
            },
            current_version: "1".to_owned(),
            target_version: "2".to_owned(),
            conflicts,
            staging,
            staged_instance: staged.root().to_owned(),
            old_owned,
            new_owned,
            previous_source: Some(previous.source),
            meta_dir: manager.meta_dir.clone(),
        };
        current.memory_max = Some("8G".to_owned());
        current.config_sync_profile = Some("shared".to_owned());
        current.last_played = Some(chrono::Utc::now());
        manager.save(&current).unwrap();
        manager
            .save_config_sync_profile(name, current.config_sync_profile.clone())
            .unwrap();
        recover_interrupted(temp.path());
        assert!(stage_path.join("archives/target/pack.zip").is_file());
        let applied = apply(plan, &HashSet::new()).unwrap();
        assert_eq!(applied.memory_max, current.memory_max);
        assert_eq!(applied.config_sync_profile, current.config_sync_profile);
        assert_eq!(applied.last_played, current.last_played);
        assert_eq!(
            manager.load_one(name).unwrap().memory_max,
            current.memory_max
        );
        assert_eq!(
            std::fs::read(live.minecraft().join("mods/updated.jar")).unwrap(),
            b"updated"
        );
        assert_eq!(
            std::fs::read(live.minecraft().join("config/pack.toml")).unwrap(),
            b"updated config"
        );
        assert_eq!(
            std::fs::read(live.minecraft().join("mods/user.jar")).unwrap(),
            b"user"
        );
        assert_eq!(
            std::fs::read(live.minecraft().join("mods/conflict.jar")).unwrap(),
            b"user conflict"
        );
        assert!(!live.minecraft().join("mods/obsolete.jar").exists());
        assert!(!stage_path.exists());
        let serialized: serde_json::Value =
            serde_json::from_slice(&std::fs::read(live.modpack_state()).unwrap()).unwrap();
        assert_eq!(
            serialized["files"],
            serde_json::json!(["mods/updated.jar", "config/pack.toml"])
        );
    }

    #[test]
    fn ownership_rejects_unsafe_paths_without_overwriting_valid_state() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstancePaths::new(temp.path());
        let mut state = PackState {
            source: ProviderProject {
                provider: "modrinth".to_owned(),
                project_id: "pack".to_owned(),
                version_id: "1".to_owned(),
            },
            files: vec![PathBuf::from("mods").join("pack.jar")],
        };
        state.save(&paths).unwrap();
        let original = std::fs::read(paths.modpack_state()).unwrap();
        for path in [
            "../escape",
            r"..\escape",
            "mods/../escape",
            "C:victim",
            r"C:\absolute",
            r"\rooted",
            r"\\server\share\file",
            r"\\?\C:\file",
            "/absolute",
            "mods/C:victim",
            "mods/NUL.jar",
            "mods/file:stream",
            "mods/trailing.",
        ] {
            let value = serde_json::json!({"source": state.source, "files": [path]});
            assert!(
                serde_json::from_value::<PackState>(value.clone()).is_err(),
                "{path}"
            );
            state.files = vec![PathBuf::from(path)];
            assert!(state.save(&paths).is_err(), "{path}");
            assert_eq!(std::fs::read(paths.modpack_state()).unwrap(), original);
        }
    }

    #[test]
    fn refresh_lease_child() {
        use std::io::Write;
        let Some(root) = std::env::var_os("RMCL_REFRESH_LEASE_TEST") else {
            return;
        };
        let root = Path::new(&root);
        let staging = RefreshStaging::new(root).unwrap();
        let _restore_lock =
            crate::instance::runtime::lock_instance(root, "rmcl-refresh-Pack").unwrap();
        let _cleanup_lock = crate::instance::runtime::lock_instance(root, "Live").unwrap();
        println!(
            "LEASED {}",
            staging.path().file_name().unwrap().to_str().unwrap()
        );
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        drop(staging);
    }

    #[test]
    fn recovery_skips_another_process_staging_and_both_locked_backup_paths() {
        use std::io::BufRead;
        let temp = tempfile::tempdir().unwrap();
        for directory in [
            ".rmcl-refresh-Pack.rmcl-backup",
            ".Live.rmcl-backup",
            "Live",
            ".rmcl-refresh-legacy",
        ] {
            std::fs::create_dir(temp.path().join(directory)).unwrap();
            std::fs::write(temp.path().join(directory).join("instance.json"), b"{}").unwrap();
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "instance::import::refresh::tests::refresh_lease_child",
                "--nocapture",
            ])
            .env("RMCL_REFRESH_LEASE_TEST", temp.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stage_name = std::io::BufReader::new(child.stdout.take().unwrap())
            .lines()
            .find_map(|line| line.unwrap().strip_prefix("LEASED ").map(str::to_owned))
            .expect("child acquired its leases");
        let staging = temp.path().join(stage_name);
        recover_interrupted(temp.path());
        let active_stage_survived = staging.is_dir();
        let restore_backup_survived = temp
            .path()
            .join(".rmcl-refresh-Pack.rmcl-backup/instance.json")
            .is_file()
            && !temp.path().join("rmcl-refresh-Pack").exists();
        let cleanup_backup_survived = temp
            .path()
            .join(".Live.rmcl-backup/instance.json")
            .is_file();
        let _ = child.kill();
        child.wait().unwrap();
        recover_interrupted(temp.path());
        assert!(active_stage_survived);
        assert!(restore_backup_survived);
        assert!(cleanup_backup_survived);
        assert!(!staging.exists());
        assert!(
            temp.path()
                .join("rmcl-refresh-Pack/instance.json")
                .is_file()
        );
        assert!(!temp.path().join(".rmcl-refresh-Pack.rmcl-backup").exists());
        assert!(temp.path().join("Live/instance.json").is_file());
        assert!(!temp.path().join(".Live.rmcl-backup").exists());
        assert!(!temp.path().join(".rmcl-refresh-legacy").exists());
    }

    #[tokio::test]
    async fn cancelled_preparation_cleans_its_distinct_leased_staging() {
        let temp = tempfile::tempdir().unwrap();
        let other = RefreshStaging::new(temp.path()).unwrap();
        let root = temp.path().to_owned();
        let (ready, received) = tokio::sync::oneshot::channel();
        let preparation = tokio::spawn(async move {
            let staging = RefreshStaging::new(&root).unwrap();
            ready.send(staging.path().to_owned()).unwrap();
            std::future::pending::<()>().await;
            drop(staging);
        });
        let stage = received.await.unwrap();
        assert_ne!(stage, other.path());
        recover_interrupted(temp.path());
        assert!(stage.is_dir());
        assert!(other.path().is_dir());
        preparation.abort();
        assert!(preparation.await.unwrap_err().is_cancelled());
        assert!(!stage.exists());
        assert!(other.path().is_dir());
    }
}
