// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

//! Release-gate smoke tests for live upstream APIs.
//!
//! These are intentionally ignored during normal test runs. The release
//! workflow runs them with `cargo test --all-targets -- --ignored`.

use rmcl::instance::{ContentKind, ModLoader, VanillaInstaller};
use rmcl::net::{self, HttpClient};

fn numeric_core(version: &str) -> Vec<u64> {
    version
        .split(['-', '+'])
        .next()
        .unwrap()
        .split('.')
        .map(|part| part.parse().unwrap())
        .collect()
}

fn assert_published_newest_first(versions: &[net::modrinth::VersionInfo]) {
    let dates = versions
        .iter()
        .map(|version| chrono::DateTime::parse_from_rfc3339(&version.date_published).unwrap())
        .collect::<Vec<_>>();
    assert!(dates.windows(2).all(|pair| pair[0] >= pair[1]));
}

#[tokio::test]
#[ignore = "hits live Fabric API"]
async fn fabric_versions_are_available() {
    let client = HttpClient::new();
    let versions = net::fabric::fetch_fabric_versions(&client, "1.20.1")
        .await
        .unwrap();

    assert!(!versions.is_empty());
    assert!(versions[0].loader.version.contains('.'));
    assert!(
        versions
            .windows(2)
            .all(|pair| numeric_core(&pair[0].loader.version)
                >= numeric_core(&pair[1].loader.version))
    );
}

#[tokio::test]
#[ignore = "hits live Fabric API"]
async fn fabric_game_versions_are_available() {
    let versions = net::fabric::fetch_fabric_game_versions(&HttpClient::new())
        .await
        .unwrap();

    assert!(versions.iter().any(|version| version.id == "1.20.1"));
}

#[tokio::test]
#[ignore = "hits live Quilt API"]
async fn quilt_versions_are_available() {
    let versions = net::quilt::fetch_quilt_versions(&HttpClient::new(), "1.20.1")
        .await
        .unwrap();

    assert!(!versions.is_empty());
    assert!(
        versions
            .windows(2)
            .all(|pair| numeric_core(&pair[0].loader.version)
                >= numeric_core(&pair[1].loader.version))
    );
}

#[tokio::test]
#[ignore = "hits live Quilt API"]
async fn quilt_game_versions_are_available() {
    let versions = net::quilt::fetch_quilt_game_versions(&HttpClient::new())
        .await
        .unwrap();

    assert!(versions.iter().any(|version| version.id == "1.20.1"));
}

#[tokio::test]
#[ignore = "hits live Forge API"]
async fn forge_versions_are_available() {
    let versions = net::forge::fetch_forge_versions(&HttpClient::new(), "1.20.1")
        .await
        .unwrap();

    assert!(!versions.is_empty());
    assert!(
        versions
            .windows(2)
            .all(|pair| numeric_core(&pair[0]) >= numeric_core(&pair[1]))
    );
}

#[tokio::test]
#[ignore = "hits live Forge API"]
async fn forge_game_versions_are_available() {
    let versions = net::forge::fetch_forge_game_versions(&HttpClient::new())
        .await
        .unwrap();

    assert!(versions.iter().any(|version| version.id == "1.20.1"));
}

#[tokio::test]
#[ignore = "hits live NeoForge API"]
async fn neoforge_versions_are_available() {
    let versions = net::neoforge::fetch_neoforge_versions(&HttpClient::new(), "1.21")
        .await
        .unwrap();

    assert!(!versions.is_empty());
    assert!(
        versions
            .windows(2)
            .all(|pair| numeric_core(&pair[0]) >= numeric_core(&pair[1]))
    );
}

#[tokio::test]
#[ignore = "hits live NeoForge API"]
async fn neoforge_game_versions_are_available() {
    let versions = net::neoforge::fetch_neoforge_game_versions(&HttpClient::new())
        .await
        .unwrap();

    assert!(versions.iter().any(|version| version.id == "1.21"));
}

#[tokio::test]
#[ignore = "hits live Mojang API"]
async fn vanilla_installer_lists_game_versions() {
    use rmcl::instance::ModLoaderInstaller;

    let versions = VanillaInstaller
        .get_game_versions(&HttpClient::new())
        .await
        .unwrap();

    assert!(
        versions
            .iter()
            .any(|version| version.id == "1.20.1" && version.stable)
    );
}

