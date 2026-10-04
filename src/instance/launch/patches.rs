// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// lwjgl3ify ships a patched launchwrapper and retrofuturabootstrap that
// replace the old URLClassLoader-based launcher with one that works on
// modern java.

use std::path::{Path, PathBuf};

use super::LaunchError;
use crate::launch_profile::{model::LaunchProfile, system::JavaPlatform};

const LOG4J_FIXED_BASE: &str = "https://files.prismlauncher.org/maven/org/apache/logging/log4j";

#[derive(Debug)]
pub struct LwjglifyPatches {
    pub profile: LaunchProfile,
    patches_path: PathBuf,
}

pub fn load(
    minecraft_dir: &Path,
    platform: &JavaPlatform,
) -> Result<Option<LwjglifyPatches>, LaunchError> {
    use sha1::Digest;
    use std::io::Read;

    let mods_dir = minecraft_dir.join("mods");
    let Some(lwjgl3ify_jar) = find_lwjgl3ify_jar(&mods_dir)? else {
        return Ok(None);
    };
    let file = std::fs::File::open(&lwjgl3ify_jar)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        LaunchError::Parse(format!(
            "Invalid lwjgl3ify jar {}: {error}",
            lwjgl3ify_jar.display()
        ))
    })?;
    let entry = archive
        .by_name("me/eigenraven/lwjgl3ify/relauncher/version.json")
        .map_err(|error| {
            LaunchError::Parse(format!(
                "lwjgl3ify bundled client profile is missing in {}: {error}",
                lwjgl3ify_jar.display()
            ))
        })?;
    let mut profile: LaunchProfile = serde_json::from_reader(entry)?;
    if profile
        .main_class
        .as_deref()
        .is_none_or(|main| main.trim().is_empty())
        || profile.arguments.is_none()
        || profile
            .java_version
            .as_ref()
            .is_none_or(|version| version.major_version == 0)
    {
        return Err(LaunchError::Parse(
            "lwjgl3ify bundled client profile is missing mainClass, arguments or javaVersion"
                .into(),
        ));
    }
    for lib in &profile.libraries {
        if lib
            .downloads
            .as_ref()
            .is_none_or(|downloads| downloads.artifact.is_none() && lib.natives.is_none())
        {
            return Err(LaunchError::Parse(format!(
                "lwjgl3ify bundled library {} is missing download metadata",
                lib.name
            )));
        }
    }
    select_lwjgl_natives(&mut profile, platform)?;

    let patch_artifact = profile
        .libraries
        .iter()
        .find(|lib| lib.name.ends_with(":forgePatches"))
        .and_then(|lib| lib.downloads.as_ref())
        .and_then(|downloads| downloads.artifact.as_ref())
        .ok_or_else(|| {
            LaunchError::Parse(
                "lwjgl3ify bundled profile is missing the forgePatches artifact".into(),
            )
        })?;
    let mut payload = Vec::new();
    archive
        .by_name("me/eigenraven/lwjgl3ify/relauncher/forgePatches.zip")
        .map_err(|error| {
            LaunchError::Parse(format!(
                "lwjgl3ify forgePatches payload is missing: {error}"
            ))
        })?
        .read_to_end(&mut payload)?;
    if payload.len() as u64 != patch_artifact.size
        || !format!("{:x}", sha1::Sha1::digest(&payload)).eq_ignore_ascii_case(&patch_artifact.sha1)
    {
        return Err(LaunchError::Parse(
            "lwjgl3ify embedded forgePatches does not match its bundled profile".into(),
        ));
    }
    profile
        .libraries
        .retain(|lib| !lib.name.ends_with(":forgePatches"));

    // rfb only scans jars for plugin metadata. keeping the extracted
    // forgePatches payload as a zip makes it skip rfb-asm-safety and
    // rfb-modern-java, which GTNH needs on modern Java.
    let patches_path = minecraft_dir.join(".forge-patches.jar");
    crate::storage::write_atomic(&patches_path, &payload)?;
    Ok(Some(LwjglifyPatches {
        profile,
        patches_path,
    }))
}

