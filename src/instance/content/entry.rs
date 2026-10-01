// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;

use super::icons::IconCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldGameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
    Hardcore,
}

impl WorldGameMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Survival => "Survival",
            Self::Creative => "Creative",
            Self::Adventure => "Adventure",
            Self::Spectator => "Spectator",
            Self::Hardcore => "Hardcore",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorldDetails {
    pub game_mode: Option<WorldGameMode>,
    pub last_played: Option<chrono::DateTime<chrono::Utc>>,
    pub minecraft_version: Option<String>,
    pub size: Option<String>,
    pub datapacks: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ContentEntry {
    pub file_stem: String,
    pub name: String,
    pub source_slug: Option<String>,
    pub installed_path: Option<PathBuf>,
    pub provider_project: Option<super::manifest::ProviderProject>,
    pub world_details: Option<WorldDetails>,
    pub title_suffix: Option<String>,
    pub footer_label: Option<String>,
    pub footer_change: Option<(String, String)>,
    pub description: String,
    pub enabled: bool,
    pub icon_bytes: Option<Vec<u8>>,
    pub provider_icon: bool,
    pub provider_description: bool,
    pub path: PathBuf,
    pub icon_lines: Option<Vec<Vec<IconCell>>>,
}

pub fn toggle_entry(entry: &ContentEntry) -> Result<(), std::io::Error> {
    toggle_entry_path(entry).map(drop)
}

pub(crate) fn toggle_entry_path(entry: &ContentEntry) -> Result<Option<PathBuf>, std::io::Error> {
    super::local::toggle(entry)
}
