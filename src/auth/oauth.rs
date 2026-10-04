// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// device code auth is used because it works without a redirect URI, which is nice for a TUI.

use std::sync::{Arc, Mutex};

use minecraft_msa_auth::MinecraftAuthorizationFlow;
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, ClientId, DeviceAuthorizationUrl, RefreshToken, Scope,
    StandardDeviceAuthorizationResponse, TokenResponse, TokenUrl,
};
use serde::Deserialize;

use super::accounts::{Account, AccountType, AuthResult};

const CLIENT_ID: &str = "cc1b2d89-8d8b-439f-94e6-4a7fc484f672";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const MSA_AUTHORIZE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const MSA_TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const MC_TOKEN_CACHE_REFRESH_MARGIN_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
}

#[derive(Deserialize)]
struct McProfile {
    id: String,
    name: String,
}

fn normalize_profile_uuid(id: &str) -> Option<String> {
    (id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| {
        format!(
            "{}-{}-{}-{}-{}",
            &id[..8],
            &id[8..12],
            &id[12..16],
            &id[16..20],
            &id[20..32],
        )
    })
}

#[derive(Default)]
pub struct MicrosoftAuth {
    pub result: Arc<Mutex<Option<AuthResult>>>,
    pub device_code: Arc<Mutex<Option<DeviceCodeInfo>>>,
}

async fn run_full_oauth_flow(
    http_client: &reqwest::Client,
    display: &Mutex<Option<DeviceCodeInfo>>,
) -> Result<(String, Option<String>), String> {
    let oauth_client = BasicClient::new(ClientId::new(CLIENT_ID.to_owned()))
        .set_auth_uri(AuthUrl::new(MSA_AUTHORIZE_URL.to_owned()).map_err(|e| e.to_string())?)
        .set_token_uri(TokenUrl::new(MSA_TOKEN_URL.to_owned()).map_err(|e| e.to_string())?)
        .set_device_authorization_url(
            DeviceAuthorizationUrl::new(DEVICE_CODE_URL.to_owned()).map_err(|e| e.to_string())?,
        );

    let details: StandardDeviceAuthorizationResponse = oauth_client
        .exchange_device_code()
        .add_scope(Scope::new("XboxLive.signin".to_owned()))
        .add_scope(Scope::new("offline_access".to_owned()))
        .request_async(http_client)
        .await
        .map_err(|e| format!("Device code request failed: {e}"))?;

    if let Ok(mut slot) = display.lock() {
        *slot = Some(DeviceCodeInfo {
            user_code: details.user_code().secret().to_owned(),
            verification_uri: details.verification_uri().to_string(),
        });
        crate::feedback::request_redraw();
    }

    let token = oauth_client
        .exchange_device_access_token(&details)
        .request_async(http_client, tokio::time::sleep, None)
        .await
        .map_err(|e| format!("Authentication failed: {e}"))?;

    let ms_access_token = token.access_token().secret().to_owned();
    let ms_refresh_token = token.refresh_token().map(|r| r.secret().to_owned());

    Ok((ms_access_token, ms_refresh_token))
}

pub fn start_microsoft_auth() -> MicrosoftAuth {
    let auth = MicrosoftAuth::default();
    let result_clone = auth.result.clone();
    let device_code = auth.device_code.clone();

    tokio::spawn(async move {
        let outcome = run_full_auth_flow(&device_code).await;
        if let Ok(mut slot) = result_clone.lock() {
            *slot = Some(outcome);
            crate::feedback::request_redraw();
        }
    });

    auth
}

async fn run_full_auth_flow(display: &Mutex<Option<DeviceCodeInfo>>) -> AuthResult {
    let http_client = match auth_http_client() {
        Ok(client) => client,
        Err(error) => return AuthResult::Error(error),
    };
    let (ms_access_token, ms_refresh_token) = match run_full_oauth_flow(&http_client, display).await
    {
        Ok(pair) => pair,
        Err(e) => return AuthResult::Error(e),
    };

    let Some(refresh_token) = ms_refresh_token.as_deref() else {
        return AuthResult::Error(
            "Microsoft did not return a refresh token; try signing in again".to_owned(),
        );
    };
    exchange_and_build_account(&http_client, &ms_access_token, Some(refresh_token)).await
}

fn auth_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("Could not create authentication HTTP client: {error}"))
}

