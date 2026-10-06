// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::instance::loader::GameVersion;
use crate::net::{HttpClient, NetError, download_loader_libraries};

const FABRIC_META_BASE: &str = "https://meta.fabricmc.net/v2";

#[derive(Debug, Clone, Deserialize)]
pub struct FabricLoaderVersion {
    pub loader: FabricVersion,
    pub intermediary: FabricVersion,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FabricVersion {
    pub version: String,
    #[serde(default)]
    pub stable: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FabricGameVersion {
    pub version: String,
    pub stable: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FabricProfile {
    pub id: String,
    pub main_class: String,
    pub libraries: Vec<FabricLibrary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FabricLibrary {
    pub name: String,
    pub url: String,
}

pub async fn fetch_fabric_game_versions(client: &HttpClient) -> Result<Vec<GameVersion>, NetError> {
    fetch_fabric_game_versions_from(client, FABRIC_META_BASE).await
}

pub async fn fetch_fabric_game_versions_from(
    client: &HttpClient,
    meta_base: &str,
) -> Result<Vec<GameVersion>, NetError> {
    let url = format!("{}/versions/game", meta_base);
    tracing::debug!("Fetching Fabric game versions from {}", url);
    let versions: Vec<FabricGameVersion> = client.get_json(&url).await?;
    tracing::debug!("Fetched {} Fabric game version(s)", versions.len());

    Ok(versions
        .into_iter()
        .map(|version| GameVersion {
            id: version.version,
            stable: version.stable,
        })
        .collect())
}

pub async fn fetch_fabric_versions(
    client: &HttpClient,
    game_version: &str,
) -> Result<Vec<FabricLoaderVersion>, NetError> {
    fetch_fabric_versions_from(client, FABRIC_META_BASE, game_version).await
}

pub async fn fetch_fabric_versions_from(
    client: &HttpClient,
    meta_base: &str,
    game_version: &str,
) -> Result<Vec<FabricLoaderVersion>, NetError> {
    let url = format!("{}/versions/loader/{}", meta_base, game_version);
    tracing::debug!("Fetching Fabric loader versions for {}", game_version);
    let mut versions: Vec<FabricLoaderVersion> = client.get_json(&url).await?;
    versions.sort_by(|a, b| {
        super::versions::compare_fabric_versions(&b.loader.version, &a.loader.version)
    });
    tracing::debug!(
        "Fetched {} Fabric loader version(s) for {}",
        versions.len(),
        game_version
    );
    Ok(versions)
}

pub async fn fetch_fabric_profile(
    client: &HttpClient,
    game_version: &str,
    loader_version: &str,
) -> Result<FabricProfile, NetError> {
    fetch_fabric_profile_from(client, FABRIC_META_BASE, game_version, loader_version).await
}

pub async fn fetch_fabric_profile_from(
    client: &HttpClient,
    meta_base: &str,
    game_version: &str,
    loader_version: &str,
) -> Result<FabricProfile, NetError> {
    let url = format!(
        "{}/versions/loader/{}/{}/profile/json",
        meta_base, game_version, loader_version
    );
    tracing::debug!(
        "Fetching Fabric profile for Minecraft {} loader {}",
        game_version,
        loader_version
    );
    client.get_json(&url).await
}

// like fetch_fabric_profile but also returns the raw response bytes so the
// caller can write the upstream JSON byte-for-byte to disk. used by the
// install path so we don't lose data (e.g. any future arguments field) by
// re-serializing through our narrow FabricProfile struct.
pub async fn fetch_fabric_profile_with_raw(
    client: &HttpClient,
    game_version: &str,
    loader_version: &str,
) -> Result<(FabricProfile, Vec<u8>), NetError> {
    fetch_fabric_profile_with_raw_from(client, FABRIC_META_BASE, game_version, loader_version).await
}

pub async fn fetch_fabric_profile_with_raw_from(
    client: &HttpClient,
    meta_base: &str,
    game_version: &str,
    loader_version: &str,
) -> Result<(FabricProfile, Vec<u8>), NetError> {
    let url = format!(
        "{}/versions/loader/{}/{}/profile/json",
        meta_base, game_version, loader_version
    );
    tracing::debug!(
        "Fetching raw Fabric profile for Minecraft {} loader {}",
        game_version,
        loader_version
    );
    client.get_json_with_raw(&url, "Fabric profile").await
}

pub async fn download_fabric_libraries(
    client: &HttpClient,
    profile: &FabricProfile,
    meta_dir: &Path,
) -> Result<(), NetError> {
    tracing::debug!(
        "Resolving {} Fabric libraries into {}",
        profile.libraries.len(),
        crate::storage::MetadataPaths::new(meta_dir)
            .libraries()
            .display()
    );
    let libraries: Vec<_> = profile
        .libraries
        .iter()
        .map(|lib| (lib.name.as_str(), lib.url.as_str()))
        .collect();
    download_loader_libraries(client, libraries, meta_dir, "Fabric").await?;

    tracing::debug!("Fabric library resolution complete for {}", profile.id);
    Ok(())
}
