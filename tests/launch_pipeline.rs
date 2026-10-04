// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::json;
use tempfile::TempDir;

use rmcl::instance::launch::{
    LaunchAuth, LaunchError, build_launch_invocation_with_settings, supports_quick_play,
};
use rmcl::instance::models::{InstanceConfig, ModLoader};

const PLAYER: &str = "TestPlayer";
const PLAYER_UUID: &str = "00000000-0000-0000-0000-000000000001";
const PLAYER_TOKEN: &str = "secret-access-token";

async fn build_launch_invocation(
    config: &InstanceConfig,
    instances_dir: &Path,
    meta_dir: &Path,
    auth: &LaunchAuth<'_>,
    quick_play_world: Option<&str>,
) -> Result<rmcl::instance::launch::LaunchInvocation, LaunchError> {
    build_launch_invocation_with_settings(
        config,
        instances_dir,
        meta_dir,
        auth,
        quick_play_world,
        &rmcl::config::Config::default(),
    )
    .await
}

fn test_auth() -> LaunchAuth<'static> {
    LaunchAuth {
        username: PLAYER,
        uuid: PLAYER_UUID,
        token: PLAYER_TOKEN,
        user_type: "msa",
    }
}

fn make_config(name: &str, game_version: &str, loader: ModLoader) -> InstanceConfig {
    make_config_with(name, game_version, loader, None)
}

fn make_config_with(
    name: &str,
    game_version: &str,
    loader: ModLoader,
    loader_version: Option<&str>,
) -> InstanceConfig {
    InstanceConfig {
        name: name.into(),
        game_version: game_version.into(),
        loader,
        loader_version: loader_version.map(str::to_owned),
        created: Utc::now(),
        last_played: None,
        java_path: Some(default_fake_java()),
        memory_max: None,
        memory_min: None,
        jvm_args: Vec::new(),
        environment: Default::default(),
        window_mode: Default::default(),
        inherit_window_mode: false,
        resolution: None,
        inherit_resolution: false,
        preferred_account: None,
        pre_launch_command: Default::default(),
        post_exit_command: Default::default(),
        glfw_path: None,
        config_sync_profile: None,
        modpack_source: None,
    }
}

fn fake_java(tmp: &TempDir, major: u32) -> String {
    fake_java_platform(tmp, major, &host_java_platform())
}

fn host_java_platform() -> rmcl::launch_profile::system::JavaPlatform {
    rmcl::launch_profile::system::JavaPlatform::from_java_properties(
        match std::env::consts::OS {
            "windows" => "Windows 10",
            "macos" => "Mac OS X",
            _ => "Linux",
        },
        std::env::consts::ARCH,
        if cfg!(target_pointer_width = "64") {
            "64"
        } else {
            "32"
        },
        "10.0",
    )
    .unwrap()
}

fn default_fake_java() -> String {
    static JAVA: std::sync::OnceLock<(TempDir, String)> = std::sync::OnceLock::new();
    JAVA.get_or_init(|| {
        let temp = tempfile::tempdir().unwrap();
        let java = fake_java(&temp, 25);
        (temp, java)
    })
    .1
    .clone()
}

