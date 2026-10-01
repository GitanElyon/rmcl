// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::io;
use std::path::Path;

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with_mode(path, bytes, false)
}

pub fn write_atomic_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with_mode(path, bytes, true)
}

fn write_atomic_with_mode(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    std::fs::create_dir_all(parent)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".rmcl-write-");
    #[cfg(unix)]
    if !private {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = builder.tempfile_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map(|_| ()).map_err(|error| error.error)
}

#[cfg(any(unix, windows))]
pub(crate) fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    copy_symlink_target(source, destination, &std::fs::read_link(source)?)
}

#[cfg(unix)]
pub(crate) fn copy_symlink_target(
    _source: &Path,
    destination: &Path,
    target: &Path,
) -> io::Result<()> {
    std::os::unix::fs::symlink(target, destination)
}

#[cfg(windows)]
pub(crate) fn copy_symlink_target(
    source: &Path,
    destination: &Path,
    target: &Path,
) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;
    if std::fs::symlink_metadata(source)?
        .file_type()
        .is_symlink_dir()
    {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
}