pub async fn apply(
    patches: LwjglifyPatches,
    minecraft_dir: &Path,
    lib_dir: &Path,
    classpath: &mut Vec<PathBuf>,
) -> Result<(String, Vec<String>, Vec<String>), LaunchError> {
    classpath.insert(0, patches.patches_path);
    replace_log4j_fixed(lib_dir, classpath).await?;
    let mut jvm_args = Vec::new();

    // on java 24+, SecurityManager.getClassContext() was reimplemented to use
    // StackWalker. log4j 2.0-beta9's ThrowableProxy calls getClassContext() to
    // resolve stack frames, but StackWalker triggers class loading through
    // LaunchClassLoader, which debug-logs failures, which creates another
    // ThrowableProxy... infinite recursion. we break the loop by providing a
    // log4j config that sets the root level to INFO, so the debug() call in
    // LaunchClassLoader is a no-op and never creates a ThrowableProxy.
    write_log4j_config(minecraft_dir, &mut jvm_args)?;

    // RfbSystemClassLoader discovers plugins differently depending on whether
    // the main class is loaded by the JVM directly or through the system
    // classloader's loadClass(). direct invocation misses the rfb-asm-safety
    // and rfb-modern-java plugins from forgePatches, causing
    // ClassCircularityErrors. we use a tiny shim jar that loads the real main
    // class through ClassLoader.getSystemClassLoader().loadClass(), matching
    // how prism's EntryPoint does it.
    let shim_path = deploy_shim(minecraft_dir)?;
    classpath.insert(0, shim_path);

    Ok((
        "RmclShim".to_owned(),
        vec![
            patches
                .profile
                .main_class
                .expect("validated client main class"),
        ],
        jvm_args,
    ))
}

const SHIM_JAR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rmcl-shim.jar"));

fn deploy_shim(minecraft_dir: &Path) -> std::io::Result<PathBuf> {
    let dest = minecraft_dir.join(".rmcl-shim.jar");
    crate::storage::write_atomic(&dest, SHIM_JAR)?;
    Ok(dest)
}

fn find_lwjgl3ify_jar(mods_dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let entries = match std::fs::read_dir(mods_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut found = None;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("lwjgl3ify") && name.ends_with(".jar") {
            if found.is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple lwjgl3ify jars found in mods",
                ));
            }
            found = Some(entry.path());
        }
    }
    Ok(found)
}

// strips libraries from the classpath that are replaced by forge patches
// or not needed with lwjgl3ify. matches what prism's component system does:
// no old lwjgl2, no vanilla launchwrapper/asm, no extra vanilla-only libs.
pub fn strip_replaced_libs(profile: &mut LaunchProfile) {
    let replaced = [
        "launchwrapper-",
        "asm-all-",
        "lwjgl-2.",
        "lwjgl_util-",
        "commons-compress-",
        "commons-io-",
        "guava-15.",
    ];

    profile.libraries.retain(|lib| {
        if lib.name.starts_with("org.lwjgl.lwjgl:") {
            return false;
        }
        let name = lib.name.split(':').collect::<Vec<_>>();
        let filename = match name.as_slice() {
            [_, artifact, version, ..] => format!("{artifact}-{version}"),
            _ => return true,
        };
        let dominated = replaced.iter().any(|prefix| filename.starts_with(prefix));
        if dominated {
            tracing::info!("Stripping {} (replaced by forge-patches)", lib.name);
        }
        !dominated
    });
}

// The bundled profile names natives as separate Maven artifacts and uses
// OS-only rules for multiple CPU variants. Select exactly the selected JVM's variant.
fn select_lwjgl_natives(
    profile: &mut LaunchProfile,
    platform: &JavaPlatform,
) -> Result<(), LaunchError> {
    let classifier = match (platform.os_name, platform.arch.as_str(), platform.bitness) {
        ("osx", "arm64", 64) => "natives-macos-arm64",
        ("osx", "x86_64", 64) => "natives-macos",
        ("windows", "arm64", 64) => "natives-windows-arm64",
        ("windows", "x86", 32) => "natives-windows-x86",
        ("windows", "x86_64", 64) => "natives-windows",
        ("linux", "arm64", 64) => "natives-linux-arm64",
        ("linux", "arm", 32) => "natives-linux-arm32",
        ("linux", "ppc64le", 64) => "natives-linux-ppc64le",
        ("linux", "x86_64", 64) => "natives-linux",
        _ => {
            return Err(LaunchError::Parse(format!(
                "lwjgl3ify does not support Java {} {} ({}-bit)",
                platform.os_name, platform.arch, platform.bitness
            )));
        }
    };
    let native = |name: &str| -> Option<(String, String)> {
        let parts: Vec<_> = name.split(':').collect();
        match parts.as_slice() {
            ["org.lwjgl", artifact, _, variant] if variant.starts_with("natives-") => {
                Some(((*artifact).to_owned(), (*variant).to_owned()))
            }
            ["org.lwjgl", artifact, _] => artifact
                .split_once("-natives-")
                .map(|(module, variant)| (module.to_owned(), format!("natives-{variant}"))),
            _ => None,
        }
    };
    let mut native_modules = std::collections::HashSet::new();
    let mut selected = std::collections::HashSet::new();
    profile.libraries.retain(|lib| {
        let Some((module, variant)) = native(&lib.name) else {
            return true;
        };
        native_modules.insert(module.clone());
        if variant == classifier {
            selected.insert(module);
            true
        } else {
            false
        }
    });
    if let Some(missing) = native_modules.difference(&selected).next() {
        return Err(LaunchError::Parse(format!(
            "lwjgl3ify bundled profile has no {classifier} for {missing}"
        )));
    }
    Ok(())
}

