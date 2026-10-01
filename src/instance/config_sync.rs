// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigSyncError {
    #[error("Invalid config sync profile: {0}")]
    InvalidProfile(String),
    #[error("Cannot switch config profiles while '{instance}' is running")]
    InstanceRunning { instance: String },
    #[error("Config profile '{0}' is in use by another instance")]
    ProfileInUse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to save config profile: {0}")]
    Save(String),
    #[error("Failed to save config profile ({save}) and roll back profile files ({rollback})")]
    SaveRollback { save: String, rollback: String },
}

#[derive(Debug)]
pub struct ConfigSyncLock {
    file: std::fs::File,
    profile_dir: PathBuf,
}

impl Drop for ConfigSyncLock {
    fn drop(&mut self) {
        if let Err(e) = self.file.unlock() {
            tracing::warn!("Failed to release config sync lock: {}", e);
        }
    }
}

pub fn prepare(
    profile: Option<&str>,
    meta_dir: &Path,
    minecraft_dir: &Path,
) -> Result<bool, ConfigSyncError> {
    Ok(prepare_for_launch(profile, meta_dir, minecraft_dir)?.is_some())
}

pub fn prepare_for_launch(
    profile: Option<&str>,
    meta_dir: &Path,
    minecraft_dir: &Path,
) -> Result<Option<ConfigSyncLock>, ConfigSyncError> {
    let Some(profile) = profile.and_then(normalize_profile) else {
        return Ok(None);
    };
    validate_profile(profile)?;

    let profile_dir = profile_dir(meta_dir, profile);
    if !profile_dir.exists() {
        return Ok(None);
    }
    let lock = acquire_lock(&profile_dir)?;

    if !profile_payload_exists(&profile_dir)? {
        sync_to_profile(minecraft_dir, &profile_dir)?;
    } else {
        sync_from_profile(&profile_dir, minecraft_dir)?;
    }

    Ok(Some(lock))
}

pub fn finish_launch(lock: ConfigSyncLock, minecraft_dir: &Path) -> Result<(), ConfigSyncError> {
    sync_to_profile(minecraft_dir, &lock.profile_dir)
}

pub fn finish(
    profile: Option<&str>,
    meta_dir: &Path,
    minecraft_dir: &Path,
) -> Result<(), ConfigSyncError> {
    let Some(profile) = profile.and_then(normalize_profile) else {
        return Ok(());
    };
    validate_profile(profile)?;

    let profile_dir = profile_dir(meta_dir, profile);
    let _lock = acquire_lock(&profile_dir)?;
    sync_to_profile(minecraft_dir, &profile_dir)
}

pub fn list_profiles(meta_dir: &Path) -> Result<Vec<String>, ConfigSyncError> {
    let root = profiles_dir(meta_dir);
    let mut profiles = Vec::new();
    if !root.exists() {
        return Ok(profiles);
    }

    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if validate_profile(&name).is_ok() {
                profiles.push(name);
            }
        }
    }
    profiles.sort_unstable();
    Ok(profiles)
}

pub fn create_profile(meta_dir: &Path, profile: &str) -> Result<String, ConfigSyncError> {
    let Some(profile) = normalize_profile(profile) else {
        return Err(ConfigSyncError::InvalidProfile(profile.to_string()));
    };
    validate_profile(profile)?;
    std::fs::create_dir_all(profile_dir(meta_dir, profile))?;
    Ok(profile.to_string())
}

pub fn delete_profile(meta_dir: &Path, profile: &str) -> Result<(), ConfigSyncError> {
    validate_profile(profile)?;
    let dir = profile_dir(meta_dir, profile);
    if dir.exists() {
        let _lock = acquire_lock(&dir)?;
        remove_path(&dir)?;
    }
    Ok(())
}

fn switch_profile(
    instance_name: &str,
    current_profile: Option<&str>,
    target_profile: Option<&str>,
    meta_dir: &Path,
    instance_dir: &Path,
) -> Result<Option<String>, ConfigSyncError> {
    if crate::instance::runtime::is_active(instance_name) {
        return Err(ConfigSyncError::InstanceRunning {
            instance: instance_name.to_string(),
        });
    }

    let current_profile = current_profile.and_then(normalize_profile);
    let target_profile = target_profile.and_then(normalize_profile);
    if let Some(profile) = current_profile {
        validate_profile(profile)?;
    }
    if let Some(profile) = target_profile {
        validate_profile(profile)?;
    }

    if current_profile == target_profile {
        return Ok(current_profile.map(str::to_string));
    }

    if let Some(profile) = current_profile {
        let profile_dir = profile_dir(meta_dir, profile);
        if profile_dir.exists() {
            let _lock = acquire_lock(&profile_dir)?;
            sync_to_profile(&minecraft_dir(instance_dir), &profile_dir)?;
        }
    }

    match (current_profile, target_profile) {
        (None, Some(_)) => {
            sync_to_profile(
                &minecraft_dir(instance_dir),
                &local_backup_dir(instance_dir),
            )?;
        }
        (Some(_), None) => {
            let backup = local_backup_dir(instance_dir);
            if backup.exists() {
                sync_from_profile(&backup, &minecraft_dir(instance_dir))?;
            }
            return Ok(None);
        }
        _ => {}
    }

    let Some(profile) = target_profile else {
        return Ok(None);
    };

    let profile_dir = profile_dir(meta_dir, profile);
    let _lock = acquire_lock(&profile_dir)?;

    if !profile_payload_exists(&profile_dir)? {
        sync_to_profile(&minecraft_dir(instance_dir), &profile_dir)?;
    }
    sync_from_profile(&profile_dir, &minecraft_dir(instance_dir))?;

    Ok(Some(profile.to_string()))
}

