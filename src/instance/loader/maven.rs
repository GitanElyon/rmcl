// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

#[must_use]
pub fn maven_coord_to_path(coord: &str) -> Option<String> {
    let parts: Vec<&str> = coord.split(':').collect();
    let path = match parts.as_slice() {
        [group, artifact, version] => {
            let group_path = group.replace('.', "/");
            format!(
                "{}/{}/{}/{}-{}.jar",
                group_path, artifact, version, artifact, version
            )
        }
        [group, artifact, version, classifier] => {
            let group_path = group.replace('.', "/");
            format!(
                "{}/{}/{}/{}-{}-{}.jar",
                group_path, artifact, version, artifact, version, classifier
            )
        }
        _ => return None,
    };
    crate::storage::safe_relative_path(std::path::Path::new(&path)).then_some(path)
}

#[cfg(test)]
#[path = "../tests/loader/maven.rs"]
mod tests;