// Suppresses LaunchClassLoader's debug() call to avoid the
// ThrowableProxy -> SecurityManager -> StackWalker recursion on Java 24+.
fn write_log4j_config(minecraft_dir: &Path, jvm_args: &mut Vec<String>) -> std::io::Result<()> {
    let config_path = minecraft_dir.join(".rmcl-log4j2.xml");
    let config = r#"<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
    <Appenders>
        <Console name="SysOut" target="SYSTEM_OUT">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level] [%logger]: %msg%n"/>
        </Console>
        <Queue name="ServerGuiConsole">
            <PatternLayout pattern="[%d{HH:mm:ss} %level]: %msg%n"/>
        </Queue>
        <RollingRandomAccessFile name="File" fileName="logs/latest.log"
                filePattern="logs/%d{yyyy-MM-dd}-%i.log.gz">
            <PatternLayout pattern="[%d{HH:mm:ss}] [%t/%level]: %msg%n"/>
            <Policies>
                <TimeBasedTriggeringPolicy/>
                <OnStartupTriggeringPolicy/>
            </Policies>
        </RollingRandomAccessFile>
    </Appenders>
    <Loggers>
        <Root level="info">
            <AppenderRef ref="SysOut"/>
            <AppenderRef ref="File"/>
        </Root>
    </Loggers>
</Configuration>
"#;

    crate::storage::write_atomic(&config_path, config.as_bytes())?;

    jvm_args.push(format!(
        "-Dlog4j.configurationFile={}",
        config_path.display()
    ));
    Ok(())
}

// replaces log4j-api and log4j-core 2.0-beta9 in the classpath with
// prism's patched "-fixed" builds. the vanilla 2.0-beta9 has a bug
// where ThrowableProxy calls SecurityManager.getClassContext() which
// on java 24+ uses StackWalker internally, triggering class loading
// through LaunchClassLoader, which triggers more logging, infinite
// recursion, stack overflow. the fixed builds patch this out.
async fn replace_log4j_fixed(lib_dir: &Path, classpath: &mut [PathBuf]) -> Result<(), LaunchError> {
    let replacements = [
        (
            "log4j-api-2.0-beta9.jar",
            "org/apache/logging/log4j/log4j-api/2.0-beta9-fixed/log4j-api-2.0-beta9-fixed.jar",
            format!("{LOG4J_FIXED_BASE}/log4j-api/2.0-beta9-fixed/log4j-api-2.0-beta9-fixed.jar"),
        ),
        (
            "log4j-core-2.0-beta9.jar",
            "org/apache/logging/log4j/log4j-core/2.0-beta9-fixed/log4j-core-2.0-beta9-fixed.jar",
            format!("{LOG4J_FIXED_BASE}/log4j-core/2.0-beta9-fixed/log4j-core-2.0-beta9-fixed.jar"),
        ),
    ];

    for (old_name, fixed_rel, url) in &replacements {
        if !classpath
            .iter()
            .any(|entry| entry.file_name().is_some_and(|name| name == *old_name))
        {
            continue;
        }
        let fixed_path = lib_dir.join(fixed_rel);

        if !fixed_path.exists() {
            tracing::info!("Downloading patched {old_name}...");
            let client = crate::net::HttpClient::new();
            crate::net::download_file(&client, url, &fixed_path, |_, _| {}).await?;
        }

        for entry in classpath.iter_mut() {
            if entry
                .file_name()
                .is_some_and(|n| n.to_string_lossy() == *old_name)
            {
                tracing::info!("Replacing {old_name} with patched version");
                *entry = fixed_path.clone();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/launch/patches.rs"]
mod tests;