fn fake_java_platform(
    tmp: &TempDir,
    major: u32,
    platform: &rmcl::launch_profile::system::JavaPlatform,
) -> String {
    let os_name = match platform.os_name {
        "windows" => "Windows 10",
        "osx" => "Mac OS X",
        _ => "Linux",
    };
    let text = format!(
        "java.version = {major}.0.1\nos.name = {os_name}\nos.arch = {}\nsun.arch.data.model = {}\nos.version = {}",
        platform.arch, platform.bitness, platform.os_version
    );
    let path = tmp.path().join(format!(
        "java probe-{major}-{}.{}",
        platform.arch,
        if cfg!(windows) { "cmd" } else { "sh" }
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' {} >&2\n",
                text.lines()
                    .map(|line| format!("'{line}'"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    std::fs::write(
        &path,
        format!(
            "@echo off\r\n{}\r\n",
            text.lines()
                .map(|line| format!("echo {line} 1>&2"))
                .collect::<Vec<_>>()
                .join("\r\n")
        ),
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

fn cache_fixture_artifacts(meta_dir: &Path, profile: &mut serde_json::Value) {
    use sha1::Digest;
    let bytes = b"synthetic jar";
    if let Some(libraries) = profile["libraries"].as_array_mut() {
        for library in libraries {
            let name = library["name"].as_str().unwrap().to_owned();
            if let Some(artifact) = library["downloads"]
                .get_mut("artifact")
                .filter(|value| value.is_object())
            {
                let path = artifact["path"]
                    .as_str()
                    .filter(|path| !path.is_empty())
                    .map(str::to_owned)
                    .or_else(|| rmcl::instance::loader::maven::maven_coord_to_path(&name))
                    .unwrap();
                let dest = rmcl::storage::MetadataPaths::new(meta_dir)
                    .libraries()
                    .join(path);
                std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                std::fs::write(dest, bytes).unwrap();
                artifact["sha1"] = json!(format!("{:x}", sha1::Sha1::digest(bytes)));
                artifact["size"] = json!(bytes.len());
            }
        }
    }
}

struct Fixture {
    _tmp: TempDir,
    instances_dir: PathBuf,
    meta_dir: PathBuf,
}

impl Fixture {
    fn new(instance_name: &str, game_version: &str, mut vanilla_meta: serde_json::Value) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let instances_dir = tmp.path().join("instances");
        let meta_dir = tmp.path().join("meta");

        let instance_minecraft = instances_dir.join(instance_name).join("minecraft");
        std::fs::create_dir_all(&instance_minecraft).unwrap();

        std::fs::create_dir_all(meta_dir.join("cache/minecraft/libraries")).unwrap();
        cache_fixture_artifacts(&meta_dir, &mut vanilla_meta);
        std::fs::create_dir_all(meta_dir.join("cache/loaders/profiles")).unwrap();
        let version_dir = meta_dir.join("cache/minecraft/versions").join(game_version);
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::write(
            version_dir.join("meta.json"),
            serde_json::to_vec_pretty(&vanilla_meta).unwrap(),
        )
        .unwrap();

        Self {
            _tmp: tmp,
            instances_dir,
            meta_dir,
        }
    }

    fn write_loader_profile(&self, filename: &str, mut content: serde_json::Value) {
        cache_fixture_artifacts(&self.meta_dir, &mut content);
        std::fs::write(
            self.meta_dir.join("cache/loaders/profiles").join(filename),
            serde_json::to_vec_pretty(&content).unwrap(),
        )
        .unwrap();
    }

    fn instance_libraries_dir(&self, instance_name: &str) -> PathBuf {
        self.instances_dir
            .join(instance_name)
            .join("minecraft")
            .join("libraries")
    }
}

fn modern_vanilla_meta(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "type": "release",
        "mainClass": "net.minecraft.client.main.Main",
        "assetIndex": {
            "id": "5",
            "url": "https://example.com/assets/index.json",
            "sha1": "0000000000000000000000000000000000000000"
        },
        "libraries": [
            {
                "name": "org.slf4j:slf4j-api:2.0.7",
                "downloads": {
                    "artifact": {
                        "url": "https://example.com/slf4j.jar",
                        "path": "org/slf4j/slf4j-api/2.0.7/slf4j-api-2.0.7.jar",
                        "sha1": "0000000000000000000000000000000000000000",
                        "size": 0
                    }
                }
            }
        ],
        "arguments": {
            "game": [
                "--username", "${auth_player_name}",
                "--version", "${version_name}",
                "--uuid", "${auth_uuid}",
                "--accessToken", "${auth_access_token}",
                "--userType", "${user_type}",
                "--versionType", "${version_type}"
            ],
            "jvm": [
                "-Djava.library.path=${natives_directory}",
                "-Dminecraft.launcher.brand=${launcher_name}"
            ]
        }
    })
}

fn modern_vanilla_meta_with_java(id: &str, java_major: u32) -> serde_json::Value {
    let mut meta = modern_vanilla_meta(id);
    meta["javaVersion"] = json!({
        "component": format!("java-runtime-{java_major}"),
        "majorVersion": java_major
    });
    meta
}

fn legacy_vanilla_meta(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "type": "release",
        "mainClass": "net.minecraft.launchwrapper.Launch",
        "assetIndex": {
            "id": "1.7.10",
            "url": "https://example.com/assets/index.json",
            "sha1": "0000000000000000000000000000000000000000"
        },
        "libraries": [],
        "minecraftArguments": "--username ${auth_player_name} --uuid ${auth_uuid} --accessToken ${auth_access_token} --userType ${user_type}"
    })
}

fn platform_classpath_sep() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

#[tokio::test]
async fn vanilla_modern_builds_complete_invocation() {
    let fx = Fixture::new("v1", "1.20.1", modern_vanilla_meta("1.20.1"));
    let config = make_config("v1", "1.20.1", ModLoader::Vanilla);

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert_eq!(inv.java, config.java_path.as_deref().unwrap());
    assert_eq!(inv.main_class, "net.minecraft.client.main.Main");
    assert!(inv.extra_args.is_empty());

    let expected_natives = host_java_platform().natives_directory(&fx.meta_dir, "1.20.1");
    let actual_natives = inv
        .jvm_args
        .iter()
        .find_map(|arg| arg.strip_prefix("-Djava.library.path="))
        .map(Path::new);
    assert_eq!(
        actual_natives,
        Some(expected_natives.as_path()),
        "jvm_args missing natives substitution: {:?}",
        inv.jvm_args,
    );
    assert!(
        inv.jvm_args
            .iter()
            .any(|a| a == "-Dminecraft.launcher.brand=rmcl"),
        "jvm_args missing launcher brand: {:?}",
        inv.jvm_args
    );

    assert_eq!(
        inv.working_dir,
        fx.instances_dir.join("v1").join("minecraft")
    );
    let slf4j = fx
        .meta_dir
        .join("cache/minecraft/libraries/org/slf4j/slf4j-api/2.0.7/slf4j-api-2.0.7.jar");
    let client_jar = fx
        .meta_dir
        .join("cache/minecraft/versions/1.20.1/1.20.1.jar");
    assert!(inv.classpath.contains(&slf4j));
    assert!(inv.classpath.contains(&client_jar));
}

#[tokio::test]
async fn quick_play_passes_the_selected_save_folder() {
    let mut meta = modern_vanilla_meta("1.20.1");
    meta["arguments"]["game"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "rules": [{
                "action": "allow",
                "features": { "is_quick_play_singleplayer": true }
            }],
            "value": ["--quickPlaySingleplayer", "${quickPlaySingleplayer}"]
        }));
    let fx = Fixture::new("quick", "1.20.1", meta);
    let world_dir = fx.instances_dir.join("quick/minecraft/saves/Display World");
    std::fs::create_dir_all(&world_dir).unwrap();
    let config = make_config("quick", "1.20.1", ModLoader::Vanilla);

    assert!(supports_quick_play(&fx.meta_dir, "1.20.1"));
    let invocation = build_launch_invocation(
        &config,
        &fx.instances_dir,
        &fx.meta_dir,
        &test_auth(),
        Some("Display World"),
    )
    .await
    .unwrap();

    assert!(
        invocation
            .game_args
            .windows(2)
            .any(|args| args == ["--quickPlaySingleplayer", "Display World"])
    );
}

