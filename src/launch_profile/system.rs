// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::{Path, PathBuf};

// Native libraries and rules describe the Java process, which can have a
// different architecture from the launcher (for example, Intel Java on macOS ARM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaPlatform {
    pub os_name: &'static str,
    pub arch: String,
    pub bitness: u32,
    pub os_version: String,
}

impl JavaPlatform {
    pub fn from_java_properties(
        os_name: &str,
        arch: &str,
        bitness: &str,
        os_version: &str,
    ) -> std::io::Result<Self> {
        let invalid = |message| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
        let os_name = match os_name {
            name if name.starts_with("Windows") => "windows",
            "Mac OS X" | "macOS" | "Darwin" => "osx",
            "Linux" => "linux",
            other => return Err(invalid(format!("Unsupported Java os.name: {other}"))),
        };
        let arch = match arch {
            "amd64" | "x86_64" | "x64" => "x86_64",
            "x86" | "i386" | "i486" | "i586" | "i686" => "x86",
            "aarch64" | "arm64" => "arm64",
            other => other,
        };
        if arch.is_empty()
            || !arch
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(invalid(format!("Invalid Java os.arch: {arch}")));
        }
        let bitness = match bitness {
            "32" => 32,
            "64" => 64,
            other => {
                return Err(invalid(format!(
                    "Invalid Java sun.arch.data.model: {other}"
                )));
            }
        };
        if os_version.is_empty() {
            return Err(invalid("Missing Java os.version".to_owned()));
        }
        Ok(Self {
            os_name,
            arch: arch.to_owned(),
            bitness,
            os_version: os_version.to_owned(),
        })
    }

    pub fn natives_directory(&self, meta_dir: &Path, version: &str) -> PathBuf {
        crate::storage::MetadataPaths::new(meta_dir)
            .versions()
            .join(version)
            .join("natives")
            .join(format!("{}-{}-{}", self.os_name, self.arch, self.bitness))
    }
}

pub fn mojang_os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "osx",
        other => other,
    }
}
