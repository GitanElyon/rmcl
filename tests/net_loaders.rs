// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use rmcl::net::HttpClient;
use rmcl::net::fabric::{
    FabricLibrary, FabricProfile, download_fabric_libraries, fetch_fabric_game_versions_from,
    fetch_fabric_profile_from, fetch_fabric_versions_from,
};
use rmcl::net::forge::{fetch_forge_game_versions_from, fetch_forge_versions_from};
use rmcl::net::neoforge::{fetch_neoforge_game_versions_from, fetch_neoforge_versions_from};
use rmcl::net::quilt::{
    QuiltLibrary, QuiltProfile, download_quilt_libraries, fetch_quilt_game_versions_from,
    fetch_quilt_profile_from, fetch_quilt_versions_from,
};

#[tokio::test]
async fn forge_fetch_versions_filters_by_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/promotions.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "promos": {
                "1.20.1-latest": "47.10.0",
                "1.20.1-recommended": "47.9.0",
                "1.19.4-latest": "45.1.0",
                "1.7.10-latest": "10.13.4.1614"
            }
        })))
        .mount(&server)
        .await;

    let url = format!("{}/promotions.json", server.uri());
    let versions = fetch_forge_versions_from(&HttpClient::new(), &url, "1.20.1")
        .await
        .expect("forge versions");

    assert_eq!(versions, vec!["47.10.0", "47.9.0"]);
}

#[tokio::test]
async fn forge_fetch_game_versions_extracts_unique() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/promotions.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "promos": {
                "1.20.1-latest": "47.2.0",
                "1.20.1-recommended": "47.1.0",
                "1.19.4-latest": "45.1.0",
                "1.9.4-latest": "12.17.0.2317",
                "1.10-latest": "12.18.0.2000",
                "26.1.2-latest": "62.1.0"
            }
        })))
        .mount(&server)
        .await;

    let url = format!("{}/promotions.json", server.uri());
    let versions = fetch_forge_game_versions_from(&HttpClient::new(), &url)
        .await
        .expect("forge game versions");

    let ids: Vec<&str> = versions.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, vec!["26.1.2", "1.20.1", "1.19.4", "1.10", "1.9.4"]);
}

#[tokio::test]
async fn fabric_fetch_game_versions_parses_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/versions/game"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "version": "26.4-snapshot-2", "stable": false },
            { "version": "26.3", "stable": true },
            { "version": "26.3-rc-3", "stable": false },
            { "version": "26.3-snapshot-10", "stable": false },
            { "version": "26.3-snapshot-9", "stable": false },
            { "version": "24w01a", "stable": false },
            { "version": "1.20.1", "stable": true }
        ])))
        .mount(&server)
        .await;

    let versions = fetch_fabric_game_versions_from(&HttpClient::new(), &server.uri())
        .await
        .expect("fabric game versions");

    assert_eq!(
        versions.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        [
            "26.4-snapshot-2",
            "26.3",
            "26.3-rc-3",
            "26.3-snapshot-10",
            "26.3-snapshot-9",
            "24w01a",
            "1.20.1"
        ]
    );
    assert!(!versions[0].stable);
    assert!(versions[1].stable);
}

#[tokio::test]
async fn fabric_fetch_versions_parses_loader_entries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/versions/loader/1.20.1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "loader": { "version": "0.16.9", "stable": true },
                "intermediary": { "version": "1.20.1", "stable": true }
            },
            {
                "loader": { "version": "0.16.10", "stable": true },
                "intermediary": { "version": "1.20.1", "stable": true }
            },
            {
                "loader": { "version": "0.17.0-beta.2", "stable": false },
                "intermediary": { "version": "1.20.1", "stable": true }
            },
            {
                "loader": { "version": "0.7.8+build.9", "stable": true },
                "intermediary": { "version": "1.20.1", "stable": true }
            },
            {
                "loader": { "version": "0.7.8+build.10", "stable": true },
                "intermediary": { "version": "1.20.1", "stable": true }
            }
        ])))
        .mount(&server)
        .await;

    let versions = fetch_fabric_versions_from(&HttpClient::new(), &server.uri(), "1.20.1")
        .await
        .expect("fabric versions");

    assert_eq!(
        versions
            .iter()
            .map(|v| v.loader.version.as_str())
            .collect::<Vec<_>>(),
        [
            "0.17.0-beta.2",
            "0.16.10",
            "0.16.9",
            "0.7.8+build.10",
            "0.7.8+build.9"
        ]
    );
}

