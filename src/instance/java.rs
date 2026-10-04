// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaInstallation {
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaRuntime {
    pub major_version: u32,
    pub platform: crate::launch_profile::system::JavaPlatform,
}

#[must_use]
pub fn resolve_java_path(instance_java: Option<&str>) -> String {
    let settings = crate::config::SETTINGS.read().clone();
    let environment = merge_environment(&settings.defaults.environment, &BTreeMap::new());
    resolve_java_path_in(
        instance_java.or(settings.paths.effective_java_path()),
        &std::env::current_dir().unwrap_or_default(),
        &environment,
    )
}

/// Command uses the platform's environment-key comparison, including Windows Unicode casing.
pub fn merge_environment(
    global: &BTreeMap<String, String>,
    instance: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut command = std::process::Command::new("java");
    command.envs(global).envs(instance);
    command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_str()
                    .expect("environment names originate from strings")
                    .to_owned(),
                value
                    .expect("only environment additions were configured")
                    .to_str()
                    .expect("environment values originate from strings")
                    .to_owned(),
            )
        })
        .collect()
}

// These lookups are for the ASCII variable names PATH, JAVA_HOME and JDK_HOME.
fn environment_value(
    environment: &BTreeMap<String, String>,
    name: &str,
) -> Option<std::ffi::OsString> {
    environment
        .iter()
        .rev()
        .find(|(key, _)| {
            if cfg!(windows) {
                key.eq_ignore_ascii_case(name)
            } else {
                key.as_str() == name
            }
        })
        .map(|(_, value)| std::ffi::OsString::from(value))
        .or_else(|| std::env::var_os(name))
}

pub fn resolve_java_path_in(
    configured: Option<&str>,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> String {
    let search_path = environment_value(environment, "PATH");
    let find = |name: &str| {
        if Path::new(name).components().count() > 1 {
            return which::which_in(name, None::<&std::ffi::OsStr>, cwd).ok();
        }
        search_path.as_deref().and_then(|path| {
            std::env::split_paths(path)
                .filter(|directory| !cfg!(windows) || !directory.as_os_str().is_empty())
                .find_map(|directory| {
                    which::which_in(
                        cwd.join(directory).join(name),
                        None::<&std::ffi::OsStr>,
                        cwd,
                    )
                    .ok()
                })
        })
    };
    if let Some(configured) = configured.filter(|path| !path.is_empty()) {
        return find(configured)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| configured.to_owned());
    }
    if let Some(home) = environment_value(environment, "JAVA_HOME") {
        let bin = cwd
            .join(home)
            .join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" });
        if bin.is_file() {
            return bin.to_string_lossy().into_owned();
        }
    }
    find("java")
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "java".to_owned())
}

/// Probe the selected executable, rather than inferring its architecture from rmcl.
pub async fn probe_java(path: &Path) -> std::io::Result<JavaRuntime> {
    let (cwd, environment) = default_probe_context()?;
    probe_java_in(path, &cwd, &environment).await
}

pub async fn probe_java_in(
    path: &Path,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> std::io::Result<JavaRuntime> {
    let output = java_version_output_in(path, cwd, environment).await?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "Java settings probe exited with {}",
            output.status
        )));
    }
    parse_runtime_properties(&format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn default_probe_context() -> std::io::Result<(PathBuf, BTreeMap<String, String>)> {
    Ok((
        std::env::current_dir()?,
        merge_environment(
            &crate::config::SETTINGS.read().defaults.environment,
            &BTreeMap::new(),
        ),
    ))
}

fn parse_runtime_properties(output: &str) -> std::io::Result<JavaRuntime> {
    let property = |name: &str| -> std::io::Result<&str> {
        output
            .lines()
            .filter_map(|line| line.trim().split_once(" = "))
            .find_map(|(key, value)| (key == name).then_some(value.trim()))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Java probe did not report {name}"),
                )
            })
    };
    let major_version = parse_java_major_version(property("java.version")?).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid Java java.version")
    })?;
    Ok(JavaRuntime {
        major_version,
        platform: crate::launch_profile::system::JavaPlatform::from_java_properties(
            property("os.name")?,
            property("os.arch")?,
            property("sun.arch.data.model")?,
            property("os.version")?,
        )?,
    })
}

