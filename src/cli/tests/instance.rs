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

#[test]
fn jvm_argument_updates_preserve_quoted_paths_and_repeated_options() {
    let mut config: crate::instance::InstanceConfig = serde_json::from_value(serde_json::json!({
        "name": "test", "game_version": "1.21.1", "loader": "vanilla",
        "loader_version": null, "created": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    super::apply_config_update(&mut config, "jvm-args", r#""-Dpath=C:\Program Files\Java" --add-opens java.base/java.lang=ALL-UNNAMED --add-opens java.base/java.util=ALL-UNNAMED"#).unwrap();
    assert_eq!(
        config.jvm_args,
        [
            r"-Dpath=C:\Program Files\Java",
            "--add-opens",
            "java.base/java.lang=ALL-UNNAMED",
            "--add-opens",
            "java.base/java.util=ALL-UNNAMED"
        ]
    );
    let previous = config.jvm_args.clone();
    assert!(super::apply_config_update(&mut config, "jvm-args", "\"unterminated").is_err());
    assert_eq!(config.jvm_args, previous);
    super::apply_config_update(&mut config, "jvm-args", "").unwrap();
    assert!(config.jvm_args.is_empty());
}

#[test]
fn config_sync_profile_display_is_read_only_and_deleted_profiles_restore_local_defaults() {
    for target in ["default", "other"] {
        let tmp = tempfile::tempdir().unwrap();
        let manager = crate::instance::InstanceManager::new(
            tmp.path().join("instances"),
            tmp.path().join("meta"),
        );
        let instance = manager.instances_dir.join("inst");
        let minecraft = crate::storage::InstancePaths::new(&instance).minecraft();
        std::fs::create_dir_all(minecraft.join("config")).unwrap();
        std::fs::write(minecraft.join("options.txt"), b"local-default").unwrap();
        std::fs::write(minecraft.join("config/value"), b"local-default").unwrap();
        let mut config = serde_json::from_value(serde_json::json!({
            "name": "inst", "game_version": "1.21.1", "loader": "vanilla", "loader_version": null,
            "created": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        manager.save(&config).unwrap();
        crate::instance::config_sync::switch_profile_and_save(&manager, &mut config, Some("main"))
            .unwrap();
        std::fs::write(minecraft.join("options.txt"), b"profile-options").unwrap();
        crate::instance::config_sync::delete_profile(&manager.meta_dir, "main").unwrap();
        crate::instance::config_sync::create_profile(&manager.meta_dir, "other").unwrap();
        std::fs::write(
            crate::storage::MetadataPaths::new(&manager.meta_dir)
                .profiles()
                .join("other/options.txt"),
            b"other-options",
        )
        .unwrap();
        let original_config = std::fs::read(instance.join("instance.json")).unwrap();
        let matches = crate::cli::build_command()
            .try_get_matches_from(["rmcl", "instance", "profile", "inst"])
            .unwrap();
        let profile_matches = matches
            .subcommand_matches("instance")
            .unwrap()
            .subcommand_matches("profile")
            .unwrap();
        super::profile_instance(profile_matches, &manager).unwrap();
        assert_eq!(
            std::fs::read(instance.join("instance.json")).unwrap(),
            original_config
        );
        assert_eq!(
            std::fs::read(minecraft.join("options.txt")).unwrap(),
            b"profile-options"
        );

        let matches = crate::cli::build_command()
            .try_get_matches_from(["rmcl", "instance", "profile", "inst", target])
            .unwrap();
        let profile_matches = matches
            .subcommand_matches("instance")
            .unwrap()
            .subcommand_matches("profile")
            .unwrap();
        super::profile_instance(profile_matches, &manager).unwrap();
        if target == "other" {
            assert_eq!(
                manager
                    .load_one("inst")
                    .unwrap()
                    .config_sync_profile
                    .as_deref(),
                Some("other")
            );
            assert_eq!(
                std::fs::read(minecraft.join("options.txt")).unwrap(),
                b"other-options"
            );
            let matches = crate::cli::build_command()
                .try_get_matches_from(["rmcl", "instance", "profile", "inst", "default"])
                .unwrap();
            let profile_matches = matches
                .subcommand_matches("instance")
                .unwrap()
                .subcommand_matches("profile")
                .unwrap();
            super::profile_instance(profile_matches, &manager).unwrap();
        }
        assert_eq!(manager.load_one("inst").unwrap().config_sync_profile, None);
        assert_eq!(
            std::fs::read(minecraft.join("options.txt")).unwrap(),
            b"local-default"
        );
        assert_eq!(
            std::fs::read(minecraft.join("config/value")).unwrap(),
            b"local-default"
        );
        assert!(
            !crate::storage::MetadataPaths::new(&manager.meta_dir)
                .profiles()
                .join("main")
                .exists()
        );
    }
}
