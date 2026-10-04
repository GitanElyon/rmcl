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
    std::fs::write(&path, build_content("a b", None, "/path with spaces/rmcl")).unwrap();
    assert!(owns_legacy_shortcut(&path, "a b"));
    assert!(!owns_legacy_shortcut(&path, "a_b"));
}

#[test]
#[cfg(target_os = "linux")]
fn build_content_linux() {
    let content = build_content("TestPack", None, "/path with spaces/rmcl");
    assert!(content.contains("Name=Minecraft - TestPack"));
    assert!(content.contains("Exec=\"/path with spaces/rmcl\" instance launch \"TestPack\""));
    assert!(content.contains("Terminal=false"));
    assert!(content.contains("Categories=Game;"));
}

#[test]
#[cfg(target_os = "linux")]
fn build_content_linux_with_icon() {
    let icon = PathBuf::from("/tmp/icon.png");
    let content = build_content("TestPack", Some(&icon), "/bin/rmcl");
    assert!(content.contains("Icon=/tmp/icon.png"));
}

#[test]
fn shortcut_arguments_escape_platform_metacharacters() {
    assert_eq!(
        quote_desktop_exec_arg("Pack \"$HOME`\\"),
        r#""Pack \\"\\$HOME\\`\\\\""#
    );
    assert_eq!(quote_desktop_exec_arg("Pack%f %Z"), r#""Pack%%f %%Z""#);
    assert_eq!(quote_shell_arg("Pack 'quoted'"), "'Pack '\\''quoted'\\'''");
    assert_eq!(
        quote_windows_arg("Pack \\\"quoted"),
        "\"Pack \\\\\\\"quoted\""
    );
}

#[test]
fn shortcuts_use_the_current_executable_and_literal_instance_arguments() {
    let executable = "/path with spaces/rmcl";
    let linux = build_linux_desktop("%USERNAME%", None, executable);
    assert!(linux.contains("Exec=\"/path with spaces/rmcl\" instance launch \"%%USERNAME%%\""));
    let macos = build_macos_command("%USERNAME%", executable);
    assert!(macos.contains("'/path with spaces/rmcl' instance launch '%USERNAME%'"));
    let windows = build_windows_shortcut("%USERNAME%", r"C:\path with spaces\rmcl.exe");
    assert!(windows.contains("shell.ShellExecute \"C:\\path with spaces\\rmcl.exe\", \"instance launch \"\"%USERNAME%\"\"\""));
    assert!(!windows.contains("shell.Run"));
}

#[test]
fn old_bare_executable_shortcuts_remain_owned() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("shortcut");
    #[cfg(target_os = "linux")]
    let content = "Exec=rmcl instance launch \"a b\"\n";
    #[cfg(target_os = "macos")]
    let content = "#!/bin/bash\nrmcl instance launch 'a b'\n";
    #[cfg(target_os = "windows")]
    let content = "shell.Run \"rmcl instance launch \"\"a b\"\"\", 0, False\r\n";
    std::fs::write(&path, content).unwrap();
    assert!(owns_legacy_shortcut(&path, "a b"));
    assert!(!owns_legacy_shortcut(&path, "a_b"));
}

#[test]
fn unicode_windows_scripts_round_trip_and_keep_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unicode.vbs");
    let content = format!(
        "{}\r\n{}",
        shortcut_marker("世界"),
        build_windows_shortcut("世界", r"C:\用户\rmcl.exe")
    );
    std::fs::write(&path, windows_script_bytes(&content)).unwrap();
    assert_eq!(read_shortcut(&path).unwrap(), content);
    assert!(owns_legacy_shortcut(&path, "世界"));
    assert!(!owns_legacy_shortcut(&path, "other"));
}

#[cfg(windows)]
#[test]
fn windows_shortcut_execution_preserves_percent_expressions() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("literal.txt");
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let script = format!(
        "[IO.File]::WriteAllText('{}', '%USERNAME% 世界')",
        output.display().to_string().replace('\'', "''")
    );
    let arguments = format!(
        "-NoProfile -NonInteractive -Command {}",
        quote_windows_arg(&script)
    );
    let shortcut = temp.path().join("literal.vbs");
    std::fs::write(
        &shortcut,
        windows_script_bytes(&windows_shell_execute(
            powershell.to_str().unwrap(),
            &arguments,
        )),
    )
    .unwrap();
    assert!(
        std::process::Command::new("cscript.exe")
            .arg("//Nologo")
            .arg(&shortcut)
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !output.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(std::fs::read_to_string(output).unwrap(), "%USERNAME% 世界");
}

