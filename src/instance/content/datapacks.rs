// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::Path;

use super::entry::ContentEntry;

pub fn scan_one_datapack(path: &Path, file_stem: &str, enabled: bool) -> ContentEntry {
    super::packs::scan_one_pack(path, file_stem, enabled)
}