#[tokio::test]
async fn launch_fails_when_selected_java_is_older_than_profile_requires() {
    let fx = Fixture::new(
        "java-old",
        "26.1.2",
        modern_vanilla_meta_with_java("26.1.2", 25),
    );
    let mut config = make_config("java-old", "26.1.2", ModLoader::Vanilla);
    config.java_path = Some(fake_java(&fx._tmp, 21));

    let err = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .expect_err("java 21 should fail for a Java 25 profile");

    assert!(
        matches!(
            err,
            LaunchError::JavaTooOld {
                required: 25,
                detected: 21,
                ..
            }
        ),
        "expected JavaTooOld, got {err:?}"
    );
}

#[tokio::test]
async fn vanilla_legacy_args_format_substitutes_tokens() {
    let fx = Fixture::new("vlegacy", "1.7.10", legacy_vanilla_meta("1.7.10"));
    let config = make_config("vlegacy", "1.7.10", ModLoader::Vanilla);

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert_eq!(inv.main_class, "net.minecraft.launchwrapper.Launch");

    assert_eq!(inv.jvm_args.len(), 3);
    assert!(inv.jvm_args[2].starts_with("-Djava.library.path="));
    assert!(inv.jvm_args[0].starts_with("-Xms"));
    assert!(inv.jvm_args[1].starts_with("-Xmx"));

    let joined = inv.game_args.join(" ");
    assert!(joined.contains(&format!("--username {}", PLAYER)));
    assert!(joined.contains(&format!("--uuid {}", PLAYER_UUID)));
    assert!(joined.contains(&format!("--accessToken {}", PLAYER_TOKEN)));
    assert!(joined.contains("--userType msa"));
}

#[tokio::test]
async fn missing_or_empty_artifact_paths_use_maven_coordinates() {
    for path in [None, Some("")] {
        let mut meta = modern_vanilla_meta("1.20.1");
        let artifact = meta["libraries"][0]["downloads"]["artifact"]
            .as_object_mut()
            .unwrap();
        if let Some(path) = path {
            artifact.insert("path".to_owned(), json!(path));
        } else {
            artifact.remove("path");
        }
        let fixture = Fixture::new("artifact", "1.20.1", meta);
        let invocation = build_launch_invocation(
            &make_config("artifact", "1.20.1", ModLoader::Vanilla),
            &fixture.instances_dir,
            &fixture.meta_dir,
            &test_auth(),
            None,
        )
        .await
        .unwrap();
        assert!(
            invocation.classpath.contains(
                &rmcl::storage::MetadataPaths::new(&fixture.meta_dir)
                    .libraries()
                    .join("org/slf4j/slf4j-api/2.0.7/slf4j-api-2.0.7.jar")
            )
        );
    }
}

#[tokio::test]
async fn forge_modern_includes_add_opens() {
    let fx = Fixture::new("f1", "1.20.1", modern_vanilla_meta("1.20.1"));
    fx.write_loader_profile(
        "forge-1.20.1-47.2.0.json",
        json!({
            "id": "1.20.1-forge-47.2.0",
            "inheritsFrom": "1.20.1",
            "type": "release",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": [
                { "name": "net.minecraftforge:fmlloader:1.20.1-47.2.0" }
            ],
            "arguments": {
                "game": ["--launchTarget", "forge_client"],
                "jvm": [
                    "--add-opens",
                    "java.base/sun.security.util=ALL-UNNAMED",
                    "-DignoreList=bootstraplauncher,securejarhandler"
                ]
            }
        }),
    );
    let config = make_config_with("f1", "1.20.1", ModLoader::Forge, Some("47.2.0"));

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert_eq!(
        inv.main_class,
        "cpw.mods.bootstraplauncher.BootstrapLauncher"
    );
    assert!(
        inv.jvm_args.iter().any(|a| a == "--add-opens"),
        "Forge --add-opens missing: {:?}",
        inv.jvm_args
    );
    assert!(
        inv.jvm_args
            .iter()
            .any(|a| a == "java.base/sun.security.util=ALL-UNNAMED"),
        "Forge --add-opens target missing: {:?}",
        inv.jvm_args
    );
    assert!(
        inv.game_args
            .windows(2)
            .any(|w| w == ["--launchTarget", "forge_client"]),
        "Forge launchTarget missing from game_args: {:?}",
        inv.game_args
    );
}

