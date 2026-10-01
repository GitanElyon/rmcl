// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::io;
use std::time::Duration;

use clap::ArgMatches;

use crate::auth::{Account, AccountStore, AuthResult};
use crate::cli::output::{active_marker, print_table};

type CliResult = Result<(), Box<dyn std::error::Error>>;

pub async fn handle_account(matches: &ArgMatches) -> CliResult {
    match matches.subcommand() {
        Some(("list", _)) => list_accounts(),
        Some(("add", sub_matches)) => add_account(sub_matches).await,
        Some(("delete", sub_matches)) => delete_account(sub_matches),
        Some(("use", sub_matches)) => use_account(sub_matches),
        _ => Ok(()),
    }
}

fn list_accounts() -> CliResult {
    let store = AccountStore::load();
    store.check_loaded()?;
    let rows = store
        .accounts
        .iter()
        .map(|account| {
            vec![
                active_marker(account.active).to_string(),
                account.username.clone(),
                format!("{:?}", account.account_type),
                account.uuid.clone(),
            ]
        })
        .collect::<Vec<_>>();

    print_table(&[" ", "Username", "Type", "UUID"], &rows);
    Ok(())
}

async fn add_account(matches: &ArgMatches) -> CliResult {
    if matches.get_flag("microsoft") {
        return add_microsoft_account().await;
    }

    if let Some(username) = matches.get_one::<String>("offline") {
        let mut store = AccountStore::load();
        add_offline_account(&mut store, username)?;
        println!("Added offline account '{}'.", username);
    }

    Ok(())
}

async fn add_microsoft_account() -> CliResult {
    let result_arc = crate::auth::start_microsoft_auth();

    let info = wait_for_device_code(&result_arc).await?;
    println!("Open: {}", info.verification_uri);
    println!("Code: {}", info.user_code);

    loop {
        if let Ok(slot) = result_arc.result.lock()
            && let Some(result) = slot.as_ref()
        {
            return match result {
                AuthResult::Success(account) => {
                    let mut store = AccountStore::load();
                    store.add(account.clone())?;
                    println!("Added Microsoft account '{}'.", account.username);
                    Ok(())
                }
                AuthResult::Error(message) => Err(io::Error::other(message.clone()).into()),
            };
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_for_device_code(
    result: &crate::auth::MicrosoftAuth,
) -> io::Result<crate::auth::DeviceCodeInfo> {
    loop {
        if let Ok(slot) = result.device_code.lock()
            && let Some(info) = slot.as_ref()
        {
            return Ok(info.clone());
        }
        if let Ok(slot) = result.result.lock()
            && let Some(AuthResult::Error(message)) = slot.as_ref()
        {
            return Err(io::Error::other(message.clone()));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn add_offline_account(store: &mut AccountStore, username: &str) -> CliResult {
    store.check_loaded()?;
    let username = username.trim();
    if username.is_empty() {
        return Err(io::Error::other("offline username cannot be empty").into());
    }
    if !store.has_microsoft_account() {
        return Err(io::Error::other(
            "add a Microsoft account that owns Minecraft before adding offline accounts",
        )
        .into());
    }

    store.add(crate::auth::create_offline_account(username))?;
    Ok(())
}

fn delete_account(matches: &ArgMatches) -> CliResult {
    let username = required_arg(matches, "username")?;
    let mut store = AccountStore::load();
    store.check_loaded()?;
    let index = find_account_index(&store.accounts, username)?;

    if !matches.get_flag("yes") && !confirm(&format!("Delete '{}'", username))? {
        println!("Cancelled.");
        return Ok(());
    }

    store.remove(index)?;
    println!("Deleted '{}'.", username);
    Ok(())
}

fn use_account(matches: &ArgMatches) -> CliResult {
    let username = required_arg(matches, "username")?;
    let mut store = AccountStore::load();
    store.check_loaded()?;
    let index = find_account_index(&store.accounts, username)?;
    store.set_active(index)?;
    println!("Active account set to '{}'.", username);
    Ok(())
}

fn find_account_index(accounts: &[Account], selector: &str) -> io::Result<usize> {
    if let Some(index) = accounts.iter().position(|account| account.uuid == selector) {
        return Ok(index);
    }
    let mut matches = accounts
        .iter()
        .enumerate()
        .filter(|(_, account)| account.username.eq_ignore_ascii_case(selector));
    let (index, _) = matches
        .next()
        .ok_or_else(|| io::Error::other(format!("account '{selector}' not found")))?;
    if matches.next().is_some() {
        return Err(io::Error::other(format!(
            "Account '{selector}' is ambiguous; use its UUID"
        )));
    }
    Ok(index)
}

use super::utils::{confirm, required_arg};

#[cfg(test)]
#[path = "tests/account.rs"]
mod tests;
