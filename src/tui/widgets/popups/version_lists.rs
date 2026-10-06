// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use crate::instance::{
    loader::{GameVersion, get_installer},
    models::ModLoader,
};

pub async fn game_versions(loader: ModLoader) -> Result<Vec<GameVersion>, String> {
    let client = crate::net::HttpClient::new();
    // Mojang, Fabric and Quilt supply chronological lists, including snapshots
    // whose IDs cannot be compared as numeric Minecraft versions.
    get_installer(loader)
        .get_game_versions(&client)
        .await
        .map_err(|error| error.to_string())
}

pub async fn loader_versions(loader: ModLoader, game_version: &str) -> Result<Vec<String>, String> {
    let client = crate::net::HttpClient::new();
    get_installer(loader)
        .get_versions(&client, game_version)
        .await
        .map_err(|error| error.to_string())
}
