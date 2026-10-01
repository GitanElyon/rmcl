// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

mod accounts;
mod oauth;

pub use accounts::{
    Account, AccountStore, AccountType, AuthResult, account_store_path, create_offline_account,
    offline_uuid,
};
pub use oauth::{DeviceCodeInfo, MicrosoftAuth, refresh_and_get_token, start_microsoft_auth};
