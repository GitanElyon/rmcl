// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use super::add_offline_account;
use crate::auth::{Account, AccountStore, AccountType};

fn microsoft_account() -> Account {
    Account {
        uuid: "00000000-0000-0000-0000-000000000001".to_owned(),
        username: "Owner".to_owned(),
        account_type: AccountType::Microsoft,
        active: false,
        refresh_token: Some("refresh".to_owned()),
        cached_mc_token: None,
        cached_mc_token_expires_at: None,
    }
}

#[test]
fn creates_offline_account_after_microsoft_account_exists() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = AccountStore::empty_for_test(temp.path().join("accounts.json"));
    store.add(microsoft_account()).unwrap();
    add_offline_account(&mut store, "Steve").expect("offline account should be added");

    assert_eq!(store.accounts.len(), 2);
    assert_eq!(store.accounts[1].username, "Steve");
    assert_eq!(store.accounts[1].account_type, AccountType::Offline);
}

#[test]
fn rejects_offline_account_before_microsoft_account_exists() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = AccountStore::empty_for_test(temp.path().join("accounts.json"));
    let err = add_offline_account(&mut store, "Steve")
        .expect_err("offline account should require a microsoft account");

    assert!(err.to_string().contains("Microsoft account"));
    assert!(store.accounts.is_empty());
}

#[test]
fn rejects_empty_offline_username() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = AccountStore::empty_for_test(temp.path().join("accounts.json"));
    assert!(add_offline_account(&mut store, "   ").is_err());
}

#[test]
fn duplicate_usernames_require_a_unique_account_selector() {
    let mut microsoft = microsoft_account();
    microsoft.username = "Steve".to_owned();
    let offline = crate::auth::create_offline_account("Steve");
    let accounts = vec![microsoft, offline.clone()];

    assert!(super::find_account_index(&accounts, "Steve").is_err());
    assert_eq!(
        super::find_account_index(&accounts, &offline.uuid).unwrap(),
        1
    );
}

#[test]
fn device_code_failure_does_not_wait_for_a_code() {
    let _guard = crate::tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let result = crate::auth::MicrosoftAuth::default();
    *result.result.lock().unwrap() = Some(crate::auth::AuthResult::Error(
        "device code failed".to_owned(),
    ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let error = runtime
        .block_on(super::wait_for_device_code(&result))
        .unwrap_err();
    assert!(error.to_string().contains("device code failed"));
}
