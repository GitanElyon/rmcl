// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::cmp::Ordering;

/// Numeric version components, then prerelease identifiers. Releases outrank
/// prereleases of the same version; build metadata does not affect precedence.
pub(crate) fn compare_versions(a: &str, b: &str) -> Ordering {
    fn split(version: &str) -> (&str, Option<&str>) {
        let version = version.split('+').next().unwrap_or(version);
        version
            .split_once('-')
            .map_or((version, None), |(core, pre)| (core, Some(pre)))
    }
    let (a_core, a_pre) = split(a);
    let (b_core, b_pre) = split(b);
    compare_natural(a_core, b_core).then_with(|| match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => compare_natural(a, b),
    })
}

/// Older Fabric versions use build metadata as a sequential loader build ID.
pub(crate) fn compare_fabric_versions(a: &str, b: &str) -> Ordering {
    compare_versions(a, b).then_with(|| {
        compare_natural(
            a.split_once('+').map_or("", |(_, build)| build),
            b.split_once('+').map_or("", |(_, build)| build),
        )
    })
}

fn compare_natural(a: &str, b: &str) -> Ordering {
    let mut a = parts(a);
    let mut b = parts(b);
    loop {
        let ordering = match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (Some(a), Some(b))
                if a.as_bytes()[0].is_ascii_digit() && b.as_bytes()[0].is_ascii_digit() =>
            {
                let a = a.trim_start_matches('0');
                let b = b.trim_start_matches('0');
                a.len().cmp(&b.len()).then_with(|| a.cmp(b))
            }
            (a, b) => a.cmp(&b),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
}

fn parts(mut value: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        let numeric = value.as_bytes().first()?.is_ascii_digit();
        let end = value
            .find(|c: char| c.is_ascii_digit() != numeric)
            .unwrap_or(value.len());
        let (part, rest) = value.split_at(end);
        value = rest;
        Some(part)
    })
}

#[cfg(test)]
#[path = "tests/versions.rs"]
mod tests;
