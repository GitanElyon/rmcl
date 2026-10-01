// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::parse_resolution;

#[test]
fn parses_valid_resolution() {
    assert_eq!(
        parse_resolution("1920x1080").expect("should parse"),
        (1920, 1080)
    );
}

#[test]
fn rejects_invalid_resolution_format() {
    assert!(parse_resolution("1920").is_err());
    assert!(parse_resolution("1920xa").is_err());
    assert!(parse_resolution("0x1080").is_err());
}

#[test]
fn invalid_memory_update_keeps_existing_setting() {
    let mut config: crate::instance::InstanceConfig = serde_json::from_value(serde_json::json!({
        "name": "test", "game_version": "1.21.1", "loader": "vanilla",
        "loader_version": null, "created": "2026-01-01T00:00:00Z", "memory_max": "8G"
    }))
    .unwrap();

    assert!(super::apply_config_update(&mut config, "memory-max", "garbage").is_err());
    assert_eq!(config.memory_max.as_deref(), Some("8G"));
}
