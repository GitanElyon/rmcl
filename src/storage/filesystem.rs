// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_FILE_ID: AtomicU64 = AtomicU64::new(1);

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with_mode(path, bytes, false)
}

pub fn write_atomic_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with_mode(path, bytes, true)
}

fn write_atomic_with_mode(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    let temporary = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), id));
    let result = (|| {
        use std::io::Write;

        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
pub(crate) fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(std::fs::read_link(source)?, destination)
}

#[cfg(windows)]
pub(crate) fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    let target = std::fs::read_link(source)?;
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
}