#[tokio::test]
async fn forge_local_lib_dir_preferred_over_meta_dir() {
    let fx = Fixture::new("f2", "1.20.1", modern_vanilla_meta("1.20.1"));
    fx.write_loader_profile(
        "forge-1.20.1-47.2.0.json",
        json!({
            "id": "1.20.1-forge-47.2.0",
            "inheritsFrom": "1.20.1",
            "type": "release",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": [
                { "name": "net.minecraftforge:fmlloader:1.20.1-47.2.0" }
            ],
            "arguments": { "game": [], "jvm": [] }
        }),
    );

    // drop fmlloader into the instance-local libraries dir so the launcher
    // finds it there rather than under the shared meta cache.
    let local_lib = fx
        .instance_libraries_dir("f2")
        .join("net/minecraftforge/fmlloader/1.20.1-47.2.0");
    std::fs::create_dir_all(&local_lib).unwrap();
    let local_jar = local_lib.join("fmlloader-1.20.1-47.2.0.jar");
    std::fs::write(&local_jar, b"jar").unwrap();

    let config = make_config_with("f2", "1.20.1", ModLoader::Forge, Some("47.2.0"));
    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert!(
        inv.classpath.contains(&local_jar),
        "expected local fmlloader on classpath: {:?}",
        inv.classpath
    );
    let meta_candidate = rmcl::storage::MetadataPaths::new(&fx.meta_dir)
        .libraries()
        .join("net/minecraftforge/fmlloader/1.20.1-47.2.0/fmlloader-1.20.1-47.2.0.jar");
    assert!(
        !inv.classpath.contains(&meta_candidate),
        "meta-dir candidate should not be on classpath when local exists"
    );
}

#[tokio::test]
async fn fabric_implicit_inheritsfrom_resolves() {
    let fx = Fixture::new("fab", "1.20.1", modern_vanilla_meta("1.20.1"));
    // Fabric upstream profiles omit inheritsFrom; the launch flow patches it
    // in before resolving. assert that the merge picks up Fabric's main class.
    fx.write_loader_profile(
        "fabric-1.20.1-0.15.0.json",
        json!({
            "id": "fabric-loader-0.15.0-1.20.1",
            "type": "release",
            "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
            "libraries": [
                { "name": "net.fabricmc:fabric-loader:0.15.0" }
            ]
        }),
    );
    let config = make_config_with("fab", "1.20.1", ModLoader::Fabric, Some("0.15.0"));

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert_eq!(
        inv.main_class,
        "net.fabricmc.loader.impl.launch.knot.KnotClient"
    );
    let fabric_loader = fx.meta_dir.join(
        "cache/minecraft/libraries/net/fabricmc/fabric-loader/0.15.0/fabric-loader-0.15.0.jar",
    );
    let slf4j = fx
        .meta_dir
        .join("cache/minecraft/libraries/org/slf4j/slf4j-api/2.0.7/slf4j-api-2.0.7.jar");
    assert!(
        inv.classpath.contains(&fabric_loader),
        "fabric-loader missing from classpath: {:?}",
        inv.classpath
    );
    assert!(
        inv.classpath.contains(&slf4j),
        "vanilla slf4j missing from classpath: {:?}",
        inv.classpath
    );
}

#[tokio::test]
async fn neoforge_inheritsfrom_resolves() {
    let fx = Fixture::new("ne", "1.20.6", modern_vanilla_meta("1.20.6"));
    fx.write_loader_profile(
        "neoforge-20.4.190.json",
        json!({
            "id": "neoforge-20.4.190",
            "inheritsFrom": "1.20.6",
            "type": "release",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": [
                { "name": "net.neoforged:neoforge:20.4.190" }
            ],
            "arguments": {
                "game": [],
                "jvm": ["--add-modules", "ALL-MODULE-PATH"]
            }
        }),
    );
    let config = make_config_with("ne", "1.20.6", ModLoader::NeoForge, Some("20.4.190"));

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert_eq!(
        inv.main_class,
        "cpw.mods.bootstraplauncher.BootstrapLauncher"
    );
    assert!(
        inv.jvm_args
            .windows(2)
            .any(|w| w == ["--add-modules", "ALL-MODULE-PATH"]),
        "NeoForge --add-modules missing: {:?}",
        inv.jvm_args
    );
}

#[tokio::test]
async fn auth_credentials_substituted_in_game_args() {
    let fx = Fixture::new("auth", "1.20.1", modern_vanilla_meta("1.20.1"));
    let config = make_config("auth", "1.20.1", ModLoader::Vanilla);

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert!(
        inv.game_args.iter().any(|a| a == PLAYER),
        "auth_player_name not substituted: {:?}",
        inv.game_args
    );
    assert!(
        inv.game_args.iter().any(|a| a == PLAYER_UUID),
        "auth_uuid not substituted: {:?}",
        inv.game_args
    );
    assert!(
        inv.game_args.iter().any(|a| a == PLAYER_TOKEN),
        "auth_access_token not substituted: {:?}",
        inv.game_args
    );
    assert!(
        inv.game_args.iter().any(|a| a == "msa"),
        "user_type not substituted: {:?}",
        inv.game_args
    );
}