#[test]
fn case_only_shortcut_rename_preserves_current_and_legacy_ownership_on_aliasing_volumes() {
    let temp = tempfile::tempdir().unwrap();
    let old_path = temp.path().join("CasePack.shortcut");
    let new_path = temp.path().join("casepack.shortcut");
    let current = build_content(
        "CasePack",
        None,
        std::env::current_exe().unwrap().to_str().unwrap(),
    );
    #[cfg(target_os = "linux")]
    let legacy = "Exec=rmcl instance launch \"CasePack\"\n";
    #[cfg(target_os = "macos")]
    let legacy = "#!/bin/bash\nrmcl instance launch 'CasePack'\n";
    #[cfg(target_os = "windows")]
    let legacy = "shell.Run \"rmcl instance launch \"\"CasePack\"\"\", 0, False\r\n";
    for content in [current.as_str(), legacy] {
        #[cfg(windows)]
        let original = windows_script_bytes(content);
        #[cfg(not(windows))]
        let original = content.as_bytes().to_vec();
        std::fs::write(&old_path, &original).unwrap();
        if !new_path.exists() {
            assert!(
                !rename_shortcut_alias(&old_path, &new_path, "CasePack", || panic!(
                    "Distinct paths are not aliases"
                ))
                .unwrap()
            );
            eprintln!("case-insensitive shortcut branch skipped: test volume is case-sensitive");
            return;
        }
        let failure = rename_shortcut_alias(&old_path, &new_path, "CasePack", || {
            Err(std::io::Error::other("test write failure"))
        });
        assert!(failure.is_err());
        assert_eq!(std::fs::read(&old_path).unwrap(), original);
        assert!(owns_legacy_shortcut(&old_path, "CasePack"));
        let updated = build_content(
            "casepack",
            None,
            std::env::current_exe().unwrap().to_str().unwrap(),
        );
        #[cfg(windows)]
        let bytes = windows_script_bytes(&updated);
        #[cfg(not(windows))]
        let bytes = updated.as_bytes().to_vec();
        assert!(
            rename_shortcut_alias(&old_path, &new_path, "CasePack", || {
                crate::storage::write_atomic(&new_path, &bytes)
            })
            .unwrap()
        );
        assert!(owns_legacy_shortcut(&new_path, "casepack"));
        assert!(!owns_legacy_shortcut(&old_path, "CasePack"));
        assert!(
            std::fs::read_dir(temp.path())
                .unwrap()
                .any(|entry| entry.unwrap().file_name() == "casepack.shortcut")
        );
        #[cfg(windows)]
        assert!(std::fs::read(&new_path).unwrap().starts_with(&[0xff, 0xfe]));
        std::fs::remove_file(&new_path).unwrap();
    }
}

#[test]
fn disabled_or_foreign_shortcuts_are_not_enabled_by_alias_handling() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("CasePack.shortcut");
    let new = temp.path().join("casepack.shortcut");
    assert!(
        !rename_shortcut_alias(&old, &new, "CasePack", || panic!(
            "Disabled shortcut must stay disabled"
        ))
        .unwrap()
    );
    assert!(!old.exists() && !new.exists());
    std::fs::write(&old, build_content("other", None, "/launcher")).unwrap();
    assert!(
        !rename_shortcut_alias(&old, &new, "CasePack", || panic!(
            "Foreign shortcut must stay untouched"
        ))
        .unwrap()
    );
    assert!(owns_legacy_shortcut(&old, "other"));
}

#[test]
fn distinct_hard_link_shortcuts_are_not_case_aliases() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("source.shortcut");
    let new = temp.path().join("destination.shortcut");
    std::fs::write(&old, build_content("Pack", None, "/launcher")).unwrap();
    if std::fs::hard_link(&old, &new).is_err() {
        eprintln!("hard-link collision check skipped: test volume does not support hard links");
        return;
    }
    assert!(
        !rename_shortcut_alias(&old, &new, "Pack", || panic!(
            "Distinct hard links are not aliases"
        ))
        .unwrap()
    );
    assert!(owns_legacy_shortcut(&old, "Pack") && owns_legacy_shortcut(&new, "Pack"));
}