#[tokio::test]
#[ignore = "hits live Mojang, Fabric and Quilt APIs"]
async fn instance_game_version_pickers_preserve_minecraft_chronology() {
    let manifest = net::mojang::fetch_version_manifest(&HttpClient::new())
        .await
        .unwrap();
    let order = manifest
        .versions
        .iter()
        .enumerate()
        .map(|(index, version)| (version.id.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();
    for loader in [ModLoader::Vanilla, ModLoader::Fabric, ModLoader::Quilt] {
        let versions = rmcl::tui::widgets::popups::version_lists::game_versions(loader)
            .await
            .unwrap();
        let indices = versions
            .iter()
            .filter_map(|version| order.get(version.id.as_str()))
            .collect::<Vec<_>>();
        assert!(
            indices.windows(2).all(|pair| pair[0] < pair[1]),
            "{loader:?} reordered Minecraft releases/snapshots"
        );
    }
}

#[tokio::test]
#[ignore = "hits live Modrinth API"]
async fn modrinth_project_is_available() {
    let project = net::modrinth::fetch_project(&HttpClient::new(), "fabulously-optimized")
        .await
        .unwrap();

    assert_eq!(project.slug, "fabulously-optimized");
    assert!(!project.title.is_empty());
}

#[tokio::test]
#[ignore = "hits live Modrinth API"]
async fn modrinth_versions_are_available() {
    let versions = net::modrinth::fetch_versions(&HttpClient::new(), "fabulously-optimized")
        .await
        .unwrap();

    assert!(!versions.is_empty());
    assert!(!versions[0].files.is_empty());
    assert_published_newest_first(&versions);
}

#[tokio::test]
#[ignore = "hits live Modrinth API"]
async fn modrinth_discovery_returns_unique_compatible_mods() {
    use std::collections::HashSet;

    use rmcl::net::modrinth::search_discovery;

    let client = HttpClient::new();
    let filters = rmcl::instance::content::provider::DiscoverySearchFilters {
        game_versions: vec!["1.21.1".to_owned()],
        ..Default::default()
    };
    let result = search_discovery(
        &client,
        ContentKind::Mod,
        "sodium",
        &filters,
        ModLoader::Fabric,
        rmcl::instance::content::provider::DiscoverySort::Relevance,
        false,
        0,
        100,
    )
    .await
    .unwrap();

    assert!(!result.projects.is_empty());
    assert!(result.total_hits >= result.projects.len());
    let icon_url = result
        .projects
        .iter()
        .find_map(|project| project.icon_url.as_deref())
        .expect("expected at least one project icon");
    let icon = client
        .get_bytes_limited(icon_url, net::MAX_PROVIDER_ASSET_BYTES)
        .await
        .unwrap();
    assert!(image::load_from_memory(&icon).is_ok());

    let mut seen = result
        .projects
        .iter()
        .map(|project| project.id.clone())
        .collect::<HashSet<_>>();
    for offset in (100..result.total_hits.min(300)).step_by(100) {
        let next = search_discovery(
            &client,
            ContentKind::Mod,
            "sodium",
            &filters,
            ModLoader::Fabric,
            rmcl::instance::content::provider::DiscoverySort::Relevance,
            false,
            offset,
            100,
        )
        .await
        .unwrap();
        assert!(!next.projects.is_empty(), "empty page at offset {offset}");
        assert!(
            next.projects
                .iter()
                .all(|project| seen.insert(project.id.clone())),
            "duplicate project at offset {offset}"
        );
    }
}

#[tokio::test]
#[ignore = "hits live CurseForge API"]
async fn curseforge_discovery_returns_compatible_mods() {
    let api_key = net::curseforge::api_key().expect("build has no CurseForge API key");
    let result = net::curseforge::search_discovery(
        &HttpClient::new(),
        api_key,
        ContentKind::Mod,
        "sodium",
        &["1.21.1".to_owned()],
        ModLoader::Fabric,
        &[],
        rmcl::instance::content::provider::DiscoverySort::Relevance,
        false,
        0,
        20,
    )
    .await
    .unwrap();

    assert!(!result.projects.is_empty());
    assert!(result.total_hits >= result.projects.len());
    assert!(result.projects.iter().all(|project| !project.id.is_empty()));
    let versions = net::curseforge::fetch_versions(
        &HttpClient::new(),
        api_key,
        &result.projects[0].id,
        "1.21.1",
        Some(ModLoader::Fabric),
    )
    .await
    .unwrap();
    assert!(!versions.is_empty());
    assert_published_newest_first(&versions);
}