#[tokio::test]
async fn version_type_substituted() {
    let mut meta = modern_vanilla_meta("1.20.1");
    meta["type"] = json!("snapshot");
    let fx = Fixture::new("vt", "1.20.1", meta);
    let config = make_config("vt", "1.20.1", ModLoader::Vanilla);

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert!(
        inv.game_args.iter().any(|a| a == "snapshot"),
        "version_type not substituted to 'snapshot': {:?}",
        inv.game_args
    );
}

#[tokio::test]
async fn classpath_uses_platform_separator() {
    let fx = Fixture::new("cp", "1.20.1", modern_vanilla_meta("1.20.1"));
    let config = make_config("cp", "1.20.1", ModLoader::Vanilla);

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert_eq!(inv.classpath.len(), 2);
    let sep = platform_classpath_sep();
    assert_eq!(inv.classpath_string.matches(sep).count(), 1);
}

#[tokio::test]
async fn rule_disallow_excludes_library() {
    let mut meta = modern_vanilla_meta("1.20.1");
    meta["libraries"] = json!([
        {
            "name": "org.slf4j:slf4j-api:2.0.7",
            "downloads": {
                "artifact": {
                    "url": "",
                    "path": "org/slf4j/slf4j-api/2.0.7/slf4j-api-2.0.7.jar",
                    "sha1": "0000000000000000000000000000000000000000",
                    "size": 0
                }
            }
        },
        {
            "name": "com.denied:lib:1.0",
            "rules": [{ "action": "disallow" }],
            "downloads": {
                "artifact": {
                    "url": "",
                    "path": "com/denied/lib/1.0/lib-1.0.jar",
                    "sha1": "0000000000000000000000000000000000000000",
                    "size": 0
                }
            }
        }
    ]);
    let fx = Fixture::new("rule", "1.20.1", meta);
    let config = make_config("rule", "1.20.1", ModLoader::Vanilla);

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    let denied = rmcl::storage::MetadataPaths::new(&fx.meta_dir)
        .libraries()
        .join("com/denied/lib/1.0/lib-1.0.jar");
    assert!(
        !inv.classpath.contains(&denied),
        "denied library was included: {:?}",
        inv.classpath
    );
}

#[tokio::test]
async fn xms_xmx_use_config_memory() {
    let fx = Fixture::new("mem", "1.20.1", modern_vanilla_meta("1.20.1"));
    let mut config = make_config("mem", "1.20.1", ModLoader::Vanilla);
    config.memory_min = Some("1G".into());
    config.memory_max = Some("4G".into());

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert!(inv.jvm_args.iter().any(|a| a == "-Xms1G"));
    assert!(inv.jvm_args.iter().any(|a| a == "-Xmx4G"));
}

#[tokio::test]
async fn instance_launch_options_are_injected() {
    let fx = Fixture::new("options", "1.20.1", modern_vanilla_meta("1.20.1"));
    let mut config = make_config("options", "1.20.1", ModLoader::Vanilla);
    config.window_mode = rmcl::instance::WindowMode::Fullscreen;
    config.glfw_path = Some("/usr/lib/libglfw.so.3".to_owned());
    config
        .environment
        .insert("MESA_LOADER_DRIVER_OVERRIDE".to_owned(), "zink".to_owned());

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert!(
        inv.game_args
            .iter()
            .any(|argument| argument == "--fullscreen")
    );
    assert!(
        inv.jvm_args
            .iter()
            .any(|argument| argument == "-Dorg.lwjgl.glfw.libname=/usr/lib/libglfw.so.3")
    );
    assert_eq!(
        inv.environment
            .get("MESA_LOADER_DRIVER_OVERRIDE")
            .map(String::as_str),
        Some("zink")
    );
}

#[tokio::test]
async fn default_memory_used_when_unset() {
    let fx = Fixture::new("memdef", "1.20.1", modern_vanilla_meta("1.20.1"));
    let config = make_config("memdef", "1.20.1", ModLoader::Vanilla);

    let inv = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap();

    assert!(inv.jvm_args.iter().any(|a| a == "-Xms512M"));
    assert!(inv.jvm_args.iter().any(|a| a == "-Xmx2G"));
}

#[tokio::test]
async fn meta_not_found_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let instances_dir = tmp.path().join("instances");
    let meta_dir = tmp.path().join("meta");
    std::fs::create_dir_all(instances_dir.join("ghost").join("minecraft")).unwrap();
    std::fs::create_dir_all(meta_dir.join("cache/minecraft/versions")).unwrap();

    let config = make_config("ghost", "1.20.1", ModLoader::Vanilla);
    let err = build_launch_invocation(&config, &instances_dir, &meta_dir, &test_auth(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, LaunchError::MetaNotFound(_)),
        "expected MetaNotFound, got: {err:?}"
    );
}

#[tokio::test]
async fn loader_profile_missing_returns_error() {
    let fx = Fixture::new("lpmiss", "1.20.1", modern_vanilla_meta("1.20.1"));
    let config = make_config_with("lpmiss", "1.20.1", ModLoader::Fabric, Some("0.15.0"));

    let err = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, LaunchError::MetaNotFound(_)),
        "expected MetaNotFound for missing loader profile, got: {err:?}"
    );
}

