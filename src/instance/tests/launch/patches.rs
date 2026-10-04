// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use crate::launch_profile::model::Library;
use serde_json::json;
use std::io::Write;

fn make_zip(tmp: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
    let path = tmp.join(name);
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (filename, bytes) in entries {
        zip.start_file(*filename, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    path
}

fn lib(name: &str) -> Library {
    Library {
        name: name.to_owned(),
        ..Default::default()
    }
}

#[test]
fn strip_replaced_libs_removes_dominated_prefixes() {
    let mut profile = LaunchProfile {
        libraries: [
            "net.minecraft:launchwrapper:1.12",
            "org.ow2.asm:asm-all:5.0.3",
            "org.lwjgl.lwjgl:lwjgl:2.9.4",
            "org.lwjgl.lwjgl:lwjgl_util:2.9.4",
            "org.apache.commons:commons-compress:1.4.1",
            "commons-io:commons-io:2.4",
            "com.google.guava:guava:15.0",
            "org.apache.logging.log4j:log4j-core:2.0",
            "com.google.guava:guava:21.0",
        ]
        .into_iter()
        .map(lib)
        .collect(),
        ..Default::default()
    };
    strip_replaced_libs(&mut profile);
    assert_eq!(
        profile
            .libraries
            .iter()
            .map(|lib| lib.name.as_str())
            .collect::<Vec<_>>(),
        [
            "org.apache.logging.log4j:log4j-core:2.0",
            "com.google.guava:guava:21.0",
        ]
    );
}

#[test]
fn strip_replaced_libs_keeps_unrelated_entries() {
    let mut profile = LaunchProfile {
        libraries: vec![
            lib("org.apache.logging.log4j:log4j-core:2.0"),
            lib("org.spongepowered:mixin:0.8.5"),
        ],
        ..Default::default()
    };
    let original = profile.clone();
    strip_replaced_libs(&mut profile);
    assert_eq!(profile, original);
}

fn bundled_profile() -> LaunchProfile {
    // Same three-part native coordinates and overlapping OS rules as release 3.0.35.
    let mut libraries = Vec::new();
    for module in ["lwjgl", "lwjgl-sdl", "lwjgl-spng"] {
        libraries.push(json!({"name":format!("org.lwjgl:{module}:3.4.2"), "downloads":{"artifact":{
            "url":format!("https://example.invalid/{module}-3.4.2.jar"), "sha1":"0".repeat(40), "size":1,
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
            libraries.push(json!({"name":format!("org.lwjgl:{module}-{classifier}:3.4.2"),
                "rules":[{"action":"allow", "os":{"name":os}}], "downloads":{"artifact":{
                    "url":format!("https://example.invalid/{module}-3.4.2-{classifier}.jar"), "sha1":"0".repeat(40), "size":1,
                }}}));
        }
    }
    serde_json::from_value(json!({"id":"lwjgl3ify-client", "mainClass":"com.gtnewhorizons.retrofuturabootstrap.MainStartOnFirstThread",
        "javaVersion":{"majorVersion":25}, "libraries":libraries,
        "arguments":{"game":["--username", "${auth_player_name}"], "jvm":[
            {"rules":[{"action":"allow", "os":{"name":"osx"}}], "value":"-XstartOnFirstThread"},
            "--add-opens", "java.base/java.lang=ALL-UNNAMED",
            "-Djava.system.class.loader=com.gtnewhorizons.retrofuturabootstrap.RfbSystemClassLoader",
        ]},
    })).unwrap()
}

#[test]
fn bundled_natives_follow_selected_java_on_all_platforms() {
    for (os, arch, bits, expected) in [
        ("Linux", "amd64", "64", "natives-linux"),
        ("Linux", "aarch64", "64", "natives-linux-arm64"),
        ("Mac OS X", "amd64", "64", "natives-macos"),
        ("Mac OS X", "aarch64", "64", "natives-macos-arm64"),
        ("Windows 10", "x86", "32", "natives-windows-x86"),
        ("Windows 10", "amd64", "64", "natives-windows"),
        ("Windows 10", "aarch64", "64", "natives-windows-arm64"),
    ] {
        let platform = JavaPlatform::from_java_properties(os, arch, bits, "10.0").unwrap();
        let mut profile = bundled_profile();
        select_lwjgl_natives(&mut profile, &platform).unwrap();
        assert_eq!(profile.libraries.len(), 6);
        let natives: Vec<_> = profile
            .libraries
            .iter()
            .filter(|lib| lib.name.contains("-natives-"))
            .collect();
        assert_eq!(natives.len(), 3);
        assert!(
            natives
                .iter()
                .all(|lib| lib.name.ends_with(&format!("-{expected}:3.4.2")))
        );
    }
    let mut profile = bundled_profile();
    profile
        .libraries
        .retain(|lib| !lib.name.contains("lwjgl-spng-natives-windows-x86"));
    let platform = JavaPlatform::from_java_properties("Windows 10", "x86", "32", "10.0").unwrap();
    assert!(
        select_lwjgl_natives(&mut profile, &platform)
            .unwrap_err()
            .to_string()
            .contains("lwjgl-spng")
    );
}

fn write_mod(temp: &Path, mut profile: LaunchProfile) -> PathBuf {
    use sha1::Digest;
    let payload = make_zip(
        temp,
        "patches.zip",
        &[("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\r\n")],
    );
    let payload = std::fs::read(payload).unwrap();
    profile.libraries.push(serde_json::from_value(json!({"name":"com.github.GTNewHorizons:lwjgl3ify:3.0.35:forgePatches",
        "downloads":{"artifact":{"url":"https://example.invalid/forgePatches.jar", "sha1":format!("{:x}", sha1::Sha1::digest(&payload)), "size":payload.len()}}
    })).unwrap());
    let mods = temp.join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    make_zip(
        &mods,
        "lwjgl3ify-3.0.35.jar",
        &[
            (
                "me/eigenraven/lwjgl3ify/relauncher/version.json",
                &serde_json::to_vec(&profile).unwrap(),
            ),
            (
                "me/eigenraven/lwjgl3ify/relauncher/forgePatches.zip",
                &payload,
            ),
        ],
    )
}

#[tokio::test]
async fn shim_forwards_the_metadata_client_entrypoint_and_payload_is_a_jar() {
    let temp = tempfile::tempdir().unwrap();
    write_mod(temp.path(), bundled_profile());
    let platform =
        JavaPlatform::from_java_properties("Mac OS X", "amd64", "64", "10.15.7").unwrap();
    let patches = load(temp.path(), &platform).unwrap().unwrap();
    assert_eq!(
        patches.profile.java_version.as_ref().unwrap().major_version,
        25
    );
    assert!(
        !patches
            .profile
            .libraries
            .iter()
            .any(|lib| lib.name.ends_with(":forgePatches"))
    );
    let mut classpath = Vec::new();
    let (main, extra, _) = apply(
        patches,
        temp.path(),
        &temp.path().join("libraries"),
        &mut classpath,
    )
    .await
    .unwrap();
    assert_eq!(main, "RmclShim");
    assert_eq!(
        extra,
        ["com.gtnewhorizons.retrofuturabootstrap.MainStartOnFirstThread"]
    );
    assert_eq!(classpath[1], temp.path().join(".forge-patches.jar"));
    let mut shim = zip::ZipArchive::new(std::fs::File::open(&classpath[0]).unwrap()).unwrap();
    assert!(shim.by_name("RmclShim.class").is_ok());
}

#[test]
fn broken_or_missing_bundled_metadata_is_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let platform = JavaPlatform::from_java_properties("Linux", "amd64", "64", "6.0").unwrap();
    assert!(load(temp.path(), &platform).unwrap().is_none());
    let mut incomplete = bundled_profile();
    incomplete.java_version = None;
    write_mod(temp.path(), incomplete);
    assert!(
        load(temp.path(), &platform)
            .unwrap_err()
            .to_string()
            .contains("javaVersion")
    );
    let jar = write_mod(temp.path(), bundled_profile());
    std::fs::write(&jar, b"broken jar").unwrap();
    assert!(load(temp.path(), &platform).is_err());
    make_zip(
        jar.parent().unwrap(),
        "lwjgl3ify-3.0.35.jar",
        &[("other.txt", b"no metadata")],
    );
    assert!(
        load(temp.path(), &platform)
            .unwrap_err()
            .to_string()
            .contains("bundled client profile")
    );
}
