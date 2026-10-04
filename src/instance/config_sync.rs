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
    #[error("Config profile '{}' and local settings '{}' have diverged; both were kept", profile.display(), minecraft.display())]
    Conflict {
        profile: PathBuf,
        minecraft: PathBuf,
    },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to save config profile: {0}")]
    Save(String),
    #[error("Config sync failed ({save}) and rollback failed ({rollback})")]
    SaveRollback { save: String, rollback: String },
    #[error(transparent)]
    Instance(#[from] crate::instance::InstanceError),
}

#[derive(Debug)]
pub struct ConfigSyncLock {
    file: std::fs::File,
    profile_dir: PathBuf,
    recovered_source: Option<PathBuf>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PayloadBaseline {
    profile: PathBuf,
    // External relative links are re-anchored on import, so these hashes can differ.
    local: String,
    shared: String,
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
    if !profile_exists(&profile_dir)? {
        return Ok(None);
    }
    let lock = acquire_lock(&profile_dir)?;
    prepare_locked(lock, minecraft_dir)
}

fn prepare_locked(
    lock: ConfigSyncLock,
    minecraft_dir: &Path,
) -> Result<Option<ConfigSyncLock>, ConfigSyncError> {
    let profile_dir = &lock.profile_dir;
    if !profile_exists(profile_dir)? {
        return Ok(None);
    }

    if !profile_payload_exists(profile_dir)? {
        publish_current(minecraft_dir, profile_dir, false)?;
        persist_with_baseline(minecraft_dir, Some(profile_dir), || {
            mark_unsynced(profile_dir, minecraft_dir)
        })?;
    } else {
        let identity = std::fs::canonicalize(profile_dir)?;
        if read_baseline(minecraft_dir)?.is_some_and(|baseline| baseline.profile == identity) {
            publish_current(minecraft_dir, profile_dir, false)?;
        }
        mirror_payload(profile_dir, minecraft_dir, true, || {
            persist_with_baseline(minecraft_dir, Some(profile_dir), || {
                mark_unsynced(profile_dir, minecraft_dir)
            })
        })?;
    }

    Ok(Some(lock))
}

pub fn finish_launch(lock: ConfigSyncLock, minecraft_dir: &Path) -> Result<(), ConfigSyncError> {
    let allow_untracked = match &lock.recovered_source {
        Some(source) => std::fs::canonicalize(source)? == std::fs::canonicalize(minecraft_dir)?,
        None => true,
    };
    publish_current(minecraft_dir, &lock.profile_dir, allow_untracked)?;
    remove_if_exists(&unsynced_marker(&lock.profile_dir))
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
    finish_launch(acquire_lock(&profile_dir)?, minecraft_dir)
}

pub fn list_profiles(meta_dir: &Path) -> Result<Vec<String>, ConfigSyncError> {
    let root = profiles_dir(meta_dir);
    let mut profiles = Vec::new();
    if !entry_exists(&root)? {
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
    let _lock = acquire_lock(&profile_dir(meta_dir, profile))?;
    std::fs::create_dir_all(profile_dir(meta_dir, profile))?;
    Ok(profile.to_string())
}

pub fn delete_profile(meta_dir: &Path, profile: &str) -> Result<(), ConfigSyncError> {
    validate_profile(profile)?;
    let dir = profile_dir(meta_dir, profile);
    if profile_exists(&dir)? {
        let _lock = acquire_lock(&dir)?;
        if profile_exists(&dir)? {
            remove_path(&dir)?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn switch_profile(
    instance_name: &str,
    current_profile: Option<&str>,
    target_profile: Option<&str>,
    meta_dir: &Path,
    instance_dir: &Path,
) -> Result<Option<String>, ConfigSyncError> {
    switch_profile_and_persist(
        instance_name,
        current_profile,
        target_profile,
        meta_dir,
        instance_dir,
        |_| Ok::<(), std::io::Error>(()),
    )
}

pub fn switch_profile_and_save(
    manager: &crate::instance::InstanceManager,
    config: &mut crate::instance::InstanceConfig,
    target_profile: Option<&str>,
) -> Result<(), ConfigSyncError> {
    let _instance_lock =
        crate::instance::runtime::lock_instance(&manager.instances_dir, &config.name)?;
    let current = manager.load_one(&config.name)?;
    let instance_dir = manager.instances_dir.join(&current.name);
    let mut saved = None;
    switch_profile_and_persist(
        &current.name,
        current.config_sync_profile.as_deref(),
        target_profile,
        &manager.meta_dir,
        &instance_dir,
        |selected| {
            saved =
                Some(manager.save_config_sync_profile(&current.name, selected.map(str::to_owned))?);
            Ok::<(), crate::instance::InstanceError>(())
        },
    )?;
    *config = saved.expect("successful profile switch persisted its selection");
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
    if crate::instance::runtime::is_active(instance_name) {
        return Err(ConfigSyncError::InstanceRunning {
            instance: instance_name.to_owned(),
        });
    }
    let current = current_profile.and_then(normalize_profile);
    let target = target_profile.and_then(normalize_profile);
    for profile in [current, target].into_iter().flatten() {
        validate_profile(profile)?;
        profile_exists(&profile_dir(meta_dir, profile))?;
    }
    let save =
        |selected| persist(selected).map_err(|error| ConfigSyncError::Save(error.to_string()));
    if current == target {
        save(current)?;
        return Ok(current.map(str::to_owned));
    }
    let minecraft = minecraft_dir(instance_dir);
    if let Some(profile) = current {
        let dir = profile_dir(meta_dir, profile);
        if profile_exists(&dir)? {
            let _lock = acquire_lock(&dir)?;
            if profile_exists(&dir)? {
                publish_current(&minecraft, &dir, false)?;
            }
        }
    }
    let local = local_backup_dir(instance_dir);
    if current.is_none() && target.is_some() {
        sync_to_profile(&minecraft, &local)?;
    }
    let Some(profile) = target else {
        if current.is_some() && entry_exists(&local)? {
            return mirror_payload(&local, &minecraft, true, || {
                persist_with_baseline(&minecraft, None, || save(None))?;
                Ok(None)
            });
        }
        persist_with_baseline(&minecraft, None, || save(None))?;
        return Ok(None);
    };
    let dir = profile_dir(meta_dir, profile);
    let _lock = acquire_lock(&dir)?;
    if !profile_payload_exists(&dir)? {
        sync_to_profile(&minecraft, &dir)?;
    }
    mirror_payload(&dir, &minecraft, true, || {
        persist_with_baseline(&minecraft, Some(&dir), || save(Some(profile)))?;
        Ok(Some(profile.to_owned()))
    })
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
        || !crate::instance::manager::portable_component(profile)
        || profile.eq_ignore_ascii_case("default")
        || profile.eq_ignore_ascii_case("none")
        || profile.eq_ignore_ascii_case("local")
        || profile.eq_ignore_ascii_case("instance default")
        || profile.eq_ignore_ascii_case("local default")
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
    let parent = profile_dir
        .parent()
        .ok_or_else(|| std::io::Error::other("Config profile has no parent"))?;
    std::fs::create_dir_all(parent)?;
    // Keep locks outside deletable profiles so recreating a profile cannot bypass an old handle.
    let path = parent.join(format!(
        ".{}.lock",
        profile_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ));
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
    let mut lock = ConfigSyncLock {
        file,
        profile_dir: profile_dir.to_owned(),
        recovered_source: None,
    };
    profile_exists(profile_dir)?;
    lock.recovered_source = recover_unsynced(profile_dir)?;
    Ok(lock)
}

fn unsynced_marker(profile: &Path) -> PathBuf {
    profile.join(".unsynced.json")
}

fn mark_unsynced(profile: &Path, minecraft: &Path) -> Result<(), ConfigSyncError> {
    let source = std::path::absolute(minecraft)?;
    if !std::fs::metadata(&source)?.is_dir() {
        return Err(std::io::Error::other("Config sync source is not a directory").into());
    }
    let bytes = serde_json::to_vec(&source).map_err(std::io::Error::other)?;
    crate::storage::write_atomic(&unsynced_marker(profile), &bytes)?;
    Ok(())
}

fn recover_unsynced(profile: &Path) -> Result<Option<PathBuf>, ConfigSyncError> {
    let marker = unsynced_marker(profile);
    let bytes = match std::fs::read(&marker) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let source: PathBuf = serde_json::from_slice(&bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Invalid config sync marker '{}': {error}", marker.display()),
        )
    })?;
    if !source.is_absolute() || !std::fs::metadata(&source)?.is_dir() {
        return Err(std::io::Error::other(format!(
            "Invalid unsynced config source '{}'",
            source.display()
        ))
        .into());
    }
    publish_current(&source, profile, true)?;
    std::fs::remove_file(marker)?;
    Ok(Some(source))
}

fn baseline_path(minecraft: &Path) -> Result<PathBuf, ConfigSyncError> {
    let instance = minecraft
        .parent()
        .ok_or_else(|| std::io::Error::other("Config sync source has no parent"))?;
    Ok(crate::storage::InstancePaths::new(instance)
        .state()
        .join("config-sync.json"))
}

fn read_baseline(minecraft: &Path) -> Result<Option<PayloadBaseline>, ConfigSyncError> {
    let path = baseline_path(minecraft)?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid config sync baseline '{}': {error}", path.display()),
            )
            .into()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn baseline_bytes(
    profile: &Path,
    local: String,
    shared: String,
) -> Result<Vec<u8>, ConfigSyncError> {
    let baseline = PayloadBaseline {
        profile: std::fs::canonicalize(profile)?,
        local,
        shared,
    };
    Ok(serde_json::to_vec(&baseline).map_err(std::io::Error::other)?)
}

fn persist_with_baseline<T>(
    minecraft: &Path,
    profile: Option<&Path>,
    persist: impl FnOnce() -> Result<T, ConfigSyncError>,
) -> Result<T, ConfigSyncError> {
    let path = baseline_path(minecraft)?;
    let previous = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(profile) = profile {
        crate::storage::write_atomic(
            &path,
            &baseline_bytes(profile, payload_hash(minecraft)?, payload_hash(profile)?)?,
        )?;
    } else {
        remove_if_exists(&path)?;
    }
    match persist() {
        Ok(value) => Ok(value),
        Err(error) => {
            let rollback = match previous {
                Some(bytes) => {
                    crate::storage::write_atomic(&path, &bytes).map_err(ConfigSyncError::from)
                }
                None => remove_if_exists(&path),
            };
            match rollback {
                Ok(()) => Err(error),
                Err(rollback) => Err(ConfigSyncError::SaveRollback {
                    save: error.to_string(),
                    rollback: format!("Restoring baseline '{}': {rollback}", path.display()),
                }),
            }
        }
    }
}

fn publish_current(
    minecraft: &Path,
    profile: &Path,
    allow_untracked: bool,
) -> Result<(), ConfigSyncError> {
    let local = payload_hash(minecraft)?;
    let shared = payload_hash(profile)?;
    let identity = std::fs::canonicalize(profile)?;
    let baseline = read_baseline(minecraft)?.filter(|baseline| baseline.profile == identity);
    let needs_export = local != shared;
    if needs_export {
        match baseline {
            Some(baseline) if local == baseline.local => return Ok(()),
            Some(baseline) if shared == baseline.shared => {}
            None if allow_untracked || !profile_payload_exists(profile)? => {}
            _ => {
                return Err(ConfigSyncError::Conflict {
                    profile: profile.to_owned(),
                    minecraft: minecraft.to_owned(),
                });
            }
        }
    }
    let persist = |shared| {
        crate::storage::write_atomic(
            &baseline_path(minecraft)?,
            &baseline_bytes(profile, local, shared)?,
        )?;
        Ok(())
    };
    if needs_export {
        mirror_payload(minecraft, profile, true, || persist(payload_hash(profile)?))
    } else {
        persist(shared)
    }
}

fn payload_hash(dir: &Path) -> Result<String, ConfigSyncError> {
    use sha2::Digest;
    require_source_dir(dir)?;
    check_config_root(dir)?;
    let mut hash = sha2::Sha256::new();
    for name in payload_names(dir, true)? {
        hash_payload_entry(&dir.join(name), dir, &mut hash)?;
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn hash_payload_entry(
    path: &Path,
    root: &Path,
    hash: &mut sha2::Sha256,
) -> Result<(), ConfigSyncError> {
    use sha2::Digest;
    let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
    let bytes = relative.as_os_str().as_encoded_bytes();
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    let kind = std::fs::symlink_metadata(path)?.file_type();
    if kind.is_symlink() {
        hash.update(b"link");
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileTypeExt;
            hash.update([u8::from(kind.is_symlink_dir())]);
        }
        let target = std::fs::read_link(path)?;
        let bytes = target.as_os_str().as_encoded_bytes();
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    } else if kind.is_file() {
        hash.update(b"file");
        hash.update(
            crate::instance::content::manifest::fingerprint(path)?.hashes["sha512"].as_bytes(),
        );
    } else if kind.is_dir() {
        hash.update(b"dir");
        let mut entries = std::fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort();
        for entry in entries {
            hash_payload_entry(&entry, root, hash)?;
        }
    } else {
        return Err(std::io::Error::other(format!(
            "Unsupported config entry '{}'",
            path.display()
        ))
        .into());
    }
    Ok(())
}

fn profile_exists(dir: &Path) -> Result<bool, ConfigSyncError> {
    match std::fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "Config profile must be a real directory: '{}'",
                dir.display()
            ),
        )
        .into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn require_source_dir(src: &Path) -> Result<(), ConfigSyncError> {
    let metadata = std::fs::metadata(src).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "Cannot read config sync source '{}': {error}",
                src.display()
            ),
        )
    })?;
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Config sync source is not a directory: '{}'", src.display()),
        )
        .into());
    }
    Ok(())
}

