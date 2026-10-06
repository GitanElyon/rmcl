// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[rstest::rstest]
#[case("21.1.251", "21.1.9")]
#[case("21.10.64", "21.10.63")]
#[case("47.10.0", "47.9.0")]
#[case("0.19.5", "0.19.4")]
#[case("0.30.0", "0.30.0-beta.10")]
#[case("0.30.0-beta.10", "0.30.0-beta.9")]
#[case("0.30.0-rc.1", "0.30.0-beta.10")]
#[case("0.30.0-beta.1", "0.29.3")]
#[case("0.1.0.50", "0.1.0.9")]
#[case("26.1.2.100", "26.1.2.99")]
#[case("1.21.11", "1.21.9")]
#[case("1.10", "1.9.4")]
#[case("999999999999999999999999", "99999999999999999999999")]
fn numeric_and_prerelease_versions_compare_by_precedence(#[case] newer: &str, #[case] older: &str) {
    assert_eq!(compare_versions(newer, older), Ordering::Greater);
    assert_eq!(compare_versions(older, newer), Ordering::Less);
    assert_eq!(compare_versions(newer, newer), Ordering::Equal);
}

#[test]
fn build_metadata_does_not_affect_precedence() {
    assert_eq!(
        compare_versions("0.30.0+build.2", "0.30.0+build.1"),
        Ordering::Equal
    );
    assert_eq!(
        compare_versions("0.30.0-beta.1+build.2", "0.30.0-beta.1"),
        Ordering::Equal
    );
}

#[test]
fn fabric_build_numbers_break_equal_version_ties_numerically() {
    assert_eq!(
        compare_fabric_versions("0.7.8+build.10", "0.7.8+build.9"),
        Ordering::Greater
    );
    assert_eq!(
        compare_fabric_versions("0.7.9+build.9", "0.7.8+build.10"),
        Ordering::Greater
    );
}
