// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::find_entry_by_stem;
use crate::instance::content::entry::ContentEntry;
use std::path::PathBuf;

fn entry(file_stem: &str) -> ContentEntry {
    ContentEntry {
        file_stem: file_stem.to_string(),
        name: file_stem.to_string(),
        source_slug: None,
        installed_path: None,
        provider_project: None,
        world_details: None,
        title_suffix: None,
        footer_label: None,
        footer_change: None,
        description: String::new(),
        enabled: true,
        icon_bytes: None,
        provider_icon: false,
        provider_description: false,
        path: PathBuf::from(file_stem),
        icon_lines: None,
    }
}

#[test]
fn matches_by_stem_case_insensitively() {
    let entries = vec![entry("Sodium"), entry("Lithium")];
    let found = find_entry_by_stem(&entries, "sOdIuM")
        .unwrap()
        .expect("entry should match");
    assert_eq!(found.file_stem, "Sodium");
}

#[test]
fn returns_none_for_missing_stem() {
    let entries = vec![entry("Sodium")];
    assert!(find_entry_by_stem(&entries, "iris").unwrap().is_none());
}

#[test]
fn colliding_enabled_and_disabled_files_require_an_exact_filename_and_cannot_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let mut enabled = entry("same");
    enabled.path = temp.path().join("same.jar");
    let mut disabled = enabled.clone();
    disabled.enabled = false;
    disabled.path = temp.path().join("same.jar.disabled");
    std::fs::write(&enabled.path, b"enabled").unwrap();
    std::fs::write(&disabled.path, b"disabled").unwrap();
    let entries = [enabled, disabled];
    assert!(find_entry_by_stem(&entries, "same").is_err());
    assert!(
        !find_entry_by_stem(&entries, "same.jar.disabled")
            .unwrap()
            .unwrap()
            .enabled
    );
    for entry in &entries {
        assert!(crate::instance::content::entry::toggle_entry(entry).is_err());
    }
    assert_eq!(std::fs::read(&entries[0].path).unwrap(), b"enabled");
    assert_eq!(std::fs::read(&entries[1].path).unwrap(), b"disabled");
}