fn check_config_root(dir: &Path) -> Result<(), ConfigSyncError> {
    let config = dir.join("config");
    if entry_exists(&config)? && std::fs::symlink_metadata(&config)?.file_type().is_symlink() {
        return Err(std::io::Error::other(format!(
            "Cannot mirror symlinked config directory '{}'",
            config.display()
        ))
        .into());
    }
    Ok(())
}

fn sync_to_profile(minecraft_dir: &Path, profile_dir: &Path) -> Result<(), ConfigSyncError> {
    mirror_payload(minecraft_dir, profile_dir, true, || Ok(()))
}

#[cfg(test)]
fn sync_from_profile(profile_dir: &Path, minecraft_dir: &Path) -> Result<(), ConfigSyncError> {
    mirror_payload(profile_dir, minecraft_dir, true, || Ok(()))
}

fn profile_payload_exists(profile_dir: &Path) -> Result<bool, ConfigSyncError> {
    Ok(!payload_names(profile_dir, true)?.is_empty())
}

#[cfg(test)]
fn mirror_options(src: &Path, dst: &Path) -> Result<(), ConfigSyncError> {
    mirror_payload(src, dst, false, || Ok(()))
}

fn entry_exists(path: &Path) -> Result<bool, ConfigSyncError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn remove_if_exists(path: &Path) -> Result<(), ConfigSyncError> {
    if entry_exists(path)? {
        remove_path(path)?;
    }
    Ok(())
}

