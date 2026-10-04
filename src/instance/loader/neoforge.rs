// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::{collections::BTreeMap, path::Path};

use async_trait::async_trait;

use super::{GameVersion, InstallError, InstallerError, ModLoaderInstaller};
use crate::feedback::progress::set_action;
use crate::instance::models::ModLoader;
use crate::net::{HttpClient, NetError, neoforge as neoforge_api};

pub struct NeoForgeInstaller;

#[async_trait]
impl ModLoaderInstaller for NeoForgeInstaller {
    fn loader_type(&self) -> ModLoader {
        ModLoader::NeoForge
    }

    async fn get_game_versions(&self, client: &HttpClient) -> Result<Vec<GameVersion>, NetError> {
        neoforge_api::fetch_neoforge_game_versions(client).await
    }

    async fn get_versions(
        &self,
        client: &HttpClient,
        game_version: &str,
    ) -> Result<Vec<String>, NetError> {
        tracing::debug!("Fetching NeoForge versions for Minecraft {}", game_version);
        let versions = neoforge_api::fetch_neoforge_versions(client, game_version).await?;
        tracing::debug!(
            "Fetched {} NeoForge version(s) for Minecraft {}",
            versions.len(),
            game_version
        );
        Ok(versions)
    }

    async fn install(
        &self,
        client: &HttpClient,
        game_version: &str,
        loader_version: &str,
        instance_dir: &Path,
        meta_dir: &Path,
    ) -> Result<(), InstallError> {
        let settings = crate::config::SETTINGS.read().clone();
        let environment = crate::instance::java::merge_environment(
            &settings.defaults.environment,
            &Default::default(),
        );
        self.install_with_java(
            client,
            game_version,
            loader_version,
            instance_dir,
            meta_dir,
            settings.paths.effective_java_path(),
            &environment,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn install_with_java(
        &self,
        client: &HttpClient,
        _game_version: &str,
        loader_version: &str,
        instance_dir: &Path,
        meta_dir: &Path,
        java_path: Option<&str>,
        environment: &BTreeMap<String, String>,
    ) -> Result<(), InstallError> {
        let installer_jar = instance_dir
            .join(crate::storage::MINECRAFT_DIR_NAME)
            .join("neoforge-installer.jar");
        tracing::info!("Installing NeoForge {}", loader_version);
        tracing::debug!("NeoForge installer path: {}", installer_jar.display());

        neoforge_api::download_neoforge_installer(client, loader_version, &installer_jar).await?;

        if let Err(e) =
            run_neoforge_installer(&installer_jar, instance_dir, java_path, environment).await
        {
            let _ = tokio::fs::remove_file(&installer_jar).await;
            return Err(InstallError::Installer(e));
        }

        if let Err(e) = tokio::fs::remove_file(&installer_jar).await {
            tracing::warn!("Failed to remove NeoForge installer JAR: {}", e);
        }

        save_neoforge_profile(instance_dir, meta_dir, loader_version)
            .map_err(InstallError::Installer)?;

        tracing::debug!("Installed NeoForge {}", loader_version);
        Ok(())
    }
}

pub async fn run_neoforge_installer(
    installer_path: &Path,
    instance_dir: &Path,
    java_path: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Result<(), InstallerError> {
    use tokio::process::Command;

    set_action("Running NeoForge installer...");
    let minecraft_dir = std::path::absolute(instance_dir.join(crate::storage::MINECRAFT_DIR_NAME))?;
    let java_path =
        crate::instance::java::resolve_java_path_in(java_path, &minecraft_dir, environment);
    tracing::debug!("Running NeoForge installer with Java {}", java_path);

    let output = match crate::instance::process::output(
        Command::new(&java_path)
            .arg("-jar")
            .arg(std::path::absolute(installer_path)?)
            .arg("--installClient")
            .arg(&minecraft_dir)
            .current_dir(&minecraft_dir)
            .envs(environment),
    )
    .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::debug!(
                "Failed to spawn NeoForge installer {} with Java {}: {}",
                installer_path.display(),
                java_path,
                e
            );
            return Err(InstallerError::Io(e));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().last().unwrap_or("").trim();
        tracing::debug!(
            "NeoForge installer {} failed with status {:?}: {}",
            installer_path.display(),
            output.status.code(),
            detail
        );
        return Err(InstallerError::ProcessFailed(format!(
            "NeoForge installer exited with {:?}",
            output.status.code()
        )));
    }

    tracing::debug!("NeoForge installer completed successfully");
    Ok(())
}

fn save_neoforge_profile(
    instance_dir: &Path,
    meta_dir: &Path,
    loader_version: &str,
) -> Result<(), super::InstallerError> {
    let version_dir_name = format!("neoforge-{loader_version}");
    let profile_filename = format!("neoforge-{loader_version}.json");
    super::save_installer_profile(instance_dir, meta_dir, &version_dir_name, &profile_filename)
}