#[tokio::test]
async fn merged_profile_missing_main_class_fails() {
    // a meta.json that deserialises cleanly but lacks the required mainClass
    // must surface as a Parse error; otherwise the launcher would try to
    // unwrap None and crash later.
    let mut meta = modern_vanilla_meta("1.20.1");
    meta.as_object_mut().unwrap().remove("mainClass");
    let fx = Fixture::new("nomc", "1.20.1", meta);
    let config = make_config("nomc", "1.20.1", ModLoader::Vanilla);

    let err = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        matches!(err, LaunchError::Parse(_)) && msg.contains("mainClass"),
        "expected Parse error mentioning mainClass, got: {err:?}"
    );
}

#[tokio::test]
async fn legacy_loader_profile_missing_loader_version_errors() {
    // an instance with no loader_version recorded must error out cleanly
    // when its loader profile is in the legacy stripped shape (no
    // inheritsFrom, no arguments, no minecraftArguments, but has the rmcl
    // 0.3.0-era gameArguments field).
    let fx = Fixture::new("nolv", "1.20.1", modern_vanilla_meta("1.20.1"));
    fx.write_loader_profile(
        "forge-1.20.1-unknown.json",
        json!({
            "id": "1.20.1-forge",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": [],
            "gameArguments": ["--launchTarget", "forge_client"]
        }),
    );

    let mut config = make_config("nolv", "1.20.1", ModLoader::Forge);
    config.loader_version = None;

    let err = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        matches!(err, LaunchError::Parse(_)) && msg.contains("Reinstall"),
        "expected Parse error asking for reinstall, got: {err:?}"
    );
}

#[tokio::test]
async fn legacy_loader_profile_missing_installer_json_errors() {
    // legacy stripped profile + loader_version, but the installer JSON the
    // migration would copy from isn't present on disk. Parse error.
    let fx = Fixture::new("noinst", "1.20.1", modern_vanilla_meta("1.20.1"));
    fx.write_loader_profile(
        "forge-1.20.1-47.2.0.json",
        json!({
            "id": "1.20.1-forge-47.2.0",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": [],
            "gameArguments": ["--launchTarget", "forge_client"]
        }),
    );

    let config = make_config_with("noinst", "1.20.1", ModLoader::Forge, Some("47.2.0"));

    let err = build_launch_invocation(&config, &fx.instances_dir, &fx.meta_dir, &test_auth(), None)
        .await
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        matches!(err, LaunchError::Parse(_))
            && msg.contains("installer JSON")
            && msg.contains("missing"),
        "expected Parse error about missing installer JSON, got: {err:?}"
    );
}

#[tokio::test]
async fn rules_and_natives_directory_follow_the_selected_java_properties() {
    for (os, arch, bits, normalized) in [
        ("Windows 10", "x86", "32", "x86"),
        ("Windows 10", "amd64", "64", "x86_64"),
        ("Mac OS X", "x86_64", "64", "x86_64"),
        ("Mac OS X", "aarch64", "64", "arm64"),
        ("Linux", "amd64", "64", "x86_64"),
    ] {
        let platform = rmcl::launch_profile::system::JavaPlatform::from_java_properties(
            os, arch, bits, "10.0",
        )
        .unwrap();
        let mut meta = modern_vanilla_meta("1.16.5");
        for cpu in ["x86", "x86_64", "arm64"] {
            meta["libraries"].as_array_mut().unwrap().push(json!({
                "name":format!("test:only-{cpu}:1"), "rules":[{"action":"allow", "os":{"arch":cpu}}],
            }));
        }
        meta["arguments"]["jvm"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "rules":[{"action":"allow", "os":{"name":"windows", "version":"^10\\."}}],
                "value":["-Dos.name=Windows 10", "-Dos.version=10.0"],
            }));
        let fixture = Fixture::new("java rules", "1.16.5", meta);
        let mut config = make_config("java rules", "1.16.5", ModLoader::Vanilla);
        config.java_path = Some(fake_java_platform(&fixture._tmp, 25, &platform));
        let invocation = build_launch_invocation(
            &config,
            &fixture.instances_dir,
            &fixture.meta_dir,
            &test_auth(),
            None,
        )
        .await
        .unwrap();
        let cpu_jars: Vec<_> = invocation
            .classpath
            .iter()
            .filter(|path| path.to_string_lossy().contains("only-"))
            .collect();
        assert_eq!(cpu_jars.len(), 1);
        assert!(cpu_jars[0].ends_with(format!("only-{normalized}-1.jar")));
        assert!(invocation.jvm_args.contains(&format!(
                "-Djava.library.path={}",
                platform
                    .natives_directory(&fixture.meta_dir, "1.16.5")
                    .display()
            )));
        assert_eq!(
            invocation
                .jvm_args
                .contains(&"-Dos.version=10.0".to_owned()),
            platform.os_name == "windows"
        );
    }
}