#[must_use]
pub fn detect_java_path() -> String {
    let Ok((cwd, environment)) = default_probe_context() else {
        return "java".to_owned();
    };
    resolve_java_path_in(None, &cwd, &environment)
}

/// Finds Java executables in the environment and the conventional installation
/// directories for the current platform. The search is deliberately bounded so
/// opening the selector never walks an entire drive.
#[must_use]
pub fn discover_installations() -> Vec<JavaInstallation> {
    let Ok((cwd, environment)) = default_probe_context() else {
        return Vec::new();
    };
    discover_installations_in(&cwd, &environment)
}

#[must_use]
pub fn discover_installations_in(
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Vec<JavaInstallation> {
    let mut candidates = Vec::new();
    for variable in ["JAVA_HOME", "JDK_HOME"] {
        if let Some(java_home) = environment_value(environment, variable) {
            add_java_home(&cwd.join(java_home), &mut candidates);
        }
    }
    let search_path = environment_value(environment, "PATH");
    if let Ok(path) = which::which_in(
        resolve_java_path_in(None, cwd, environment),
        None::<&std::ffi::OsStr>,
        cwd,
    ) {
        candidates.push(path);
    }
    if let Some(path) = search_path {
        for directory in std::env::split_paths(&path) {
            let executable = if cfg!(target_os = "windows") {
                "java.exe"
            } else {
                "java"
            };
            candidates.push(cwd.join(directory).join(executable));
        }
    }

    for root in java_roots() {
        collect_java_executables(&root, 3, &mut candidates);
    }

    let mut seen = HashSet::new();
    let mut installations = candidates
        .into_iter()
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let identity = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            seen.insert(identity)
                .then(|| inspect_installation_in(&path, cwd, environment))
                .flatten()
        })
        .collect::<Vec<_>>();
    installations.sort_by(|a, b| {
        java_major(b.version.as_deref())
            .cmp(&java_major(a.version.as_deref()))
            .then_with(|| a.path.cmp(&b.path))
    });
    installations
}

/// Reads the version reported by one Java executable.
#[must_use]
pub fn inspect_installation(path: &Path) -> Option<JavaInstallation> {
    let (cwd, environment) = default_probe_context().ok()?;
    inspect_installation_in(path, &cwd, &environment)
}