async fn exchange_and_build_account(
    client: &reqwest::Client,
    ms_access_token: &str,
    ms_refresh_token: Option<&str>,
) -> AuthResult {
    let mc_flow = MinecraftAuthorizationFlow::new(client.clone());
    let mc_token = match mc_flow.exchange_microsoft_token(ms_access_token).await {
        Ok(t) => t,
        Err(e) => return AuthResult::Error(format!("Minecraft auth failed: {e}")),
    };

    let profile =
        match fetch_profile(client, MC_PROFILE_URL, mc_token.access_token().as_ref()).await {
            Ok(p) => p,
            Err(e) => return AuthResult::Error(e),
        };

    // mojang returns uuids without dashes
    let uuid = match normalize_profile_uuid(&profile.id) {
        Some(uuid) => uuid,
        None => return AuthResult::Error("Profile returned an invalid UUID".to_owned()),
    };

    AuthResult::Success(Account {
        uuid,
        username: profile.name,
        account_type: AccountType::Microsoft,
        active: false,
        refresh_token: ms_refresh_token.map(|s| s.to_owned()),
        cached_mc_token: Some(mc_token.access_token().as_ref().to_owned()),
        cached_mc_token_expires_at: Some(
            chrono::Utc::now().timestamp() + i64::from(mc_token.expires_in()),
        ),
    })
}

async fn fetch_profile(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<McProfile, String> {
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Profile fetch failed: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("Account does not own Minecraft".to_owned());
    }
    let response = response
        .error_for_status()
        .map_err(|e| format!("Profile fetch failed: {e}"))?;
    response
        .json()
        .await
        .map_err(|e| format!("Profile parse failed: {e}"))
}

fn valid_cached_mc_token(account: &Account, now: i64) -> Option<&str> {
    let (Some(cached), Some(expires_at)) = (
        account.cached_mc_token.as_deref(),
        account.cached_mc_token_expires_at,
    ) else {
        return None;
    };

    if now < expires_at.saturating_sub(MC_TOKEN_CACHE_REFRESH_MARGIN_SECS) {
        Some(cached)
    } else {
        None
    }
}

// cached tokens return no expiry so callers don't rewrite the account store.
pub async fn refresh_and_get_token(
    account: &Account,
) -> Result<(String, Option<String>, Option<i64>), String> {
    match account.account_type {
        AccountType::Offline => Ok(("0".to_owned(), None, None)),
        AccountType::Microsoft => {
            let now = chrono::Utc::now().timestamp();
            if let Some(cached) = valid_cached_mc_token(account, now) {
                tracing::info!("Using cached Minecraft token for '{}'", account.username);
                return Ok((cached.to_owned(), None, None));
            }

            tracing::info!("Refreshing Minecraft token for '{}'", account.username);

            let refresh = account.refresh_token.as_deref().ok_or_else(|| {
                format!(
                    "No saved credentials for '{}'. Please remove and re-add the account.",
                    account.username
                )
            })?;

            let oauth_client = BasicClient::new(ClientId::new(CLIENT_ID.to_owned()))
                .set_auth_uri(
                    AuthUrl::new(MSA_AUTHORIZE_URL.to_owned()).map_err(|e| e.to_string())?,
                )
                .set_token_uri(TokenUrl::new(MSA_TOKEN_URL.to_owned()).map_err(|e| e.to_string())?);

            let http_client = auth_http_client()?;

            let token = oauth_client
                .exchange_refresh_token(&RefreshToken::new(refresh.to_owned()))
                .add_scope(Scope::new("XboxLive.signin".to_owned()))
                .add_scope(Scope::new("offline_access".to_owned()))
                .request_async(&http_client)
                .await
                .map_err(|e| format!("Token refresh failed: {e}"))?;

            let ms_access_token = token.access_token().secret().to_owned();
            let new_refresh = token.refresh_token().map(|r| r.secret().to_owned());

            let mc_flow = MinecraftAuthorizationFlow::new(http_client);
            let mc_token = mc_flow
                .exchange_microsoft_token(&ms_access_token)
                .await
                .map_err(|e| format!("Minecraft auth failed: {e}"))?;

            let expires_at = chrono::Utc::now().timestamp() + i64::from(mc_token.expires_in());

            Ok((
                mc_token.access_token().as_ref().to_owned(),
                new_refresh,
                Some(expires_at),
            ))
        }
    }
}

#[cfg(test)]
#[path = "tests/oauth.rs"]
mod tests;
