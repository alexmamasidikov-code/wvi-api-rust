use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult};
use super::models::{PrivyTokenResult, PrivyUser};

const PRIVY_API_BASE: &str = "https://auth.privy.io/api/v1";
const CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

/// Privy authentication client for Rust
pub struct PrivyClient {
    app_id: String,
    app_secret: String,
    http: reqwest::Client,
    cache: Mutex<HashMap<String, (PrivyTokenResult, Instant)>>,
}

impl PrivyClient {
    pub fn new(app_id: String, app_secret: String) -> Self {
        Self {
            app_id,
            app_secret,
            http: reqwest::Client::new(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Check if Privy is configured (non-empty credentials)
    pub fn is_configured(&self) -> bool {
        !self.app_id.is_empty() && !self.app_secret.is_empty()
    }

    /// Basic auth header value: base64(app_id:app_secret)
    fn basic_auth(&self) -> String {
        use std::io::Write;
        let mut buf = Vec::new();
        write!(buf, "{}:{}", self.app_id, self.app_secret).unwrap();
        format!("Basic {}", base64_encode(&buf))
    }

    /// Verify a Privy access token
    /// POST https://auth.privy.io/api/v1/token/verify
    pub async fn verify_token(&self, token: &str) -> AppResult<PrivyTokenResult> {
        // Check cache first
        if let Ok(cache) = self.cache.lock() {
            if let Some((result, cached_at)) = cache.get(token) {
                if cached_at.elapsed() < CACHE_TTL {
                    return Ok(result.clone());
                }
            }
        }

        let resp = self.http
            .post(format!("{PRIVY_API_BASE}/token/verify"))
            .header("Content-Type", "application/json")
            .header("privy-app-id", &self.app_id)
            .header("Authorization", self.basic_auth())
            .json(&serde_json::json!({ "token": token }))
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Privy request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Unauthorized(
                format!("Privy token verification failed ({status}): {body}")
            ));
        }

        let result: PrivyTokenResult = resp.json().await
            .map_err(|e| AppError::Internal(format!("Privy response parse error: {e}")))?;

        // Cache the result
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(token.to_string(), (result.clone(), Instant::now()));
            // Evict expired entries periodically
            if cache.len() > 1000 {
                cache.retain(|_, (_, ts)| ts.elapsed() < CACHE_TTL);
            }
        }

        Ok(result)
    }

    /// Get user details by Privy DID
    /// GET https://auth.privy.io/api/v1/users/{did}
    pub async fn get_user(&self, did: &str) -> AppResult<PrivyUser> {
        let resp = self.http
            .get(format!("{PRIVY_API_BASE}/users/{did}"))
            .header("privy-app-id", &self.app_id)
            .header("Authorization", self.basic_auth())
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Privy get_user failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(AppError::NotFound(format!("Privy user not found: {did}")));
        }

        resp.json().await
            .map_err(|e| AppError::Internal(format!("Privy user parse error: {e}")))
    }

    /// Get user by wallet address
    /// GET https://auth.privy.io/api/v1/users?wallet_address={address}
    pub async fn get_user_by_wallet(&self, address: &str) -> AppResult<PrivyUser> {
        let resp = self.http
            .get(format!("{PRIVY_API_BASE}/users"))
            .query(&[("wallet_address", address)])
            .header("privy-app-id", &self.app_id)
            .header("Authorization", self.basic_auth())
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Privy wallet lookup failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(AppError::NotFound(format!("No user with wallet: {address}")));
        }

        resp.json().await
            .map_err(|e| AppError::Internal(format!("Privy user parse error: {e}")))
    }
}

/// Simple base64 encode (no dependency needed)
fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
