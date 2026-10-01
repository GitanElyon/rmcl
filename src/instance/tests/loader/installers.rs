// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[rstest::rstest]
#[case::vanilla(ModLoader::Vanilla)]
#[case::forge(ModLoader::Forge)]
#[case::neoforge(ModLoader::NeoForge)]
#[case::fabric(ModLoader::Fabric)]
#[case::quilt(ModLoader::Quilt)]
fn get_installer_returns_matching_loader_type(#[case] loader: ModLoader) {
    let installer = get_installer(loader);
    assert_eq!(installer.loader_type(), loader);
}

#[test]
fn save_installer_profile_copies_raw_bytes_verbatim() {
    use tempfile::TempDir;
    let tmp = TempDir::new().unwrap();
    let instance_dir = tmp.path().join("instance");
    let meta_dir = tmp.path().join("meta");

    // Preserve modern JVM arguments that older rmcl caches stripped.
    let installer_json = br#"{
            "id": "1.20.1-forge-47.2.0",
            "inheritsFrom": "1.20.1",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": [{ "name": "net.minecraftforge:forge:47.2.0" }],
            "arguments": {
                "game": ["--launchTarget", "forge_client"],
                "jvm": ["--add-opens", "java.base/sun.security.util=cpw.mods.securejarhandler"]
            }
        }"#;

    let ver_dir = instance_dir
        .join(crate::storage::MINECRAFT_DIR_NAME)
        .join("versions")
        .join("1.20.1-forge-47.2.0");
    std::fs::create_dir_all(&ver_dir).unwrap();
    let ver_json_path = ver_dir.join("1.20.1-forge-47.2.0.json");
    std::fs::write(&ver_json_path, installer_json).unwrap();

    save_installer_profile(
        &instance_dir,
        &meta_dir,
        "1.20.1-forge-47.2.0",
        "forge-1.20.1-47.2.0.json",
    )
    .unwrap();

    let saved = std::fs::read(
        meta_dir
            .join("cache/loaders/profiles")
            .join("forge-1.20.1-47.2.0.json"),
    )
    .unwrap();
    assert_eq!(
        saved,
        installer_json.to_vec(),
        "saved profile should be byte-for-byte identical to installer output"
    );
}

#[test]
fn save_installer_profile_rejects_invalid_json_without_overwriting_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let instance_dir = tmp.path().join("instance");
    let meta_dir = tmp.path().join("meta");
    let version_name = "1.20.1-forge-broken";
    let version_dir = instance_dir
        .join(crate::storage::MINECRAFT_DIR_NAME)
        .join("versions")
        .join(version_name);
    std::fs::create_dir_all(&version_dir).unwrap();
    std::fs::write(
        version_dir.join(format!("{version_name}.json")),
        b"not json",
    )
    .unwrap();
    let cached = crate::storage::MetadataPaths::new(&meta_dir)
        .loader_profiles()
        .join("forge-broken.json");
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, b"previous").unwrap();

    assert!(
        save_installer_profile(&instance_dir, &meta_dir, version_name, "forge-broken.json")
            .is_err()
    );
    assert_eq!(std::fs::read(cached).unwrap(), b"previous");
}

#[rstest::rstest]
#[case(ModLoader::Vanilla, None)]
#[case(ModLoader::Fabric, Some("fabric-1.21.1-1.0.json"))]
#[case(ModLoader::Quilt, Some("quilt-1.21.1-1.0.json"))]
#[case(ModLoader::Forge, Some("forge-1.21.1-1.0.json"))]
#[case(ModLoader::NeoForge, Some("neoforge-1.0.json"))]
fn profile_filenames_match_launch_cache_names(
    #[case] loader: ModLoader,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        profile_filename(loader, "1.21.1", "1.0").as_deref(),
        expected
    );
}

