// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn profile_status_errors_and_stalls_are_reported_accurately() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    let client = reqwest::Client::builder()
        .read_timeout(std::time::Duration::from_millis(30))
        .build()
        .unwrap();
    for status in [404, 401, 429, 503] {
        let route = format!("/{status}");
        Mock::given(path(&route))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let error = fetch_profile(&client, &format!("{}{route}", server.uri()), "test")
            .await
            .err()
            .unwrap();
        if status == 404 {
            assert!(error.contains("does not own"));
        } else {
            assert!(error.contains(&status.to_string()));
            assert!(!error.contains("does not own"));
        }
    }
    Mock::given(path("/stall"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(1)))
        .mount(&server)
        .await;
    assert!(
        fetch_profile(&client, &format!("{}/stall", server.uri()), "test")
            .await
            .is_err()
    );
}

fn microsoft_account(cached_mc_token_expires_at: Option<i64>) -> Account {
    Account {
        uuid: "00000000-0000-0000-0000-000000000000".to_owned(),
        username: "TestPlayer".to_owned(),
        account_type: AccountType::Microsoft,
        active: true,
        refresh_token: Some("refresh".to_owned()),
        cached_mc_token: Some("cached".to_owned()),
        cached_mc_token_expires_at,
    }
}

#[test]
fn cached_mc_token_is_valid_before_refresh_margin() {
    let now = 1_000;
    let account = microsoft_account(Some(now + MC_TOKEN_CACHE_REFRESH_MARGIN_SECS + 1));

    assert_eq!(valid_cached_mc_token(&account, now), Some("cached"));
}

#[test]
fn cached_mc_token_expires_inside_refresh_margin() {
    let now = 1_000;
    let account = microsoft_account(Some(now + MC_TOKEN_CACHE_REFRESH_MARGIN_SECS));

    assert!(valid_cached_mc_token(&account, now).is_none());
    assert!(valid_cached_mc_token(&microsoft_account(Some(i64::MIN)), now).is_none());
}

#[test]
fn cached_mc_token_requires_expiry() {
    let account = microsoft_account(None);

    assert!(valid_cached_mc_token(&account, 1_000).is_none());
}

#[test]
fn profile_uuid_is_normalized_without_slicing_unicode() {
    assert_eq!(
        normalize_profile_uuid("0123456789abcdef0123456789abcdef"),
        Some("01234567-89ab-cdef-0123-456789abcdef".to_owned())
    );

    let unicode_id = format!("{}é{}", "a".repeat(7), "b".repeat(23));
    assert_eq!(unicode_id.len(), 32);
    assert_eq!(normalize_profile_uuid(&unicode_id), None);
    assert_eq!(
        normalize_profile_uuid("not-a-valid-minecraft-profile-id"),
        None
    );
}