fn cleanup_sync_path(path: &Path) {
    if let Err(error) = remove_if_exists(path) {
        tracing::warn!(
            "Could not remove config sync staging/backup '{}': {error}",
            path.display()
        );
    }
}

fn payload_names(
    dir: &Path,
    include_config: bool,
) -> Result<Vec<std::ffi::OsString>, ConfigSyncError> {
    if !entry_exists(dir)? {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = entry.file_type()?;
        if include_config && name == "config"
            || is_options_file(&name.to_string_lossy()) && (kind.is_file() || kind.is_symlink())
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

fn rollback_payload(
    dst: &Path,
    backup: &Path,
    installed: &[std::ffi::OsString],
    moved: &[std::ffi::OsString],
) -> Result<(), String> {
    let mut errors = Vec::new();
    for name in installed.iter().rev() {
        if let Err(error) = remove_if_exists(&dst.join(name)) {
            errors.push(format!("Removing '{}': {error}", dst.join(name).display()));
        }
    }
    for name in moved.iter().rev() {
        let target = dst.join(name);
        match entry_exists(&target) {
            Ok(false) => {
                if let Err(error) = std::fs::rename(backup.join(name), &target) {
                    errors.push(format!("Restoring '{}': {error}", target.display()));
                }
            }
            Ok(true) => errors.push(format!("Cannot restore occupied '{}'", target.display())),
            Err(error) => errors.push(format!("Inspecting '{}': {error}", target.display())),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn mirror_payload<T>(
    src: &Path,
    dst: &Path,
    include_config: bool,
    commit: impl FnOnce() -> Result<T, ConfigSyncError>,
) -> Result<T, ConfigSyncError> {
    require_source_dir(src)?;
    let parent = dst
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| std::io::Error::other(format!("No parent for '{}'", dst.display())))?;
    std::fs::create_dir_all(dst)?;
    if include_config {
        check_config_root(src)?;
        check_config_root(dst)?;
        let old_backup = dst.join(".config.mirror-backup");
        if entry_exists(&old_backup)? {
            return Err(std::io::Error::other(format!(
                "Previous config backup remains at '{}'",
                old_backup.display()
            ))
            .into());
        }
    }
    let name = dst
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("payload");
    let staging = parent.join(format!(".{name}.sync-stage"));
    let backup = parent.join(format!(".{name}.sync-backup"));
    // An existing backup belongs to an interrupted transaction and must survive for recovery.
    std::fs::create_dir(&backup).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("Cannot create backup '{}': {error}", backup.display()),
        )
    })?;
    let mut moved = Vec::new();
    let mut installed = Vec::new();
    let result = (|| {
        remove_if_exists(&staging)?;
        std::fs::create_dir(&staging)?;
        if include_config {
            std::fs::create_dir(staging.join("config"))?;
            if entry_exists(&src.join("config"))? {
                copy_dir_contents(
                    &src.join("config"),
                    &staging.join("config"),
                    src,
                    include_config,
                )?;
            }
        }
        for name in payload_names(src, false)? {
            let source = src.join(&name);
            let target = staging.join(&name);
            if std::fs::symlink_metadata(&source)?.file_type().is_symlink() {
                copy_payload_symlink(&source, &target, src, include_config)?;
            } else {
                std::fs::copy(&source, &target)?;
            }
        }
        for name in payload_names(dst, include_config)? {
            std::fs::rename(dst.join(&name), backup.join(&name))?;
            moved.push(name);
        }
        for name in payload_names(&staging, include_config)? {
            std::fs::rename(staging.join(&name), dst.join(&name))?;
            installed.push(name);
        }
        commit()
    })();
    let result = match result {
        Ok(value) => {
            cleanup_sync_path(&backup);
            Ok(value)
        }
        Err(error) => match rollback_payload(dst, &backup, &installed, &moved) {
            Ok(()) => {
                cleanup_sync_path(&backup);
                Err(error)
            }
            Err(rollback) => Err(ConfigSyncError::SaveRollback {
                save: error.to_string(),
                rollback: format!(
                    "{rollback}; recovery backup retained at '{}'",
                    backup.display()
                ),
            }),
        },
    };
    cleanup_sync_path(&staging);
    result
}

fn is_options_file(name: &str) -> bool {
    name == "options.txt" || name.starts_with("options") && name.ends_with(".txt")
}

fn remove_path(path: &Path) -> Result<(), ConfigSyncError> {
    let meta = std::fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if meta.file_type().is_symlink_dir() {
            std::fs::remove_dir(path)?;
            return Ok(());
        }
    }
    if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn copy_dir_contents(
    src: &Path,
    dst: &Path,
    payload: &Path,
    include_config: bool,
) -> Result<(), ConfigSyncError> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let source = entry.path();
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_dir_contents(&source, &target, payload, include_config)?;
        } else if file_type.is_file() {
            std::fs::copy(&source, &target)?;
        } else if file_type.is_symlink() {
            copy_payload_symlink(&source, &target, payload, include_config)?;
        }
    }

    Ok(())
}

