// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn curseforge_manifest_builds_import_summary() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pack.zip");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    use std::io::Write;
    zip.write_all(
        br#"{
            "name":"Example",
            "version":"1.0",
            "minecraft":{
                "version":"1.20.1",
                "modLoaders":[{"id":"forge-47.2.0","primary":true}]
            },
            "files":[{"projectID":1,"fileID":2,"required":true}],
            "overrides":"overrides"
        }"#,
    )
    .unwrap();
    zip.start_file(
        "overrides/config/example.txt",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"value").unwrap();
    zip.finish().unwrap();

    assert_eq!(
        super::super::detect_format(&path).unwrap(),
        PackFormat::CurseForge
    );
    let summary = build_summary(&path).unwrap();
    assert_eq!(summary.name, "Example");
    assert_eq!(summary.loader, ModLoader::Forge);
    assert_eq!(summary.loader_version.as_deref(), Some("47.2.0"));
    assert_eq!(summary.override_count, 1);
}

#[test]
fn empty_overrides_root_does_not_count_the_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pack.zip");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    use std::io::Write;
    zip.write_all(
        br#"{
            "name":"No Overrides",
            "version":"1.0",
            "minecraft":{"version":"1.20.1"},
            "overrides":""
        }"#,
    )
    .unwrap();
    zip.finish().unwrap();

    assert_eq!(build_summary(&path).unwrap().override_count, 0);
}

#[tokio::test]
async fn override_only_pack_tracks_files_under_its_configured_root() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pack.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    use std::io::Write;
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(
        br#"{"name":"Pack","version":"1","minecraft":{"version":"1.20.1"},"overrides":"client"}"#,
    )
    .unwrap();
    zip.start_file("client/config/pack.toml", options).unwrap();
    zip.write_all(b"pack").unwrap();
    zip.finish().unwrap();

    assert_eq!(
        owned_files(&path).await.unwrap(),
        vec![std::path::PathBuf::from("config/pack.toml")]
    );
    download_files(&parse(&path).unwrap(), &temp.path().join("minecraft"))
        .await
        .unwrap();
}
