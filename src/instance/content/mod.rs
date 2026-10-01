// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// minecraft uses a ".disabled" suffix convention for disabled content, so this leans on that heavily.

pub mod datapacks;
pub mod dependencies;
pub mod entry;
pub mod icons;
pub mod manifest;
pub mod mods;
mod packs;
pub mod provider;
pub mod reconcile;
pub mod resource_packs;
pub mod shaders;
pub mod updates;
pub mod worlds;

pub use datapacks::scan_one_datapack;
pub(crate) use icons::{
    IconCell, fallback_icon, fallback_icon_large, make_icon_pixels, make_icon_pixels_from_image,
    make_icon_quadrants_from_image,
};
pub use mods::{scan_mods, scan_one_mod};
pub use resource_packs::{scan_one_resource_pack, scan_resource_packs};
pub use shaders::{scan_one_shader, scan_shaders};
pub use worlds::{scan_one_world, scan_worlds};

use std::io::Read;

const MAX_LOCAL_METADATA_BYTES: u64 = 8 * 1024 * 1024;

fn read_local_metadata(reader: impl Read) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LOCAL_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_LOCAL_METADATA_BYTES).then_some(bytes)
}

pub(crate) fn parse_enabled_stem(file_name: &str, ext: &str) -> Option<(bool, String)> {
    let disabled_ext = format!("{ext}.disabled");
    if let Some(stem) = file_name.strip_suffix(&disabled_ext) {
        Some((false, stem.to_string()))
    } else {
        file_name
            .strip_suffix(ext)
            .map(|stem| (true, stem.to_string()))
    }
}

pub(crate) fn parse_enabled_stem_dir(file_name: &str) -> (bool, String) {
    if let Some(stem) = file_name.strip_suffix(".disabled") {
        (false, stem.to_string())
    } else {
        (true, file_name.to_string())
    }
}

pub(crate) fn read_icon_from_zip(archive: &mut zip::ZipArchive<std::fs::File>) -> Option<Vec<u8>> {
    read_local_metadata(archive.by_name("pack.png").ok()?)
}

pub(crate) fn open_zip(path: &std::path::Path) -> Option<zip::ZipArchive<std::fs::File>> {
    let file = std::fs::File::open(path).ok()?;
    zip::ZipArchive::new(file).ok()
}