fn zip_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn install_test_lwjgl3ify(fixture: &Fixture, base_url: &str) {
    use sha1::Digest;
    let bytes = b"declared library bytes";
    let mut libraries = Vec::new();
    for module in ["lwjgl", "lwjgl-sdl", "lwjgl-spng"] {
        libraries.push(json!({"name":format!("org.lwjgl:{module}:3.4.2"), "downloads":{"artifact":{
            "url":format!("{base_url}/{module}-3.4.2.jar"), "sha1":format!("{:x}", sha1::Sha1::digest(bytes)), "size":bytes.len(),
        }}}));
        for (classifier, os) in [
            ("natives-linux", "linux"),
            ("natives-linux-arm64", "linux"),
            ("natives-macos", "osx"),
            ("natives-macos-arm64", "osx"),
            ("natives-windows", "windows"),
            ("natives-windows-x86", "windows"),
            ("natives-windows-arm64", "windows"),
        ] {
            libraries.push(
                json!({"name":format!("org.lwjgl:{module}-{classifier}:3.4.2"),
                "rules":[{"action":"allow", "os":{"name":os}}], "downloads":{"artifact":{
                    "url":format!("{base_url}/{module}-3.4.2-{classifier}.jar"),
                    "sha1":format!("{:x}", sha1::Sha1::digest(bytes)), "size":bytes.len(),
                }}}),
            );
        }
    }
    let payload = zip_entries(&[("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\r\n")]);
    libraries.push(json!({"name":"com.github.GTNewHorizons:lwjgl3ify:3.0.35:forgePatches", "downloads":{"artifact":{
        "url":format!("{base_url}/forgePatches.jar"), "sha1":format!("{:x}", sha1::Sha1::digest(&payload)), "size":payload.len(),
    }}}));
    let profile = json!({"id":"lwjgl3ify-client", "mainClass":"com.gtnewhorizons.retrofuturabootstrap.MainStartOnFirstThread",
        "javaVersion":{"majorVersion":25}, "libraries":libraries,
        "arguments":{"game":["--username", "${auth_player_name}", "--tweakClass", "metadata.ClientTweaker"], "jvm":[
            {"rules":[{"action":"allow", "os":{"name":"osx"}}], "value":"-XstartOnFirstThread"},
            "--add-opens", "java.base/java.lang=ALL-UNNAMED", "--enable-native-access", "ALL-UNNAMED",
            "-Djava.library.path=${natives_directory}", "-cp", "${classpath}",
            "-Djava.system.class.loader=com.gtnewhorizons.retrofuturabootstrap.RfbSystemClassLoader",
        ]},
    });
    let mods = fixture.instances_dir.join("patched/minecraft/mods");
    std::fs::create_dir_all(&mods).unwrap();
    std::fs::write(
        mods.join("lwjgl3ify-3.0.35.jar"),
        zip_entries(&[
            (
                "me/eigenraven/lwjgl3ify/relauncher/version.json",
                &serde_json::to_vec(&profile).unwrap(),
            ),
            (
                "me/eigenraven/lwjgl3ify/relauncher/forgePatches.zip",
                &payload,
            ),
        ]),
    )
    .unwrap();
    fixture.write_loader_profile("forge-1.7.10-10.13.4.1614.json", json!({
        "id":"forge-1.7.10", "inheritsFrom":"1.7.10", "mainClass":"net.minecraft.launchwrapper.Launch",
        "libraries":[{"name":"net.minecraft:launchwrapper:1.12"}, {"name":"org.lwjgl.lwjgl:lwjgl-platform:2.9.4"}],
        "arguments":{"game":["--tweakClass", "old.parent.Tweaker"], "jvm":["-Dold.parent.jvm=true"]},
    }));
}

#[tokio::test]
async fn lwjgl3ify_empty_cache_downloads_exact_metadata_and_renders_client_startup() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    for (os, arch, bits, classifier) in [
        ("Linux", "amd64", "64", "natives-linux"),
        ("Linux", "aarch64", "64", "natives-linux-arm64"),
        ("Mac OS X", "amd64", "64", "natives-macos"),
        ("Mac OS X", "aarch64", "64", "natives-macos-arm64"),
        ("Windows 10", "x86", "32", "natives-windows-x86"),
        ("Windows 10", "amd64", "64", "natives-windows"),
        ("Windows 10", "aarch64", "64", "natives-windows-arm64"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(b"declared library bytes".to_vec()),
            )
            .expect(6)
            .mount(&server)
            .await;
        let fixture = Fixture::new("patched", "1.7.10", legacy_vanilla_meta("1.7.10"));
        install_test_lwjgl3ify(&fixture, &server.uri());
        let platform = rmcl::launch_profile::system::JavaPlatform::from_java_properties(
            os, arch, bits, "10.0",
        )
        .unwrap();
        let mut config =
            make_config_with("patched", "1.7.10", ModLoader::Forge, Some("10.13.4.1614"));
        config.java_path = Some(fake_java_platform(&fixture._tmp, 25, &platform));
        for _ in 0..2 {
            let invocation = build_launch_invocation(
                &config,
                &fixture.instances_dir,
                &fixture.meta_dir,
                &test_auth(),
                None,
            )
            .await
            .unwrap();
            assert_eq!(invocation.main_class, "RmclShim");
            assert_eq!(
                invocation.extra_args,
                ["com.gtnewhorizons.retrofuturabootstrap.MainStartOnFirstThread"]
            );
            assert_eq!(
                invocation
                    .jvm_args
                    .contains(&"-XstartOnFirstThread".to_owned()),
                platform.os_name == "osx"
            );
            assert!(
                invocation
                    .jvm_args
                    .windows(2)
                    .any(|args| args == ["--enable-native-access", "ALL-UNNAMED"])
            );
            assert!(
                invocation
                    .jvm_args
                    .windows(2)
                    .any(|args| args[0] == "-cp" && args[1] == invocation.classpath_string)
            );
            assert!(
                invocation
                    .game_args
                    .contains(&"metadata.ClientTweaker".to_owned())
            );
            assert!(
                !invocation
                    .game_args
                    .contains(&"old.parent.Tweaker".to_owned())
            );
            assert!(
                !invocation
                    .jvm_args
                    .contains(&"-Dold.parent.jvm=true".to_owned())
            );
            assert!(invocation.game_args.contains(&PLAYER.to_owned()));
            let natives: Vec<_> = invocation
                .classpath
                .iter()
                .filter(|path| path.to_string_lossy().contains("-natives-"))
                .collect();
            assert_eq!(natives.len(), 3);
            assert!(natives.iter().all(|path| {
                path.to_string_lossy()
                    .contains(&format!("-{classifier}-3.4.2.jar"))
            }));
            for path in invocation.classpath.iter().filter(|path| {
                path.to_string_lossy().contains("org/lwjgl")
                    || path.to_string_lossy().contains("org\\lwjgl")
            }) {
                assert_eq!(std::fs::read(path).unwrap(), b"declared library bytes");
            }
            assert!(
                !invocation
                    .classpath
                    .iter()
                    .any(|path| path.to_string_lossy().contains("2.9.4")
                        || path.to_string_lossy().contains("launchwrapper-"))
            );
        }
        server.verify().await;
    }
}

