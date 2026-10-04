// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub uuid: String,
    pub username: String,
    pub account_type: AccountType,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_mc_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_mc_token_expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AccountType {
    Microsoft,
    Offline,
}

#[derive(Debug)]
pub enum AuthResult {
    Success(Account),
    Error(String),
}

pub struct AccountStore {
    pub accounts: Vec<Account>,
    path: PathBuf,
    load_error: Option<String>,
}

impl AccountStore {
    pub fn load() -> Self {
        Self::load_from(account_store_path())
    }

    fn load_from(path: PathBuf) -> Self {
        let (accounts, load_error) = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(accounts) => (accounts, None),
                Err(e) => {
                    tracing::warn!("Failed to parse accounts file {}: {}", path.display(), e);
                    (
                        Vec::new(),
                        Some(format!("Invalid accounts file {}: {e}", path.display())),
                    )
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("No accounts file at {}", path.display());
                (Vec::new(), None)
            }
            Err(e) => {
                tracing::warn!("Failed to read accounts file {}: {}", path.display(), e);
                (
                    Vec::new(),
                    Some(format!("Cannot read accounts file {}: {e}", path.display())),
                )
            }
        };
        tracing::debug!(
            "Loaded {} account(s) from {}",
            accounts.len(),
            path.display()
        );
        Self {
            accounts,
            path,
            load_error,
        }
    }

    pub fn check_loaded(&self) -> std::io::Result<()> {
        if let Some(error) = &self.load_error {
            return Err(std::io::Error::other(error.clone()));
        }
        Ok(())
    }

    fn save(&self) -> std::io::Result<()> {
        self.check_loaded()?;
        let json = serde_json::to_vec_pretty(&self.accounts).map_err(std::io::Error::other)?;
        crate::storage::write_atomic_private(&self.path, &json)?;
        tracing::debug!(
            "Saved {} account(s) to {}",
            self.accounts.len(),
            self.path.display()
        );
        Ok(())
    }

    pub fn active_account(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| a.active)
    }

    pub fn has_microsoft_account(&self) -> bool {
        self.accounts
            .iter()
            .any(|account| account.account_type == AccountType::Microsoft)
    }

    pub fn set_active(&mut self, index: usize) -> std::io::Result<()> {
        let Some(account) = self.accounts.get(index) else {
            // Invalid selection must leave the active account intact.
            tracing::warn!("Tried to select missing account index {}", index);
            return Ok(());
        };
        let uuid = account.uuid.clone();
        self.update(|accounts| {
            if !accounts.iter().any(|account| account.uuid == uuid) {
                return Err(std::io::Error::other("Selected account no longer exists"));
            }
            for account in accounts {
                account.active = account.uuid == uuid;
            }
            Ok(())
        })
    }

    pub fn add(&mut self, account: Account) -> std::io::Result<()> {
        self.update(|accounts| {
            let replaced_active = accounts
                .iter()
                .any(|old| old.uuid == account.uuid && old.active);
            accounts.retain(|old| old.uuid != account.uuid);
            let mut account = account;
            account.active = accounts.is_empty() || replaced_active;
            accounts.push(account);
            Ok(())
        })
    }

    pub fn remove(&mut self, index: usize) -> std::io::Result<()> {
        if index >= self.accounts.len() {
            tracing::warn!("Tried to remove missing account index {}", index);
            return Ok(());
        }
        let uuid = self.accounts[index].uuid.clone();
        self.update(|accounts| {
            let index = accounts
                .iter()
                .position(|account| account.uuid == uuid)
                .ok_or_else(|| std::io::Error::other("Selected account no longer exists"))?;
            let account = accounts.remove(index);
            if account.active
                && let Some(first) = accounts.first_mut()
            {
                first.active = true;
            }
            Ok(())
        })
    }

    pub fn update_credentials(
        &mut self,
        uuid: &str,
        refresh: Option<String>,
        token: &str,
        expires: Option<i64>,
    ) -> std::io::Result<()> {
        if refresh.is_none() && expires.is_none() {
            return Ok(());
        }
        self.update(|accounts| {
            let account = accounts
                .iter_mut()
                .find(|account| account.uuid == uuid)
                .ok_or_else(|| std::io::Error::other("Authenticated account no longer exists"))?;
            if let Some(refresh) = refresh {
                account.refresh_token = Some(refresh);
            }
            if let Some(expires) = expires {
                account.cached_mc_token = Some(token.to_owned());
                account.cached_mc_token_expires_at = Some(expires);
            }
            Ok(())
        })
    }

    fn update(
        &mut self,
        update: impl FnOnce(&mut Vec<Account>) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        use fs2::FileExt;
        self.check_loaded()?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        lock.lock_exclusive()?;
        let mut current = Self::load_from(self.path.clone());
        current.check_loaded()?;
        update(&mut current.accounts)?;
        current.save()?;
        self.accounts = current.accounts;
        Ok(())
    }
}

pub fn account_store_path() -> PathBuf {
    crate::config::get_config_path().join("accounts.json")
}

// Keep the existing mapping so saved offline player data retains its identifier.
pub fn offline_uuid(username: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    format!("OfflinePlayer:{username}").hash(&mut hasher);
    let h = hasher.finish();
    format!(
        "{:08x}-{:04x}-3{:03x}-{:04x}-{:012x}",
        (h >> 32) as u32,
        (h >> 16) as u16,
        (h >> 4) as u16 & 0x0FFF,
        (h as u16 & 0x3FFF) | 0x8000,
        h & 0xFFFFFFFFFFFF,
    )
}

pub fn create_offline_account(username: &str) -> Account {
    Account {
        uuid: offline_uuid(username),
        username: username.to_owned(),
        account_type: AccountType::Offline,
        active: false,
        refresh_token: None,
        cached_mc_token: None,
        cached_mc_token_expires_at: None,
    }
}

#[cfg(test)]
#[path = "tests/accounts.rs"]
mod tests;
