// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use std::io::Write;

#[test]
fn oversized_pack_manifest_is_rejected() {
    assert_eq!(read_pack_manifest(&b"{}"[..]).unwrap(), b"{}");
    assert!(
        read_pack_manifest(std::io::repeat(0))
            .unwrap_err()
            .contains("8 MiB")
    );
}

fn make_pack_zip(tmp: &Path, name: &str, entries: &[(&str, &[u8])]) -> std::path::PathBuf {
    let path = tmp.join(name);
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions = Default::default();
    for (filename, bytes) in entries {
        zip.start_file(*filename, opts).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    path
}

#[test]
fn detect_format_recognises_mrpack() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_pack_zip(tmp.path(), "pack.mrpack", &[("modrinth.index.json", b"{}")]);
    assert_eq!(detect_format(&path), Ok(PackFormat::Mrpack));
}

#[test]
fn detect_format_recognises_mmc_flat() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_pack_zip(tmp.path(), "pack.zip", &[("mmc-pack.json", b"{}")]);
    assert_eq!(detect_format(&path), Ok(PackFormat::Mmc));
}

#[test]
fn detect_format_recognises_mmc_nested() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_pack_zip(tmp.path(), "pack.zip", &[("MyPack/mmc-pack.json", b"{}")]);
    assert_eq!(detect_format(&path), Ok(PackFormat::Mmc));
}

#[test]
fn detect_format_prefers_mrpack_when_both_markers_present() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_pack_zip(
        tmp.path(),
        "weird.zip",
        &[("modrinth.index.json", b"{}"), ("mmc-pack.json", b"{}")],
    );
    assert_eq!(detect_format(&path), Ok(PackFormat::Mrpack));
}

#[test]
fn detect_format_errors_on_unknown_archive() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_pack_zip(tmp.path(), "random.zip", &[("readme.txt", b"hello")]);
    let err = detect_format(&path).unwrap_err();
    assert!(
        err.contains("Unknown pack format"),
        "expected unknown format error, got: {err}"
    );
}

#[test]
fn detect_format_errors_on_missing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let err = detect_format(&tmp.path().join("missing.zip")).unwrap_err();
    assert!(err.contains("Cannot open"), "got: {err}");
}

#[test]
fn unique_name_no_collision() {
    let tmp = tempfile::tempdir().unwrap();
    let name = unique_instance_name("TestPack", tmp.path()).unwrap();
    assert_eq!(name, "TestPack");
}

#[test]
fn unique_name_with_collision() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("TestPack");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("instance.json"), "{}").unwrap();
    let name = unique_instance_name("TestPack", tmp.path()).unwrap();
    assert_eq!(name, "TestPack (2)");
}

#[test]
fn unique_name_preserves_directory_without_config() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("TestPack")).unwrap();

    assert_eq!(
        unique_instance_name("TestPack", tmp.path()).unwrap(),
        "TestPack (2)"
    );
    assert!(tmp.path().join("TestPack").exists());
}

#[test]
fn unique_name_multiple_collisions() {
    let tmp = tempfile::tempdir().unwrap();
    for suffix in ["", " (2)", " (3)"] {
        let dir = tmp.path().join(format!("TestPack{suffix}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("instance.json"), "{}").unwrap();
    }
    let name = unique_instance_name("TestPack", tmp.path()).unwrap();
    assert_eq!(name, "TestPack (4)");
}

#[test]
fn unique_name_keeps_searching_after_ninety_nine_collisions() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("Pack")).unwrap();
    for suffix in 2..=100 {
        std::fs::create_dir(tmp.path().join(format!("Pack ({suffix})"))).unwrap();
    }
    std::fs::create_dir(tmp.path().join("Pack (import)")).unwrap();
    assert_eq!(
        unique_instance_name("Pack", tmp.path()).unwrap(),
        "Pack (101)"
    );
}

#[test]
fn parse_project_url() {
    assert_eq!(
        parse_import_input("https://modrinth.com/modpack/fabulously-optimized"),
        ImportInput::ProjectSlug("fabulously-optimized".to_string())
    );
}

#[test]
fn parse_version_url() {
    assert_eq!(
        parse_import_input("https://modrinth.com/modpack/fabulously-optimized/version/abc123"),
        ImportInput::VersionId {
            slug: "fabulously-optimized".to_string(),
            version_id: "abc123".to_string(),
        }
    );
}

#[test]
fn parse_local_mrpack() {
    assert_eq!(
        parse_import_input("/home/user/pack.mrpack"),
        ImportInput::LocalFile("/home/user/pack.mrpack".to_string())
    );
}

#[test]
fn parse_local_zip() {
    assert_eq!(
        parse_import_input("GT_New_Horizons.zip"),
        ImportInput::LocalFile("GT_New_Horizons.zip".to_string())
    );
}

#[test]
fn parse_tilde_path() {
    assert_eq!(
        parse_import_input("~/Downloads/pack.mrpack"),
        ImportInput::LocalFile("~/Downloads/pack.mrpack".to_string())
    );
}

