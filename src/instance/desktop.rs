// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::{Path, PathBuf};

use crate::instance::models::InstanceConfig;

const ICON_BYTES: &[u8] = include_bytes!("../../assets/icon.svg");

pub fn desktop_path(name: &str) -> Option<PathBuf> {
    let sanitized = shortcut_name(name);

    #[cfg(target_os = "linux")]
    {
        dirs::data_dir().map(|d| {
            d.join("applications")
                .join(format!("rmcl-{sanitized}.desktop"))
        })
    }

    #[cfg(target_os = "windows")]
    {
        dirs::desktop_dir().map(|d| d.join(format!("Minecraft - {sanitized}.vbs")))
    }

    #[cfg(target_os = "macos")]
    {
        dirs::desktop_dir().map(|d| d.join(format!("Minecraft - {sanitized}.command")))
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = sanitized;
        None
    }
}

pub fn icon_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("rmcl").join("icon.svg"))
}

fn ensure_icon() -> Option<PathBuf> {
    let path = icon_path()?;
    if std::fs::read(&path).ok().as_deref() == Some(ICON_BYTES) {
        return Some(path);
    }
    let parent = path.parent()?;
    if let Err(e) = std::fs::create_dir_all(parent) {
        tracing::warn!("Failed to create icon directory: {}", e);
        return None;
    }
    if let Err(e) = crate::storage::write_atomic(&path, ICON_BYTES) {
        tracing::warn!("Failed to write bundled icon: {}", e);
        return None;
    }
    Some(path)
}

fn shortcut_name(name: &str) -> String {
    let mut encoded = String::new();
    for character in name.chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_') {
            encoded.push(character);
        } else {
            for byte in character.to_string().bytes() {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    encoded
}

fn legacy_path(name: &str) -> Option<PathBuf> {
    let legacy = desktop_path(&sanitize(name))?;
    (Some(&legacy) != desktop_path(name).as_ref()).then_some(legacy)
}

fn owns_legacy_shortcut(path: &Path, name: &str) -> bool {
    let Ok(existing) = read_shortcut(path) else {
        return false;
    };
    let marker = shortcut_marker(name);
    if existing.lines().any(|line| line == marker) {
        return true;
    }
    #[cfg(target_os = "linux")]
    let commands = {
        let quoted = quote_desktop_exec_arg(name);
        [
            format!("Exec=rmcl instance launch {quoted}"),
            format!(
                "Exec=rmcl instance launch {}",
                quoted.replace("\\\\", "\\").replace("%%", "%")
            ),
        ]
    };
    #[cfg(target_os = "windows")]
    let commands = {
        let command =
            format!("rmcl instance launch {}", quote_windows_arg(name)).replace('"', "\"\"");
        [format!("shell.Run \"{command}\", 0, False")]
    };
    #[cfg(target_os = "macos")]
    let commands = [format!("rmcl instance launch {}", quote_shell_arg(name))];
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    let commands: [String; 0] = [];
    existing
        .lines()
        .any(|line| commands.iter().any(|command| line == command))
}

fn read_shortcut(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    if let Some(bytes) = bytes.strip_prefix(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid UTF-16 shortcut",
            ));
        }
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    } else {
        String::from_utf8(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }
}

#[cfg(any(windows, test))]
fn windows_script_bytes(content: &str) -> Vec<u8> {
    // Windows Script Host reads Unicode scripts as BOM-prefixed UTF-16, not UTF-8.
    [0xfeff]
        .into_iter()
        .chain(content.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn shortcut_marker(name: &str) -> String {
    let prefix = if cfg!(windows) { "'" } else { "#" };
    format!("{prefix} rmcl-instance: {}", shortcut_name(name))
}
pub fn exists(name: &str) -> bool {
    desktop_path(name).is_some_and(|path| owns_legacy_shortcut(&path, name))
        || legacy_path(name).is_some_and(|path| owns_legacy_shortcut(&path, name))
}

pub fn create(config: &InstanceConfig) -> std::io::Result<PathBuf> {
    crate::instance::manager::validate_name(&config.name).map_err(std::io::Error::other)?;
    let path = desktop_path(&config.name)
        .ok_or_else(|| std::io::Error::other("cannot resolve shortcut directory"))?;

    match std::fs::symlink_metadata(&path) {
        Ok(_) if !owns_legacy_shortcut(&path, &config.name) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("Another shortcut already exists at '{}'", path.display()),
            ));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let icon = ensure_icon();
    let executable = std::env::current_exe()?;
    let executable = executable.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Launcher executable path is not UTF-8",
        )
    })?;
    let content = build_content(&config.name, icon.as_deref(), executable);
    if read_shortcut(&path).ok().as_deref() != Some(content.as_str()) {
        #[cfg(windows)]
        let encoded = windows_script_bytes(&content);
        #[cfg(windows)]
        let bytes = encoded.as_slice();
        #[cfg(not(windows))]
        let bytes = content.as_bytes();
        crate::storage::write_atomic(&path, bytes)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&path, perms)?;
    }

    if let Some(legacy) = legacy_path(&config.name)
        && owns_legacy_shortcut(&legacy, &config.name)
    {
        std::fs::remove_file(legacy)?;
    }

    Ok(path)
}

pub fn remove(name: &str) -> std::io::Result<()> {
    let Some(path) = desktop_path(name) else {
        return Ok(());
    };
    if owns_legacy_shortcut(&path, name) {
        std::fs::remove_file(path)?;
    }
    if let Some(legacy) = legacy_path(name)
        && owns_legacy_shortcut(&legacy, name)
    {
        std::fs::remove_file(legacy)?;
    }
    Ok(())
}