#[tokio::test]
async fn fabric_fetch_profile_parses_libraries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/versions/loader/1.20.1/0.15.0/profile/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "fabric-loader-0.15.0-1.20.1",
            "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
            "libraries": [
                { "name": "net.fabricmc:fabric-loader:0.15.0", "url": "https://maven.fabricmc.net/" }
            ]
        })))
        .mount(&server)
        .await;

    let profile = fetch_fabric_profile_from(&HttpClient::new(), &server.uri(), "1.20.1", "0.15.0")
        .await
        .expect("fabric profile");

    assert_eq!(profile.id, "fabric-loader-0.15.0-1.20.1");
    assert_eq!(
        profile.main_class,
        "net.fabricmc.loader.impl.launch.knot.KnotClient"
    );
    assert_eq!(profile.libraries.len(), 1);
    assert_eq!(profile.libraries[0].url, "https://maven.fabricmc.net/");
}

#[tokio::test]
async fn quilt_fetch_game_versions_parses_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/versions/game"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "version": "26.4-snapshot-1", "stable": false },
            { "version": "26.3", "stable": true },
            { "version": "26.3-rc-3", "stable": false },
            { "version": "24w01a", "stable": false },
            { "version": "1.20.1", "stable": true }
        ])))
        .mount(&server)
        .await;

    let versions = fetch_quilt_game_versions_from(&HttpClient::new(), &server.uri())
        .await
        .expect("quilt game versions");
    assert_eq!(
        versions.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        ["26.4-snapshot-1", "26.3", "26.3-rc-3", "24w01a", "1.20.1"]
    );
}

#[tokio::test]
async fn quilt_fetch_versions_parses_loader_entries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/versions/loader/1.20.1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "loader": { "version": "0.20.0-beta.9" } },
            { "loader": { "version": "0.30.0-beta.9" } },
            { "loader": { "version": "0.29.3" } },
            { "loader": { "version": "0.30.0-beta.10" } },
            { "loader": { "version": "0.30.0" } },
            { "loader": { "version": "0.23.0" } }
        ])))
        .mount(&server)
        .await;

    let versions = fetch_quilt_versions_from(&HttpClient::new(), &server.uri(), "1.20.1")
        .await
        .expect("quilt versions");
    assert_eq!(
        versions
            .iter()
            .map(|v| v.loader.version.as_str())
            .collect::<Vec<_>>(),
        [
            "0.30.0",
            "0.30.0-beta.10",
            "0.30.0-beta.9",
            "0.29.3",
            "0.23.0",
            "0.20.0-beta.9"
        ]
    );
}

#[tokio::test]
async fn quilt_fetch_profile_parses_libraries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/versions/loader/1.20.1/0.23.0/profile/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "quilt-loader-0.23.0-1.20.1",
            "mainClass": "org.quiltmc.loader.impl.launch.knot.KnotClient",
            "libraries": [
                { "name": "org.quiltmc:quilt-loader:0.23.0", "url": "https://maven.quiltmc.org/repository/release/" }
            ]
        })))
        .mount(&server)
        .await;

    let profile = fetch_quilt_profile_from(&HttpClient::new(), &server.uri(), "1.20.1", "0.23.0")
        .await
        .expect("quilt profile");

    assert_eq!(profile.id, "quilt-loader-0.23.0-1.20.1");
    assert_eq!(
        profile.libraries[0].url,
        "https://maven.quiltmc.org/repository/release/"
    );
}