#[test]
fn legacy_forge_version_info_deserialises_as_launch_profile() {
    let bytes = br#"{
            "id": "1.7.10-Forge10.13.4.1614-1.7.10",
            "mainClass": "net.minecraft.launchwrapper.Launch",
            "minecraftArguments": "--username ${auth_player_name} --tweakClass cpw.mods.fml.common.launcher.FMLTweaker",
            "libraries": [
                { "name": "net.minecraftforge:forge:10.13.4.1614", "url": "http://files.minecraftforge.net/maven/" },
                { "name": "net.minecraft:launchwrapper:1.9" }
            ]
        }"#;

    let profile: crate::launch_profile::model::LaunchProfile =
        serde_json::from_slice(bytes).unwrap();
    assert_eq!(profile.id, "1.7.10-Forge10.13.4.1614-1.7.10");
    assert_eq!(
        profile.main_class.as_deref(),
        Some("net.minecraft.launchwrapper.Launch")
    );
    // legacy forge profiles omit inheritsFrom; the launch flow's
    // implicit fallback adds it before resolve.
    assert!(profile.inherits_from.is_none());
    assert!(
        profile
            .minecraft_arguments
            .as_deref()
            .unwrap()
            .contains("--tweakClass")
    );
    assert_eq!(profile.libraries.len(), 2);
    assert_eq!(
        profile.libraries[0].name,
        "net.minecraftforge:forge:10.13.4.1614"
    );
    assert_eq!(
        profile.libraries[0].url.as_deref(),
        Some("http://files.minecraftforge.net/maven/")
    );
    // legacy libs typically have no downloads.artifact; they resolve
    // at launch time via maven_coord_to_path(name).
    assert!(profile.libraries[0].downloads.is_none());
}

#[test]
fn raw_fabric_profile_bytes_parse_as_launch_profile() {
    let bytes = br#"{
            "id": "fabric-loader-0.14.21-1.20.1",
            "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
            "libraries": [
                { "name": "net.fabricmc:fabric-loader:0.14.21", "url": "https://maven.fabricmc.net/" },
                { "name": "net.fabricmc:intermediary:1.20.1", "url": "https://maven.fabricmc.net/" }
            ]
        }"#;

    let parsed: crate::launch_profile::model::LaunchProfile =
        serde_json::from_slice(bytes).unwrap();
    assert_eq!(parsed.id, "fabric-loader-0.14.21-1.20.1");
    assert_eq!(
        parsed.main_class.as_deref(),
        Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
    );
    // upstream Fabric profiles omit inheritsFrom; the launch flow's
    // implicit fallback handles it before resolve.
    assert!(parsed.inherits_from.is_none());
    assert!(parsed.arguments.is_none());
    assert_eq!(parsed.libraries.len(), 2);
    assert_eq!(
        parsed.libraries[0].url.as_deref(),
        Some("https://maven.fabricmc.net/")
    );
}