#[must_use]
pub fn inspect_installation_in(
    path: &Path,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Option<JavaInstallation> {
    let resolved = path
        .to_str()
        .map(|path| resolve_java_path_in(Some(path), cwd, environment));
    let path = resolved.as_deref().map_or(path, Path::new);
    path.is_file().then(|| JavaInstallation {
        version: java_version_in(path, cwd, environment),
        path: path.to_path_buf(),
    })
}

#[must_use]
pub fn load_installation_cache(path: &Path) -> Option<Vec<JavaInstallation>> {
    let (cwd, environment) = default_probe_context().ok()?;
    load_installation_cache_in(path, &cwd, &environment)
}

#[derive(Serialize, Deserialize)]
struct InstallationCache {
    context: String,
    installations: Vec<JavaInstallation>,
}

#[must_use]
pub fn probe_context_key(cwd: &Path, environment: &BTreeMap<String, String>) -> String {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    let mut add = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    };
    add(cwd.as_os_str().as_encoded_bytes());
    let mut ambient: Vec<_> = std::env::vars_os().collect();
    ambient.sort();
    for (key, value) in &ambient {
        add(key.as_encoded_bytes());
        add(value.as_encoded_bytes());
    }
    // Store a digest only; configured environment values may contain credentials.
    for (key, value) in merge_environment(environment, &BTreeMap::new()) {
        add(key.as_bytes());
        add(value.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[must_use]
pub fn load_installation_cache_in(
    path: &Path,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Option<Vec<JavaInstallation>> {
    let mut cache: InstallationCache = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if cache.context != probe_context_key(cwd, environment) {
        return None;
    }
    cache
        .installations
        .retain(|installation| installation.path.is_file());
    (!cache.installations.is_empty()).then_some(cache.installations)
}

pub fn save_installation_cache(
    path: &Path,
    installations: &[JavaInstallation],
) -> std::io::Result<()> {
    let (cwd, environment) = default_probe_context()?;
    save_installation_cache_in(path, installations, &cwd, &environment)
}

pub fn save_installation_cache_in(
    path: &Path,
    installations: &[JavaInstallation],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> std::io::Result<()> {
    let cache = InstallationCache {
        context: probe_context_key(cwd, environment),
        installations: installations.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&cache).map_err(std::io::Error::other)?;
    crate::storage::write_atomic(path, &bytes)
}

fn java_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(target_os = "windows") {
        for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(variable) {
                let root = PathBuf::from(root);
                roots.extend([
                    root.join("Java"),
                    root.join("Eclipse Adoptium"),
                    root.join("Programs/Eclipse Adoptium"),
                    root.join("Programs/Java"),
                    root.join("Microsoft"),
                    root.join("BellSoft"),
                    root.join("Zulu"),
                    root.join("Amazon Corretto"),
                ]);
            }
        }
    } else if cfg!(target_os = "macos") {
        roots.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        for prefix in ["/opt/homebrew/opt", "/usr/local/opt"] {
            for formula in [
                "openjdk",
                "openjdk@8",
                "openjdk@11",
                "openjdk@17",
                "openjdk@21",
            ] {
                roots.push(PathBuf::from(prefix).join(formula));
            }
        }
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("Library/Java/JavaVirtualMachines"));
            roots.push(home.join(".sdkman/candidates/java"));
            roots.push(home.join(".asdf/installs/java"));
            roots.push(home.join(".local/share/mise/installs/java"));
        }
    } else {
        roots.extend([
            PathBuf::from("/usr/lib/jvm"),
            PathBuf::from("/usr/java"),
            PathBuf::from("/opt/java"),
            PathBuf::from("/opt/jdk"),
        ]);
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".sdkman/candidates/java"));
            roots.push(home.join(".jdks"));
            roots.push(home.join(".asdf/installs/java"));
            roots.push(home.join(".local/share/mise/installs/java"));
        }
    }
    roots
}

fn collect_java_executables(directory: &Path, depth: usize, candidates: &mut Vec<PathBuf>) {
    if depth == 0 || !directory.is_dir() {
        return;
    }
    add_java_home(directory, candidates);
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_java_executables(&path, depth - 1, candidates);
            }
        }
    }
}

fn add_java_home(home: &Path, candidates: &mut Vec<PathBuf>) {
    let executable = if cfg!(target_os = "windows") {
        "java.exe"
    } else {
        "java"
    };
    for path in [
        home.join("bin").join(executable),
        home.join("Contents/Home/bin").join(executable),
    ] {
        if path.is_file() {
            candidates.push(path);
        }
    }
}

pub fn java_version_in(
    path: &Path,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Option<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let output = runtime
        .block_on(java_version_output_in(path, cwd, environment))
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    parse_java_version(&text)
}

async fn java_version_output_in(
    path: &Path,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> std::io::Result<std::process::Output> {
    let java = path
        .to_str()
        .map(|path| resolve_java_path_in(Some(path), cwd, environment));
    let mut command = tokio::process::Command::new(java.as_deref().map_or(path, Path::new));
    command
        .args(["-XshowSettings:properties", "-version"])
        .current_dir(cwd)
        .envs(environment)
        .kill_on_drop(true);
    tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "Java version check timed out")
        })?
}