#[test]
fn parse_bare_slug() {
    assert_eq!(
        parse_import_input("fabulously-optimized"),
        ImportInput::ProjectSlug("fabulously-optimized".to_string())
    );
}

#[test]
fn parse_input_trims_whitespace() {
    assert_eq!(
        parse_import_input("  fabulously-optimized  "),
        ImportInput::ProjectSlug("fabulously-optimized".to_string())
    );
}

#[test]
fn windows_and_mixed_case_archive_inputs_stay_local() {
    for input in [
        r"C:\Downloads\Pack.ZIP",
        r"c:extensionless",
        r"C:/Downloads/Pack.MrPaCk",
        r"\\server\share\Pack.ZIP",
        r"\\server\share\extensionless",
        r"\Downloads\extensionless",
        r"\\?\C:\Downloads\Pack",
        r".\extensionless",
        "./extensionless",
        "../extensionless",
        "Pack.ZIP",
        "Pack.MRPACK",
        "~/Downloads/Pack.ZIP",
    ] {
        assert_eq!(
            parse_import_input(input),
            ImportInput::LocalFile(input.to_owned()),
            "{input}"
        );
    }
    assert_eq!(
        parse_import_input("https://modrinth.com/modpack/pack.zip"),
        ImportInput::ProjectSlug("pack.zip".to_owned())
    );
}

#[test]
fn imported_names_and_pack_paths_are_validated_before_joining() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["C:victim", "CON.txt", "trailing.", "../escape"] {
        assert!(unique_instance_name(name, temp.path()).is_err(), "{name}");
        cleanup_failed_import(
            &InstanceManager::new(temp.path(), temp.path().join("meta")),
            name,
        );
    }
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    std::fs::create_dir(temp.path().join("a".repeat(64))).unwrap();
    assert!(unique_instance_name(&"a".repeat(64), temp.path()).is_err());
    for path in [
        "../escape",
        "mods/../../escape",
        "C:victim",
        r"\\server\share\file",
        "/absolute",
        "mods/C:victim",
        "mods/NUL.jar",
        "mods/trailing. ",
        "mods/./file",
        "mods//file",
    ] {
        assert!(portable_pack_path(path).is_err(), "{path}");
    }
    assert_eq!(
        portable_pack_path(r"config\nested\pack.toml").unwrap(),
        "config/nested/pack.toml"
    );
    assert_eq!(
        portable_pack_path(".config/pack.toml").unwrap(),
        ".config/pack.toml"
    );
}

#[tokio::test]
async fn format_import_entry_points_validate_names_before_opening_archives() {
    let temp = tempfile::tempdir().unwrap();
    let manager = InstanceManager::new(temp.path().join("instances"), temp.path().join("meta"));
    let config = serde_json::from_value(serde_json::json!({
        "name": "C:victim", "game_version": "1.21", "loader": "vanilla", "created": "2026-01-01T00:00:00Z"
    })).unwrap();
    let summary = ImportSummary {
        name: "Pack".to_owned(),
        pack_version: "1".to_owned(),
        game_version: "1.21".to_owned(),
        loader: crate::instance::ModLoader::Vanilla,
        loader_version: None,
        mod_count: 0,
        override_count: 0,
        format: PackFormat::Mrpack,
        archive_path: temp.path().join("missing.zip"),
        source: None,
    };
    for error in [
        mrpack::execute_import(&summary, &manager, &config)
            .await
            .unwrap_err(),
        curseforge::execute_import(&summary, &manager, &config)
            .await
            .unwrap_err(),
        mmc::execute_import(&summary, &manager, &config)
            .await
            .unwrap_err(),
    ] {
        assert!(
            error.to_string().contains("Invalid instance name"),
            "{error}"
        );
    }
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn failed_import_cleanup_removes_the_partial_instance() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = InstanceManager::new(tmp.path().join("instances"), tmp.path().join("meta"));
    let instance_dir = manager.instances_dir.join("Broken");
    std::fs::create_dir_all(&instance_dir).unwrap();
    std::fs::write(instance_dir.join("partial"), b"data").unwrap();

    cleanup_failed_import(&manager, "Broken");

    assert!(!instance_dir.exists());
}

#[tokio::test]
async fn failed_import_creation_clears_progress_without_removing_existing_files() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = InstanceManager::new(tmp.path().join("instances"), tmp.path().join("meta"));
    let summary = ImportSummary {
        name: "../invalid".to_owned(),
        pack_version: String::new(),
        game_version: "1.20.1".to_owned(),
        loader: ModLoader::Vanilla,
        loader_version: None,
        mod_count: 0,
        override_count: 0,
        format: PackFormat::Mmc,
        archive_path: tmp.path().join("missing.zip"),
        source: None,
    };
    let preserved = tmp.path().join("invalid");
    std::fs::write(&preserved, b"keep").unwrap();

    assert!(execute_import(&summary, &manager).await.is_err());
    assert_eq!(std::fs::read(preserved).unwrap(), b"keep");
    assert!(
        !crate::feedback::progress::PROGRESS
            .lock()
            .unwrap()
            .current_action
            .as_deref()
            .is_some_and(|action| action.contains("../invalid"))
    );
}
