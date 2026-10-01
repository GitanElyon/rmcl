// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_PACK_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn read_pack_manifest(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_PACK_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_PACK_MANIFEST_BYTES {
        return Err("Pack manifest exceeds 8 MiB".to_owned());
    }
    Ok(bytes)
}

fn relative_path(
    entry: &zip::read::ZipFile<'_, std::fs::File>,
    root: &str,
) -> Result<Option<PathBuf>, String> {
    let root = root.trim_matches('/');
    if root.is_empty() || !entry.name().starts_with(&format!("{root}/")) {
        return Ok(None);
    }
    let enclosed = entry
        .enclosed_name()
        .ok_or_else(|| format!("Unsafe override path: {}", entry.name()))?;
    let relative = enclosed
        .strip_prefix(root)
        .map_err(|_| format!("Invalid override path: {}", entry.name()))?;
    if relative.as_os_str().is_empty() {
        return Ok(Some(relative.to_owned()));
    }
    let relative = relative
        .to_str()
        .ok_or_else(|| "Override path is not UTF-8".to_owned())?;
    Ok(Some(PathBuf::from(super::portable_pack_path(relative)?)))
}

pub(super) fn override_files(path: &Path, roots: &[&str]) -> Result<Vec<PathBuf>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if entry.is_dir() {
            continue;
        }
        for root in roots {
            if let Some(relative) = relative_path(&entry, root)?
                && !relative.as_os_str().is_empty()
            {
                files.push(relative);
                break;
            }
        }
    }
    Ok(files)
}

pub(super) fn extract_overrides(
    path: &Path,
    destination: &Path,
    roots: &[&str],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    // Later roots override earlier ones, independently of ZIP entry order.
    for root in roots {
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let Some(relative) = relative_path(&entry, root)? else {
                continue;
            };
            let target = destination.join(relative);
            if entry.is_dir() {
                std::fs::create_dir_all(&target)?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::File::create(&target)?;
            let size = std::io::copy(&mut entry, &mut file)?;
            tracing::trace!(
                "Extracted {} to {} ({} bytes)",
                entry.name(),
                target.display(),
                size
            );
        }
    }
    Ok(())
}