#[tokio::test]
async fn lwjgl3ify_download_failure_does_not_launch_with_missing_libraries() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let fixture = Fixture::new("patched", "1.7.10", legacy_vanilla_meta("1.7.10"));
    install_test_lwjgl3ify(&fixture, &server.uri());
    let config = make_config_with("patched", "1.7.10", ModLoader::Forge, Some("10.13.4.1614"));
    let error = build_launch_invocation(
        &config,
        &fixture.instances_dir,
        &fixture.meta_dir,
        &test_auth(),
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, LaunchError::Download(_)), "{error}");
}

#[tokio::test]
async fn selected_java_is_resolved_and_probed_in_the_launch_environment_and_cwd() {
    let fixture = Fixture::new("context", "1.20.1", modern_vanilla_meta("1.20.1"));
    let minecraft = fixture.instances_dir.join("context/minecraft");
    std::fs::write(minecraft.join("expected-cwd"), b"ready").unwrap();
    let bin = fixture._tmp.path().join("custom java bin");
    std::fs::create_dir_all(&bin).unwrap();
    let name = if cfg!(windows) {
        "context-java.cmd"
    } else {
        "context-java"
    };
    let java = bin.join(name);
    let original = fake_java(&fixture._tmp, 25);
    let mut script = std::fs::read_to_string(original).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        script = script.replacen("#!/bin/sh\n", "#!/bin/sh\n[ \"$PROBE_CONTEXT\" = instance ] && [ -f expected-cwd ] || exit 77\nprintf '%s' \"$PROBE_CONTEXT\" > probe-context\n", 1);
        std::fs::write(&java, &script).unwrap();
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    {
        script = script.replacen("@echo off\r\n", "@echo off\r\nif not \"%PROBE_CONTEXT%\"==\"instance\" exit /b 77\r\nif not exist expected-cwd exit /b 78\r\necho %PROBE_CONTEXT%>probe-context\r\n", 1);
        std::fs::write(&java, &script).unwrap();
    }
    let mut settings = rmcl::config::Config::default();
    settings
        .defaults
        .environment
        .insert("PATH".into(), "missing-global-path".into());
    settings
        .defaults
        .environment
        .insert("PROBE_CONTEXT".into(), "global".into());
    let mut config = make_config("context", "1.20.1", ModLoader::Vanilla);
    config.java_path = Some(name.into());
    config.environment.insert(
        if cfg!(windows) { "path" } else { "PATH" }.into(),
        bin.to_string_lossy().into_owned(),
    );
    config.environment.insert(
        if cfg!(windows) {
            "probe_context"
        } else {
            "PROBE_CONTEXT"
        }
        .into(),
        "instance".into(),
    );
    let invocation = build_launch_invocation_with_settings(
        &config,
        &fixture.instances_dir,
        &fixture.meta_dir,
        &test_auth(),
        None,
        &settings,
    )
    .await
    .unwrap();
    assert_eq!(
        Path::new(&invocation.java).canonicalize().unwrap(),
        java.canonicalize().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(minecraft.join("probe-context"))
            .unwrap()
            .trim(),
        "instance"
    );
    assert_eq!(invocation.working_dir, minecraft);
    let cwd = invocation.working_dir.clone();
    let environment = invocation.environment.clone();
    let installation = tokio::task::spawn_blocking(move || {
        rmcl::instance::java::inspect_installation_in(Path::new(name), &cwd, &environment)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        installation.path.canonicalize().unwrap(),
        java.canonicalize().unwrap()
    );
    assert_eq!(installation.version.as_deref(), Some("25.0.1"));
    if cfg!(windows) {
        assert_eq!(
            invocation
                .environment
                .keys()
                .filter(|key| key.eq_ignore_ascii_case("PATH"))
                .count(),
            1
        );
    }
}