pub fn set_enabled(config: &InstanceConfig, enabled: bool) -> std::io::Result<()> {
    if enabled {
        create(config).map(|_| ())
    } else {
        remove(&config.name)
    }
}

pub fn toggle(config: &InstanceConfig) -> std::io::Result<bool> {
    let enabled = !exists(&config.name);
    set_enabled(config, enabled)?;
    Ok(enabled)
}

pub fn rename(old_name: &str, new_config: &InstanceConfig) -> std::io::Result<()> {
    if !exists(old_name) {
        return Ok(());
    }
    let same_path = desktop_path(old_name) == desktop_path(&new_config.name);
    if !same_path && let Some(new_path) = desktop_path(&new_config.name) {
        for old_path in [desktop_path(old_name), legacy_path(old_name)]
            .into_iter()
            .flatten()
        {
            if rename_shortcut_alias(&old_path, &new_path, old_name, || {
                create(new_config).map(|_| ())
            })? {
                remove(old_name)?;
                return Ok(());
            }
        }
    }
    create(new_config)?;
    if !same_path {
        remove(old_name)?;
    }
    Ok(())
}

fn rename_shortcut_alias(
    old_path: &Path,
    new_path: &Path,
    old_name: &str,
    create: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<bool> {
    if old_path == new_path
        || !owns_legacy_shortcut(old_path, old_name)
        || !crate::instance::manager::paths_alias(old_path, new_path)?
    {
        return Ok(false);
    }
    let temporary = tempfile::Builder::new()
        .prefix(".rmcl-shortcut-")
        .tempdir_in(
            old_path
                .parent()
                .ok_or_else(|| std::io::Error::other("Shortcut has no parent"))?,
        )?;
    let original = temporary.path().join("original");
    std::fs::rename(old_path, &original)?;
    if let Err(error) = create() {
        if let Err(rollback) = std::fs::rename(&original, old_path) {
            let retained = temporary.keep();
            return Err(std::io::Error::other(format!(
                "Could not update shortcut: {error}; rollback failed: {rollback}; original retained at '{}'",
                retained.join("original").display(),
            )));
        }
        return Err(error);
    }
    Ok(true)
}

fn build_content(name: &str, icon: Option<&Path>, executable: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        build_linux_desktop(name, icon, executable)
    }

    #[cfg(target_os = "windows")]
    {
        let _ = icon;
        build_windows_shortcut(name, executable)
    }

    #[cfg(target_os = "macos")]
    {
        let _ = icon;
        build_macos_command(name, executable)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = (name, icon, executable);
        String::new()
    }
}

#[cfg(any(target_os = "linux", test))]
fn build_linux_desktop(name: &str, icon: Option<&Path>, executable: &str) -> String {
    let mut out = String::new();
    out.push_str("[Desktop Entry]\n");
    out.push_str("Version=1.0\n");
    out.push_str("Type=Application\n");
    out.push_str(&format!("Name=Minecraft - {name}\n"));
    out.push_str(&format!("Comment=Launch {name} Minecraft instance\n"));
    out.push_str(&format!("# rmcl-instance: {}\n", shortcut_name(name)));
    out.push_str(&format!(
        "Exec={} instance launch {}\n",
        quote_desktop_exec_arg(executable),
        quote_desktop_exec_arg(name)
    ));
    if let Some(icon) = icon {
        out.push_str(&format!("Icon={}\n", icon.display()));
    }
    out.push_str("Terminal=false\n");
    out.push_str("Categories=Game;\n");
    out
}

#[cfg(any(target_os = "windows", test))]
fn build_windows_shortcut(name: &str, executable: &str) -> String {
    let arguments = format!("instance launch {}", quote_windows_arg(name));
    let mut out = format!("' rmcl-instance: {}\r\n", shortcut_name(name));
    out.push_str(&windows_shell_execute(executable, &arguments));
    out
}

#[cfg(any(target_os = "windows", test))]
fn windows_shell_execute(executable: &str, arguments: &str) -> String {
    // ShellExecute passes parameters literally; WScript.Shell.Run expands %VAR% in them.
    format!(
        "Set shell = CreateObject(\"Shell.Application\")\r\nshell.ShellExecute \"{}\", \"{}\", \"\", \"open\", 0\r\n",
        executable.replace('"', "\"\""),
        arguments.replace('"', "\"\"")
    )
}

#[cfg(any(target_os = "macos", test))]
fn build_macos_command(name: &str, executable: &str) -> String {
    let mut out = String::new();
    out.push_str("#!/bin/bash\n");
    out.push_str(&format!("# rmcl-instance: {}\n", shortcut_name(name)));
    out.push_str(&format!(
        "{} instance launch {}\n",
        quote_shell_arg(executable),
        quote_shell_arg(name)
    ));
    out
}

#[cfg(any(target_os = "linux", test))]
fn quote_desktop_exec_arg(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        if matches!(character, '"' | '`' | '$' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('"');
    escaped.replace('\\', "\\\\").replace('%', "%%")
}

#[cfg(any(target_os = "macos", test))]
fn quote_shell_arg(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(any(target_os = "windows", test))]
fn quote_windows_arg(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
        } else if character == '"' {
            quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
        } else {
            quoted.extend(std::iter::repeat_n('\\', backslashes));
            quoted.push(character);
            backslashes = 0;
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

pub(crate) fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/desktop.rs"]
mod tests;