pub fn switch_profile_and_save(
    manager: &crate::instance::InstanceManager,
    config: &mut crate::instance::InstanceConfig,
    target_profile: Option<&str>,
) -> Result<(), ConfigSyncError> {
    let current_profile = config.config_sync_profile.clone();
    let instance_dir = manager.instances_dir.join(&config.name);
    let selected = switch_profile_and_persist(
        &config.name,
        current_profile.as_deref(),
        target_profile,
        &manager.meta_dir,
        &instance_dir,
        |selected| {
            let mut updated = config.clone();
            updated.config_sync_profile = selected.map(str::to_owned);
            manager.save(&updated)
        },
    )?;
    config.config_sync_profile = selected;
    Ok(())
}

fn switch_profile_and_persist<E>(
    instance_name: &str,
    current_profile: Option<&str>,
    target_profile: Option<&str>,
    meta_dir: &Path,
    instance_dir: &Path,
    persist: impl FnOnce(Option<&str>) -> Result<(), E>,
) -> Result<Option<String>, ConfigSyncError>
where
    E: std::fmt::Display,
{
    let selected = switch_profile(
        instance_name,
        current_profile,
        target_profile,
        meta_dir,
        instance_dir,
    )?;
    let Err(save) = persist(selected.as_deref()) else {
        return Ok(selected);
    };
    let save = save.to_string();

    if let Err(rollback) = switch_profile(
        instance_name,
        selected.as_deref(),
        current_profile,
        meta_dir,
        instance_dir,
    ) {
        return Err(ConfigSyncError::SaveRollback {
            save,
            rollback: rollback.to_string(),
        });
    }
    Err(ConfigSyncError::Save(save))
}

fn normalize_profile(profile: &str) -> Option<&str> {
    let trimmed = profile.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub fn validate_profile(profile: &str) -> Result<(), ConfigSyncError> {
    if profile.is_empty()
        || profile.len() > 64
        || profile.starts_with('.')
        || profile.contains('/')
        || profile.contains('\\')
        || profile.eq_ignore_ascii_case("default")
        || profile.eq_ignore_ascii_case("none")
        || profile.eq_ignore_ascii_case("local")
        || profile.eq_ignore_ascii_case("instance default")
        || profile.eq_ignore_ascii_case("local default")
        || profile
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(ConfigSyncError::InvalidProfile(profile.to_string()));
    }
    Ok(())
}

fn profile_dir(meta_dir: &Path, profile: &str) -> PathBuf {
    profiles_dir(meta_dir).join(profile)
}

fn profiles_dir(meta_dir: &Path) -> PathBuf {
    crate::storage::MetadataPaths::new(meta_dir).profiles()
}

fn minecraft_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join(crate::storage::MINECRAFT_DIR_NAME)
}

fn local_backup_dir(instance_dir: &Path) -> PathBuf {
    crate::storage::InstancePaths::new(instance_dir).local_config()
}