fn copy_payload_symlink(
    source: &Path,
    destination: &Path,
    payload: &Path,
    include_config: bool,
) -> Result<(), ConfigSyncError> {
    let mut target = std::fs::read_link(source)?;
    if target.is_relative() {
        let relative = source
            .strip_prefix(payload)
            .map_err(std::io::Error::other)?;
        let mut position = relative.parent().unwrap_or(Path::new("")).to_owned();
        let mut internal = true;
        for component in target.components() {
            match component {
                std::path::Component::Normal(name) => position.push(name),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if !position.pop() {
                        internal = false;
                        break;
                    }
                }
                _ => {
                    internal = false;
                    break;
                }
            }
            if let Some(first) = position.components().next() {
                let name = first.as_os_str();
                if include_config && name == "config" {
                    continue;
                }
                if !is_options_file(&name.to_string_lossy()) {
                    internal = false;
                    break;
                }
                match std::fs::symlink_metadata(payload.join(name)) {
                    Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                    _ => {
                        internal = false;
                        break;
                    }
                }
            }
        }
        internal &= !position.as_os_str().is_empty();
        if !internal {
            // External links keep their source anchor; internal links move with the payload.
            let anchored = std::path::absolute(source.parent().unwrap().join(target))?;
            target = match (anchored.parent(), anchored.file_name()) {
                (Some(parent), Some(name)) => match std::fs::canonicalize(parent) {
                    Ok(parent) => parent.join(name),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => anchored,
                    Err(error) => return Err(error.into()),
                },
                _ => std::fs::canonicalize(anchored)?,
            };
        }
    }
    crate::storage::copy_symlink_target(source, destination, &target)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/config_sync.rs"]
mod tests;