fn parse_java_version(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim();
        if let Some(version) = line.strip_prefix("java.version = ") {
            return Some(version.trim().to_owned());
        }
        if !line.starts_with("java version ") && !line.starts_with("openjdk version ") {
            return None;
        }
        line.split_once('"')
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(version, _)| version.to_owned())
    })
}

pub(crate) fn java_major(version: Option<&str>) -> u32 {
    let Some(version) = version else {
        return 0;
    };
    let parts = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u32>().ok())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [1, legacy_major, ..] => *legacy_major,
        [major, ..] => *major,
        [] => 0,
    }
}

pub(crate) fn parse_java_major_version(output: &str) -> Option<u32> {
    if let Some(version) = parse_java_version(output) {
        return Some(java_major(Some(&version))).filter(|major| *major > 0);
    }

    let start = output.find(|character: char| character.is_ascii_digit())?;
    Some(java_major(Some(&output[start..]))).filter(|major| *major > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_and_legacy_java_versions() {
        assert_eq!(
            parse_java_version("openjdk version \"21.0.4\" 2024-07-16"),
            Some("21.0.4".to_owned())
        );
        assert_eq!(
            parse_java_version("java version \"1.8.0_412\""),
            Some("1.8.0_412".to_owned())
        );
        assert_eq!(java_major(Some("21.0.4")), 21);
        assert_eq!(java_major(Some("1.8.0_412")), 8);
        assert_eq!(java_major(Some("21-ea")), 21);
        assert_eq!(
            parse_java_major_version("openjdk version \"21-ea\" 2026-03-17"),
            Some(21)
        );
        assert_eq!(parse_java_major_version("openjdk 17"), Some(17));
    }

    #[test]
    fn installation_cache_round_trips_existing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("java");
        std::fs::write(&executable, b"java").unwrap();
        let cache = temp.path().join("cache/java/installations.json");
        let installations = vec![JavaInstallation {
            path: executable.clone(),
            version: Some("25.0.1".to_owned()),
        }];

        save_installation_cache(&cache, &installations).unwrap();
        assert_eq!(load_installation_cache(&cache), Some(installations));

        std::fs::remove_file(executable).unwrap();
        assert_eq!(load_installation_cache(&cache), None);
    }

    #[test]
    fn runtime_properties_report_selected_java_architecture_and_os_version() {
        for (os, arch, bits, name, normalized) in [
            ("Windows 10", "x86", "32", "windows", "x86"),
            ("Windows 11", "amd64", "64", "windows", "x86_64"),
            ("Mac OS X", "x86_64", "64", "osx", "x86_64"),
            ("Mac OS X", "aarch64", "64", "osx", "arm64"),
            ("Linux", "amd64", "64", "linux", "x86_64"),
        ] {
            let properties = format!(
                "Property settings:\r\n    java.version = 1.8.0_412\r\n    os.name = {os}\r\n    os.arch = {arch}\r\n    sun.arch.data.model = {bits}\r\n    os.version = 10.0\r\n"
            );
            let runtime = parse_runtime_properties(&properties).unwrap();
            assert_eq!(runtime.major_version, 8);
            assert_eq!(runtime.platform.os_name, name);
            assert_eq!(runtime.platform.arch, normalized);
            assert_eq!(runtime.platform.bitness.to_string(), bits);
            assert_eq!(runtime.platform.os_version, "10.0");
            assert!(
                parse_runtime_properties(&properties.replace("sun.arch.data.model =", "unknown ="))
                    .unwrap_err()
                    .to_string()
                    .contains("sun.arch.data.model")
            );
        }
    }

    #[test]
    fn incomplete_runtime_properties_do_not_fall_back_to_the_launcher_architecture() {
        assert!(parse_runtime_properties("openjdk version \"25.0.1\"").is_err());
        assert!(
            crate::launch_profile::system::JavaPlatform::from_java_properties(
                "Windows 10",
                "../x86",
                "32",
                "10.0"
            )
            .is_err()
        );
        assert!(
            crate::launch_profile::system::JavaPlatform::from_java_properties(
                "Mac OS X", "aarch64", "unknown", "15.0"
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn relative_java_search_paths_are_bound_to_the_execution_directory() {
        use std::os::unix::fs::PermissionsExt;
        let temp =
            tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap();
        let cwd = temp.path().join("minecraft");
        let java = cwd.join("bin/java");
        std::fs::create_dir_all(java.parent().unwrap()).unwrap();
        std::fs::write(&java, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
        let environment = BTreeMap::from([
            ("PATH".to_owned(), "bin".to_owned()),
            ("JAVA_HOME".to_owned(), "missing-home".to_owned()),
        ]);
        for configured in [None, Some("java"), Some("bin/java")] {
            assert_eq!(
                Path::new(&resolve_java_path_in(configured, &cwd, &environment)),
                java
            );
        }
    }

    #[tokio::test]
    async fn installed_jdk_reports_runtime_properties_on_the_native_platform() {
        let cwd = std::env::current_dir().unwrap();
        let environment = BTreeMap::new();
        let java = resolve_java_path_in(None, &cwd, &environment);
        let runtime = probe_java_in(Path::new(&java), &cwd, &environment)
            .await
            .unwrap();
        assert!(runtime.major_version >= 8);
        assert_eq!(
            runtime.platform.os_name,
            crate::launch_profile::system::mojang_os_name()
        );
        assert!(matches!(runtime.platform.bitness, 32 | 64));
        assert!(!runtime.platform.os_version.is_empty());
    }

    #[test]
    fn environment_overrides_follow_native_case_rules_and_are_deterministic() {
        let global = BTreeMap::from([
            ("PATH".into(), "global".into()),
            ("TÊST".into(), "global".into()),
        ]);
        let instance = BTreeMap::from([
            ("PATH".into(), "first".into()),
            ("path".into(), "last".into()),
            ("têst".into(), "instance".into()),
        ]);
        let merged = merge_environment(&global, &instance);
        assert_eq!(
            environment_value(&instance, "PATH").unwrap(),
            std::ffi::OsString::from(if cfg!(windows) { "last" } else { "first" })
        );
        if cfg!(windows) {
            assert_eq!(merged.len(), 2);
            assert_eq!(merged["PATH"], "last");
            assert_eq!(merged["TÊST"], "instance");
        } else {
            assert_eq!(merged.len(), 4);
            assert_eq!(merged["PATH"], "first");
            assert_eq!(merged["path"], "last");
            assert_eq!(merged["TÊST"], "global");
        }
    }

    #[test]
    fn installation_cache_is_bound_to_the_probe_context_without_storing_environment_values() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("java");
        std::fs::write(&executable, b"java").unwrap();
        let cache = temp.path().join("installations.json");
        let installations = vec![JavaInstallation {
            path: executable,
            version: Some("25.0.1".into()),
        }];
        let environment =
            BTreeMap::from([("PRIVATE_TEST_VALUE".into(), "sensitive-cache-value".into())]);
        save_installation_cache_in(&cache, &installations, temp.path(), &environment).unwrap();
        assert_eq!(
            load_installation_cache_in(&cache, temp.path(), &environment),
            Some(installations)
        );
        assert!(
            !std::fs::read_to_string(&cache)
                .unwrap()
                .contains("sensitive-cache-value")
        );
        let changed = BTreeMap::from([("PRIVATE_TEST_VALUE".into(), "another-value".into())]);
        assert!(load_installation_cache_in(&cache, temp.path(), &changed).is_none());
        assert!(
            load_installation_cache_in(&cache, &temp.path().join("other-cwd"), &environment)
                .is_none()
        );
    }
}
