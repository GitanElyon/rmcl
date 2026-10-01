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
    let content = build_content(name, None);
    let command = content.lines().find(|line| {
        line.starts_with("Exec=")
            || line.starts_with("shell.Run ")
            || line.starts_with("rmcl instance launch ")
    });
    command.is_some_and(|command| {
        std::fs::read_to_string(path)
            .is_ok_and(|existing| existing.lines().any(|line| line == command))
    })
}

pub fn exists(name: &str) -> bool {
    desktop_path(name).is_some_and(|path| owns_legacy_shortcut(&path, name))
        || legacy_path(name).is_some_and(|path| owns_legacy_shortcut(&path, name))
}

pub fn create(config: &InstanceConfig) -> std::io::Result<PathBuf> {
    let path = desktop_path(&config.name)
        .ok_or_else(|| std::io::Error::other("cannot resolve shortcut directory"))?;

    if path.exists() && !owns_legacy_shortcut(&path, &config.name) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("Another shortcut already exists at '{}'", path.display()),
        ));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let icon = ensure_icon();
    let content = build_content(&config.name, icon.as_deref());
    if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
        crate::storage::write_atomic(&path, content.as_bytes())?;
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
    create(new_config)?;
    if !same_path {
        remove(old_name)?;
    }
    Ok(())
}

fn build_content(name: &str, icon: Option<&Path>) -> String {
    #[cfg(target_os = "linux")]
    {
        build_linux_desktop(name, icon)
    }

    #[cfg(target_os = "windows")]
    {
        let _ = icon;
        build_windows_shortcut(name)
    }

    #[cfg(target_os = "macos")]
    {
        let _ = icon;
        build_macos_command(name)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = (name, icon);
        String::new()
    }
}

#[cfg(target_os = "linux")]
fn build_linux_desktop(name: &str, icon: Option<&Path>) -> String {
    let mut out = String::new();
    out.push_str("[Desktop Entry]\n");
    out.push_str("Version=0.3.1\n");
    out.push_str("Type=Application\n");
    out.push_str(&format!("Name=Minecraft - {name}\n"));
    out.push_str(&format!("Comment=Launch {name} Minecraft instance\n"));
    out.push_str(&format!(
        "Exec=rmcl instance launch {}\n",
        quote_desktop_exec_arg(name)
    ));
    if let Some(icon) = icon {
        out.push_str(&format!("Icon={}\n", icon.display()));
    }
    out.push_str("Terminal=false\n");
    out.push_str("Categories=Game;\n");
    out
}

#[cfg(target_os = "windows")]
fn build_windows_shortcut(name: &str) -> String {
    let command = format!("rmcl instance launch {}", quote_windows_arg(name));
    let escaped_command = command.replace('"', "\"\"");

    let mut out = String::new();
    out.push_str("Set shell = CreateObject(\"WScript.Shell\")\r\n");
    out.push_str(&format!("shell.Run \"{escaped_command}\", 0, False\r\n"));
    out
}

#[cfg(target_os = "macos")]
fn build_macos_command(name: &str) -> String {
    let mut out = String::new();
    out.push_str("#!/bin/bash\n");
    out.push_str(&format!("# Launch Minecraft instance: {name}\n"));
    out.push_str(&format!("rmcl instance launch {}\n", quote_shell_arg(name)));
    out
}

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
    escaped
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