#[tokio::test]
async fn fabric_and_quilt_download_libraries_into_the_shared_cache() {
    let server = MockServer::start().await;
    let temp = tempfile::tempdir().unwrap();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    use std::io::Write;
    zip.start_file(
        "META-INF/MANIFEST.MF",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"Manifest-Version: 1.0\n").unwrap();
    let jar = zip.finish().unwrap().into_inner();
    for name in ["fabric", "quilt"] {
        Mock::given(method("GET"))
            .and(path(format!("/example/{name}/1.0/{name}-1.0.jar")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(jar.clone()))
            .expect(1)
            .mount(&server)
            .await;
    }

    let fabric = FabricProfile {
        id: "fabric".into(),
        main_class: String::new(),
        libraries: vec![FabricLibrary {
            name: "example:fabric:1.0".into(),
            url: server.uri(),
        }],
    };
    let quilt = QuiltProfile {
        id: "quilt".into(),
        main_class: String::new(),
        libraries: vec![QuiltLibrary {
            name: "example:quilt:1.0".into(),
            url: server.uri(),
        }],
    };

    let client = HttpClient::new();
    download_fabric_libraries(&client, &fabric, temp.path())
        .await
        .unwrap();
    download_quilt_libraries(&client, &quilt, temp.path())
        .await
        .unwrap();
    download_fabric_libraries(&client, &fabric, temp.path())
        .await
        .unwrap();
    for name in ["fabric", "quilt"] {
        assert_eq!(
            std::fs::read(
                rmcl::storage::MetadataPaths::new(temp.path())
                    .libraries()
                    .join(format!("example/{name}/1.0/{name}-1.0.jar"))
            )
            .unwrap(),
            jar
        );
    }
}

#[tokio::test]
async fn fabric_replaces_a_truncated_cached_library() {
    let server = MockServer::start().await;
    let temp = tempfile::tempdir().unwrap();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    use std::io::Write;
    zip.start_file("valid.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"ok").unwrap();
    let jar = zip.finish().unwrap().into_inner();
    Mock::given(method("GET"))
        .and(path("/example/fabric/1.0/fabric-1.0.jar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(jar.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let profile = FabricProfile {
        id: "fabric".into(),
        main_class: String::new(),
        libraries: vec![FabricLibrary {
            name: "example:fabric:1.0".into(),
            url: server.uri(),
        }],
    };
    let destination = rmcl::storage::MetadataPaths::new(temp.path())
        .libraries()
        .join("example/fabric/1.0/fabric-1.0.jar");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::write(&destination, b"truncated").unwrap();

    download_fabric_libraries(&HttpClient::new(), &profile, temp.path())
        .await
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), jar);
}

#[tokio::test]
async fn neoforge_fetch_versions_filters_by_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/maven-api"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "versions": [
                "20.4.9",
                "20.4.180-beta",
                "20.4.150",
                "20.4.190",
                "21.0.10",
                "21.0.5-alpha"
            ]
        })))
        .mount(&server)
        .await;

    let url = format!("{}/maven-api", server.uri());
    let versions = fetch_neoforge_versions_from(&HttpClient::new(), &url, "1.20.4")
        .await
        .expect("neoforge versions");

    // game version "1.20.4" maps to prefix "20.4." and beta/alpha are excluded
    assert_eq!(versions, vec!["20.4.190", "20.4.150", "20.4.9"]);
}

#[tokio::test]
async fn neoforge_fetch_versions_supports_modern_minecraft_numbering() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/maven-api"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "versions": [
                "26.1.2.75",
                "26.1.2.76",
                "26.1.2.100",
                "26.1.1.15",
                "21.1.233"
            ]
        })))
        .mount(&server)
        .await;

    let url = format!("{}/maven-api", server.uri());
    let versions = fetch_neoforge_versions_from(&HttpClient::new(), &url, "26.1.2")
        .await
        .expect("neoforge versions");

    assert_eq!(versions, vec!["26.1.2.100", "26.1.2.76", "26.1.2.75"]);
}

#[tokio::test]
async fn neoforge_fetch_game_versions_reverse_engineers_mc_versions() {
    // neoforge "21.0.x" maps back to MC 1.21 (minor=0 strips the suffix);
    // "20.4.x" maps to MC 1.20.4. beta/alpha suffixes don't affect the
    // reverse mapping since the major/minor extraction happens first.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/maven-api"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "versions": [
                "26.1.2.76",
                "26.1.1.15",
                "21.0.10",
                "21.0.5",
                "20.4.190",
                "20.4.150"
            ]
        })))
        .mount(&server)
        .await;

    let url = format!("{}/maven-api", server.uri());
    let versions = fetch_neoforge_game_versions_from(&HttpClient::new(), &url)
        .await
        .expect("neoforge game versions");
    let ids: Vec<&str> = versions.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["26.1.2", "26.1.1", "1.21", "1.20.4"]);
    assert!(versions.iter().all(|v| v.stable));
}

#[tokio::test]
async fn neoforge_fetch_versions_rejects_invalid_game_version() {
    let server = MockServer::start().await;
    let url = format!("{}/maven-api", server.uri());
    let err = fetch_neoforge_versions_from(&HttpClient::new(), &url, "bogus")
        .await
        .expect_err("invalid game version");
    assert!(format!("{err:?}").contains("Invalid game version"));
}
