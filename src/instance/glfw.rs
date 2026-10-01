// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub(crate) struct GlfwInstallation {
    pub path: PathBuf,
    pub version: Option<String>,
}

pub(crate) fn bundled_glfw_version(meta_dir: &Path, game_version: &str) -> Option<String> {
    let path = crate::storage::MetadataPaths::new(meta_dir)
        .versions()
        .join(game_version)
        .join("meta.json");
    let profile: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    profile
        .get("libraries")?
        .as_array()?
        .iter()
        .filter_map(|library| library.get("name")?.as_str())
        .find_map(|coordinate| {
            let mut parts = coordinate.split(':');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("org.lwjgl"), Some("lwjgl-glfw"), Some(version)) => Some(version.to_owned()),
                _ => None,
            }
        })
}

pub(crate) fn discover_glfw_installations() -> Vec<GlfwInstallation> {
    discover_in(&discovery_directories(), std::env::consts::OS)
}

fn discovery_directories() -> Vec<PathBuf> {
    let mut directories = Vec::<PathBuf>::new();
    let variables: &[&str] = match std::env::consts::OS {
        "windows" => &["PATH"],
        "macos" => &["DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH", "PATH"],
        _ => &["LD_LIBRARY_PATH", "PATH"],
    };
    for variable in variables {
        if let Some(value) = std::env::var_os(variable) {
            directories.extend(std::env::split_paths(&value));
        }
    }
    match std::env::consts::OS {
        "windows" => {
            if let Some(root) = std::env::var_os("SystemRoot") {
                let root = PathBuf::from(root);
                directories.extend([root.join("System32"), root.clone()]);
            }
        }
        "macos" => {
            directories.extend(
                [
                    "/usr/lib",
                    "/usr/local/lib",
                    "/opt/homebrew/lib",
                    "/opt/local/lib",
                ]
                .into_iter()
                .map(PathBuf::from),
            );
            if let Some(home) = dirs::home_dir() {
                directories.push(home.join("lib"));
            }
        }
        _ => {
            directories.extend(
                ["/usr/lib", "/usr/lib64", "/usr/local/lib", "/lib", "/lib64"]
                    .into_iter()
                    .map(PathBuf::from),
            );
            if let Some(home) = dirs::home_dir() {
                directories.push(home.join(".local/lib"));
            }
        }
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        directories.push(parent.to_owned());
    }
    if let Ok(directory) = std::env::current_dir() {
        directories.push(directory);
    }
    directories.sort();
    directories.dedup();
    directories
}

fn discover_in(roots: &[PathBuf], os: &str) -> Vec<GlfwInstallation> {
    let mut directories = roots.to_vec();
    let mut nested = Vec::new();
    for directory in &directories {
        if let Ok(entries) = std::fs::read_dir(directory) {
            nested.extend(
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir()),
            );
        }
    }
    directories.extend(nested);
    let mut seen = BTreeSet::new();
    let mut installations = Vec::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if !path.is_file()
                || !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| is_glfw_library_name(name, os))
            {
                continue;
            }
            let canonical = std::fs::canonicalize(&path).unwrap_or(path);
            if seen.insert(canonical.clone()) {
                installations.push(GlfwInstallation {
                    version: glfw_version_from_path(&canonical),
                    path: canonical,
                });
            }
        }
    }
    installations.sort_by(|left, right| left.path.cmp(&right.path));
    installations
}

fn is_glfw_library_name(name: &str, os: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let versioned_so = name.strip_prefix("libglfw.so.").is_some_and(|version| {
        version.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
    });
    match os {
        "windows" => name.starts_with("glfw") && name.ends_with(".dll"),
        "macos" => name.starts_with("libglfw.") && name.ends_with(".dylib"),
        _ => name == "libglfw.so" || versioned_so,
    }
}

pub(crate) fn glfw_version_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if let Some((_, suffix)) = name.rsplit_once(".so.") {
        return (!suffix.is_empty()).then(|| suffix.to_owned());
    }
    let stem = name
        .strip_prefix("libglfw.")
        .and_then(|name| name.strip_suffix(".dylib"))
        .or_else(|| {
            name.strip_prefix("glfw")
                .and_then(|name| name.strip_suffix(".dll"))
        })?;
    (!stem.is_empty()).then(|| stem.trim_start_matches(['-', '.']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_names_are_recognized() {
        assert!(is_glfw_library_name("libglfw.so.3", "linux"));
        assert!(is_glfw_library_name("libglfw.3.dylib", "macos"));
        assert!(is_glfw_library_name("glfw3.dll", "windows"));
        assert!(!is_glfw_library_name("libglfw.so.backup", "linux"));
        assert!(!is_glfw_library_name("libGL.so", "linux"));
        assert_eq!(
            glfw_version_from_path(Path::new("/usr/lib/libglfw.so.3.4")).as_deref(),
            Some("3.4")
        );
    }

    #[test]
    fn each_platform_discovers_its_native_libraries_in_environment_and_nested_directories() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("native");
        std::fs::create_dir(&nested).unwrap();
        for name in ["libglfw.so.3.4", "libglfw.3.dylib", "GLFW3.DLL"] {
            std::fs::write(temp.path().join(name), b"library").unwrap();
            std::fs::write(nested.join(name), b"library").unwrap();
        }
        for (os, expected, version) in [
            ("linux", "libglfw.so.3.4", "3.4"),
            ("macos", "libglfw.3.dylib", "3"),
            ("windows", "GLFW3.DLL", "3"),
        ] {
            let found = discover_in(&[temp.path().to_owned(), temp.path().to_owned()], os);
            assert_eq!(found.len(), 2, "{os}");
            assert!(
                found
                    .iter()
                    .all(|installation| installation.path.file_name().unwrap() == expected)
            );
            assert!(
                found
                    .iter()
                    .all(|installation| installation.version.as_deref() == Some(version))
            );
        }
    }
}