fn recording_java(temp: &Path) -> String {
    let path = temp.join(if cfg!(windows) {
        "installer java.cmd"
    } else {
        "installer java.sh"
    });
    #[cfg(windows)]
    std::fs::write(&path, "@echo off\r\n(echo %~1\r\necho %~2\r\necho %~3\r\necho %~4\r\n)>installer-args.txt\r\necho %CD%>installer-cwd.txt\r\n(echo %INSTALLER_CONTEXT%\r\necho %INSTALLER_GLOBAL%\r\n)>installer-env.txt\r\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            &path,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > installer-args.txt\npwd > installer-cwd.txt\nprintf '%s\\n' \"$INSTALLER_CONTEXT\" \"$INSTALLER_GLOBAL\" > installer-env.txt\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path.to_string_lossy().into_owned()
}

#[rstest::rstest]
#[case::forge(false)]
#[case::neoforge(true)]
#[tokio::test]
async fn installers_use_merged_environment_and_minecraft_cwd(#[case] neoforge: bool) {
    let temp = tempfile::tempdir().unwrap();
    let instance = temp.path().join("instance with spaces");
    let minecraft = instance.join(crate::storage::MINECRAFT_DIR_NAME);
    std::fs::create_dir_all(&minecraft).unwrap();
    let installer = minecraft.join("neoforge installer.jar");
    let java = recording_java(temp.path());
    let environment = crate::instance::java::merge_environment(
        &BTreeMap::from([
            ("PATH".to_owned(), "missing-global-java".to_owned()),
            ("INSTALLER_CONTEXT".to_owned(), "global".to_owned()),
            ("INSTALLER_GLOBAL".to_owned(), "global-only".to_owned()),
        ]),
        &BTreeMap::from([
            (
                "PATH".to_owned(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("INSTALLER_CONTEXT".to_owned(), "selected".to_owned()),
        ]),
    );
    let java_name = Path::new(&java).file_name().unwrap().to_str().unwrap();
    if neoforge {
        super::neoforge::run_neoforge_installer(
            &installer,
            &instance,
            Some(java_name),
            &environment,
        )
        .await
        .unwrap();
    } else {
        super::forge::run_forge_installer(&installer, &instance, Some(java_name), &environment)
            .await
            .unwrap();
    }
    let args = std::fs::read_to_string(minecraft.join("installer-args.txt")).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert_eq!(
        &args[..3],
        ["-jar", &installer.to_string_lossy(), "--installClient"]
    );
    if neoforge {
        assert_eq!(args[3], minecraft.to_string_lossy());
    } else {
        assert!(args[3..].iter().all(|arg| arg.trim().is_empty()));
    }
    let cwd = std::fs::read_to_string(minecraft.join("installer-cwd.txt")).unwrap();
    assert_eq!(
        Path::new(cwd.trim()).canonicalize().unwrap(),
        minecraft.canonicalize().unwrap()
    );
    assert!(!args.iter().any(|arg| arg.starts_with("-Duser.home=")));
    assert_eq!(
        std::fs::read_to_string(minecraft.join("installer-env.txt"))
            .unwrap()
            .lines()
            .map(str::trim)
            .collect::<Vec<_>>(),
        ["selected", "global-only"]
    );
}

#[tokio::test]
async fn cancelled_installers_do_not_leave_java_or_its_descendants_running() {
    let fixture = tempfile::tempdir().unwrap();
    let classes = fixture.path().join("classes");
    std::fs::create_dir_all(&classes).unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/InstallerLifetimeFixture.java");
    let compiler = std::process::Command::new("javac")
        .args(["-source", "8", "-target", "8"])
        .arg("-d")
        .arg(&classes)
        .arg(source)
        .output()
        .unwrap();
    assert!(
        compiler.status.success(),
        "{}",
        String::from_utf8_lossy(&compiler.stderr)
    );
    let jar = fixture.path().join("installer fixture.jar");
    let packer = std::process::Command::new("jar")
        .arg("cfe")
        .arg(&jar)
        .arg("InstallerLifetimeFixture")
        .arg("-C")
        .arg(&classes)
        .arg(".")
        .output()
        .unwrap();
    assert!(
        packer.status.success(),
        "{}",
        String::from_utf8_lossy(&packer.stderr)
    );
    let java = crate::instance::java::resolve_java_path_in(
        None,
        &std::env::current_dir().unwrap(),
        &Default::default(),
    );
    for neoforge in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let minecraft = temp.path().join(crate::storage::MINECRAFT_DIR_NAME);
        std::fs::create_dir_all(&minecraft).unwrap();
        {
            let future = async {
                if neoforge {
                    super::neoforge::run_neoforge_installer(
                        &jar,
                        temp.path(),
                        Some(&java),
                        &Default::default(),
                    )
                    .await
                } else {
                    super::forge::run_forge_installer(
                        &jar,
                        temp.path(),
                        Some(&java),
                        &Default::default(),
                    )
                    .await
                }
            };
            tokio::pin!(future);
            let ready = async {
                while !minecraft.join("installer-ready").is_file()
                    || !minecraft.join("installer-child-ready").is_file()
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            };
            tokio::select! {
                result = &mut future => panic!("installer exited before cancellation: {result:?}"),
                result = tokio::time::timeout(std::time::Duration::from_secs(15), ready) => result.unwrap(),
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(1600)).await;
        assert!(
            !minecraft.join("installer-survived").exists(),
            "Java process survived installer cancellation"
        );
        assert!(
            !minecraft.join("installer-child-survived").exists(),
            "Java descendant survived installer cancellation"
        );
    }
}
