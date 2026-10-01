// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::io;
use std::path::{Path, PathBuf};

mod filesystem;
pub(crate) use filesystem::copy_symlink;
pub use filesystem::{write_atomic, write_atomic_private};

pub const MINECRAFT_DIR_NAME: &str = "minecraft";
pub const INSTANCE_STATE_DIR_NAME: &str = "rmcl";
pub const LAYOUT_VERSION: u32 = 2;

#[derive(Debug, Clone)]
pub struct InstancePaths {
    root: PathBuf,
}

impl InstancePaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn minecraft(&self) -> PathBuf {
        self.root.join(MINECRAFT_DIR_NAME)
    }

    pub fn state(&self) -> PathBuf {
        self.root.join(INSTANCE_STATE_DIR_NAME)
    }

    pub fn content(&self) -> PathBuf {
        self.state().join("content")
    }

    pub fn content_manifest(&self) -> PathBuf {
        self.content().join("manifest.json")
    }

    pub fn content_updates(&self) -> PathBuf {
        self.content().join("updates.json")
    }

    pub fn modpack_state(&self) -> PathBuf {
        self.state().join("modpack.json")
    }

    pub fn local_config(&self) -> PathBuf {
        self.content().join("config")
    }
}

#[derive(Debug, Clone)]
pub struct MetadataPaths {
    root: PathBuf,
}

impl MetadataPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    pub fn profiles(&self) -> PathBuf {
        self.state().join("profiles")
    }

    pub fn backups(&self) -> PathBuf {
        self.state().join("backups")
    }

    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn minecraft_cache(&self) -> PathBuf {
        self.cache().join("minecraft")
    }

    pub fn versions(&self) -> PathBuf {
        self.minecraft_cache().join("versions")
    }

    pub fn libraries(&self) -> PathBuf {
        self.minecraft_cache().join("libraries")
    }

    pub fn assets(&self) -> PathBuf {
        self.minecraft_cache().join("assets")
    }

    pub fn loader_profiles(&self) -> PathBuf {
        self.cache().join("loaders").join("profiles")
    }

    pub fn java_installations(&self) -> PathBuf {
        self.cache().join("java").join("installations.json")
    }

    pub fn provider_cache(&self, provider: &str) -> PathBuf {
        self.cache().join("providers").join(provider)
    }

    pub fn provider_projects(&self, provider: &str) -> PathBuf {
        self.provider_cache(provider).join("projects")
    }

    pub fn provider_versions(&self, provider: &str) -> PathBuf {
        self.provider_cache(provider).join("versions")
    }

    pub fn provider_icons(&self, provider: &str) -> PathBuf {
        self.provider_cache(provider).join("icons")
    }

    pub fn temporary(&self) -> PathBuf {
        self.root.join("tmp")
    }

    pub fn layout_marker(&self) -> PathBuf {
        self.state().join("layout.json")
    }

    pub fn migration_journal(&self) -> PathBuf {
        self.state().join("migration.json")
    }

    pub fn cache_rebuild_pending(&self) -> PathBuf {
        self.state().join("cache-rebuild.pending")
    }
}

pub fn clear_disposable_caches(meta_dir: &Path) -> io::Result<()> {
    let paths = MetadataPaths::new(meta_dir);
    for path in [paths.cache().join("providers"), paths.cache().join("java")] {
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn safe_relative_path(path: &Path) -> bool {
    let mut components = path.components();
    components
        .next()
        .is_some_and(|part| matches!(part, std::path::Component::Normal(_)))
        && components.all(|part| matches!(part, std::path::Component::Normal(_)))
}

#[cfg(test)]
#[path = "tests/storage.rs"]
mod tests;
