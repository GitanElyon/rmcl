// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn sanitize_keeps_alphanumeric() {
    assert_eq!(sanitize("my-instance_123"), "my-instance_123");
}

#[test]
fn sanitize_replaces_special_chars() {
    assert_eq!(sanitize("my instance!"), "my_instance_");
    assert_eq!(sanitize("path/traversal"), "path_traversal");
}

#[test]
fn shortcut_paths_do_not_collide_after_escaping() {
    assert_ne!(shortcut_name("a b"), shortcut_name("a_b"));
    assert_ne!(shortcut_name("a?b"), shortcut_name("a b"));
    assert_ne!(shortcut_name("a%b"), shortcut_name("a%25b"));
    assert_eq!(shortcut_name("my-instance_123"), "my-instance_123");
}

#[test]
fn legacy_shortcut_is_attributed_to_the_right_instance() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rmcl-a_b.desktop");
    std::fs::write(&path, build_content("a b", None)).unwrap();
    assert!(owns_legacy_shortcut(&path, "a b"));
    assert!(!owns_legacy_shortcut(&path, "a_b"));
}

#[test]
#[cfg(target_os = "linux")]
fn build_content_linux() {
    let content = build_content("TestPack", None);
    assert!(content.contains("Name=Minecraft - TestPack"));
    assert!(content.contains("Exec=rmcl instance launch \"TestPack\""));
    assert!(content.contains("Terminal=false"));
    assert!(content.contains("Categories=Game;"));
}

#[test]
#[cfg(target_os = "linux")]
fn build_content_linux_with_icon() {
    let icon = PathBuf::from("/tmp/icon.png");
    let content = build_content("TestPack", Some(&icon));
    assert!(content.contains("Icon=/tmp/icon.png"));
}

#[test]
fn shortcut_arguments_escape_platform_metacharacters() {
    assert_eq!(
        quote_desktop_exec_arg("Pack \"$HOME`\\"),
        "\"Pack \\\"\\$HOME\\`\\\\\""
    );
    assert_eq!(quote_shell_arg("Pack 'quoted'"), "'Pack '\\''quoted'\\'''");
    assert_eq!(
        quote_windows_arg("Pack \\\"quoted"),
        "\"Pack \\\\\\\"quoted\""
    );
}
