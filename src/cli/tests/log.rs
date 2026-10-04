// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

#[test]
fn following_log_bytes_keeps_partial_lines_and_handles_truncation() {
    use super::read_new_log_bytes;
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("latest.log");
    std::fs::write(&path, b"abc").unwrap();
    let mut offset = 0;
    assert_eq!(read_new_log_bytes(&path, &mut offset).unwrap(), b"abc");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"def\nnext\n").unwrap();
    assert_eq!(
        read_new_log_bytes(&path, &mut offset).unwrap(),
        b"def\nnext\n"
    );
    assert!(read_new_log_bytes(&path, &mut offset).unwrap().is_empty());
    std::fs::write(&path, b"new\n").unwrap();
    assert_eq!(read_new_log_bytes(&path, &mut offset).unwrap(), b"new\n");
}

use super::resolve_log_path;

#[test]
fn resolves_latest_log_when_no_file_is_given() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo/minecraft/logs/launches");
    std::fs::create_dir_all(&dir).expect("log directory should exist");
    std::fs::write(dir.join("2024-01-02_03-04-05.log"), "newer").expect("write newer log");
    std::fs::write(dir.join("2024-01-01_03-04-05.log"), "older").expect("write older log");

    let path = resolve_log_path(tmp.path(), "demo", None).expect("latest log should resolve");
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("2024-01-02_03-04-05.log")
    );
}

#[test]
fn resolves_named_log_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo/minecraft/logs/launches");
    std::fs::create_dir_all(&dir).expect("log directory should exist");
    std::fs::write(dir.join("latest.log"), "hello").expect("write named log");

    let path =
        resolve_log_path(tmp.path(), "demo", Some("latest.log")).expect("named log should resolve");
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("latest.log")
    );
}

#[test]
fn rejects_log_paths_outside_the_instance() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("demo/minecraft/logs/launches");
    std::fs::create_dir_all(&dir).unwrap();
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, "private").unwrap();

    assert!(resolve_log_path(tmp.path(), "demo", Some(outside.to_str().unwrap())).is_err());
    assert!(resolve_log_path(tmp.path(), "demo", Some("../../outside.txt")).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, dir.join("escape.log")).unwrap();
        assert!(resolve_log_path(tmp.path(), "demo", Some("escape.log")).is_err());
        assert!(resolve_log_path(tmp.path(), "demo", None).is_err());
    }
}