fn acquire_lock(profile_dir: &Path) -> Result<ConfigSyncLock, ConfigSyncError> {
    std::fs::create_dir_all(profile_dir)?;
    let path = profile_dir.join(".lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(ConfigSyncError::ProfileInUse(
                profile_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    Ok(ConfigSyncLock {
        file,
        profile_dir: profile_dir.to_owned(),
    })
}

fn mirror_dir(src: &Path, dst: &Path) -> Result<(), ConfigSyncError> {
    let parent = dst.parent().filter(|p| !p.as_os_str().is_empty());
    let Some(parent) = parent else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("no parent directory for {}", dst.display()),
        )
        .into());
    };
    std::fs::create_dir_all(parent)?;
    let name = dst.file_name().and_then(|s| s.to_str()).unwrap_or("mirror");
    let staging = parent.join(format!(".{name}.mirror-tmp"));
    let backup = parent.join(format!(".{name}.mirror-backup"));
    if std::fs::symlink_metadata(dst).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(std::io::Error::other(format!(
            "Cannot replace symlinked config directory '{}'",
            dst.display()
        ))
        .into());
    }
    if backup.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("Previous config backup remains at {}", backup.display()),
        )
        .into());
    }
    if staging.exists() {
        remove_path(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    let result = (|| {
        if src.exists() {
            copy_dir_contents(src, &staging)?;
        }
        let had_dst = dst.exists();
        if had_dst {
            std::fs::rename(dst, &backup)?;
        }
        if let Err(error) = std::fs::rename(&staging, dst) {
            if had_dst && let Err(rollback) = std::fs::rename(&backup, dst) {
                return Err(ConfigSyncError::SaveRollback {
                    save: error.to_string(),
                    rollback: rollback.to_string(),
                });
            }
            return Err(error.into());
        }
        if had_dst && let Err(error) = remove_path(&backup) {
            tracing::warn!(
                "Could not remove old config backup '{}': {error}",
                backup.display()
            );
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

fn sync_to_profile(minecraft_dir: &Path, profile_dir: &Path) -> Result<(), ConfigSyncError> {
    mirror_dir(&minecraft_dir.join("config"), &profile_dir.join("config"))?;
    mirror_options(minecraft_dir, profile_dir)
}

fn sync_from_profile(profile_dir: &Path, minecraft_dir: &Path) -> Result<(), ConfigSyncError> {
    mirror_dir(&profile_dir.join("config"), &minecraft_dir.join("config"))?;
    mirror_options(profile_dir, minecraft_dir)
}

fn profile_payload_exists(profile_dir: &Path) -> Result<bool, ConfigSyncError> {
    if profile_dir.join("config").exists() {
        return Ok(true);
    }
    if !profile_dir.exists() {
        return Ok(false);
    }
    for entry in std::fs::read_dir(profile_dir)? {
        let entry = entry?;
        if (entry.file_type()?.is_file() || entry.file_type()?.is_symlink())
            && is_options_file(&entry.file_name().to_string_lossy())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn mirror_options(src: &Path, dst: &Path) -> Result<(), ConfigSyncError> {
    std::fs::create_dir_all(dst)?;
    let staging = dst.join(".rmcl-options-stage");
    if staging.exists() {
        remove_path(&staging)?;
    }
    std::fs::create_dir(&staging)?;
    let result = (|| {
        let mut copied = std::collections::HashSet::new();
        if src.exists() {
            for entry in std::fs::read_dir(src)? {
                let entry = entry?;
                let name = entry.file_name();
                let file_type = entry.file_type()?;
                if is_options_file(&name.to_string_lossy())
                    && (file_type.is_file() || file_type.is_symlink())
                {
                    if file_type.is_symlink() {
                        copy_link(&entry.path(), &staging.join(&name))?;
                    } else {
                        std::fs::copy(entry.path(), staging.join(&name))?;
                    }
                    copied.insert(name);
                }
            }
        }
        for entry in std::fs::read_dir(&staging)? {
            let entry = entry?;
            crate::storage::replace_file(&entry.path(), &dst.join(entry.file_name()))?;
        }
        remove_options(dst, &copied)?;
        Ok(())
    })();
    if let Err(error) = std::fs::remove_dir_all(&staging) {
        tracing::warn!(
            "Could not remove options staging '{}': {error}",
            staging.display()
        );
    }
    result
}

fn remove_options(
    dir: &Path,
    copied: &std::collections::HashSet<std::ffi::OsString>,
) -> Result<(), ConfigSyncError> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if is_options_file(&entry.file_name().to_string_lossy())
            && !copied.contains(&entry.file_name())
        {
            remove_path(&entry.path())?;
        }
    }
    Ok(())
}

fn is_options_file(name: &str) -> bool {
    name == "options.txt" || name.starts_with("options") && name.ends_with(".txt")
}

fn remove_path(path: &Path) -> Result<(), ConfigSyncError> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn copy_dir_contents(src: &Path, dst: &Path) -> Result<(), ConfigSyncError> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let source = entry.path();
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_dir_contents(&source, &target)?;
        } else if file_type.is_file() {
            std::fs::copy(&source, &target)?;
        } else if file_type.is_symlink() {
            copy_link(&source, &target)?;
        }
    }

    Ok(())
}

#[cfg(unix)]
fn copy_link(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(std::fs::read_link(source)?, target)
}

#[cfg(not(unix))]
fn copy_link(source: &Path, _target: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!(
            "Cannot copy config symlink '{}' on this platform",
            source.display()
        ),
    ))
}

#[cfg(test)]
#[path = "tests/config_sync.rs"]
mod tests;
