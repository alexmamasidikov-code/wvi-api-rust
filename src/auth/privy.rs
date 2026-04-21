use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::error::{AppError, AppResult};
use super::models::{PrivyTokenResult, PrivyUser};

const PRIVY_API_BASE: &str = "https://auth.privy.io/api/v1";
const CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

/// Claims we actually read from the Privy access-token JWT.
/// Privy's spec: `sub` holds the user DID, `iss` is "privy.io",
/// `aud` matches the app id. Extra fields are ignored.
#[derive(Debug, Deserialize)]
struct PrivyClaims {
    sub: String,
    #[serde(default)]
    #[allow(dead_code)]
    iss: String,
    #[serde(default)]
    #[allow(dead_code)]
    aud: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code)]
    exp: i64,
}

/// Privy authentication client for Rust.
///
/// 2026-04-21: switched from the old HTTP `/api/v1/token/verify`
/// endpoint (now returns Privy's 404 HTML page) to local JWT
/// verification using the ES256 authorization public key that Privy
/// hands out in the app settings and the iOS app already ships. Every
/// `/biometrics/sync` request was failing 401 until this fix.
pub struct PrivyClient {
    app_id: String,
    app_secret: String,
    /// Pre-parsed ES256 decoding key. `None` means no verification key
    /// was configured and the middleware dev-user bypass applies.
    verify_key: Option<DecodingKey>,
    http: reqwest::Client,
    cache: Mutex<HashMap<String, (PrivyTokenResult, Instant)>>,
}

impl PrivyClient {
    pub fn new(app_id: String, app_secret: String, verification_key_pem: String) -> Self {
        // Accept either a ready-made PEM (with BEGIN/END lines) or a
        // bare base64 SPKI blob as Privy hands out. Wrap the latter so
        // `DecodingKey::from_ec_pem` understands it.
        let verify_key = if verification_key_pem.trim().is_empty() {
            None
        } else {
            let pem = if verification_key_pem.contains("BEGIN") {
                verification_key_pem.clone()
            } else {
                format!(
                    "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
                    verification_key_pem.trim()
                )
            };
            match DecodingKey::from_ec_pem(pem.as_bytes()) {
                Ok(k) => Some(k),
                Err(e) => {
                    tracing::error!("Privy verification key parse failed: {e}");
                    None
                }
            }
        };
        Self {
            app_id,
            app_secret,
            verify_key,
            http: reqwest::Client::new(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Check if Privy is configured (non-empty credentials + parsable key)
    pub fn is_configured(&self) -> bool {
        !self.app_id.is_empty()
            && !self.app_secret.is_empty()
            && self.verify_key.is_some()
    }

    /// Basic auth header value: base64(app_id:app_secret)
    fn basic_auth(&self) -> String {
        use std::io::Write;
        let mut buf = Vec::new();
        write!(buf, "{}:{}", self.app_id, self.app_secret).unwrap();
        format!("Basic {}", base64_encode(&buf))
    }

    /// Verify a Privy access token LOCALLY via ES256.
    ///
    /// Privy deprecated POST /api/v1/token/verify — it returns a
    /// generic 404 HTML page, which surfaced to iOS as
    /// `activity-history HTTP 401` on every sync attempt. Local
    /// verification is what Privy recommends now: decode the JWT
    /// signature against the authorization public key, check exp /
    /// iss / aud, and trust the `sub` claim as the user DID.
    pub async fn verify_token(&self, token: &str) -> AppResult<PrivyTokenResult> {
        // Check cache first
        if let Ok(cache) = self.cache.lock() {
            if let Some((result, cached_at)) = cache.get(token) {
                if cached_at.elapsed() < CACHE_TTL {
                    return Ok(result.clone());
                }
            }
        }

        let key = self.verify_key.as_ref().ok_or_else(|| {
            AppError::Internal("Privy verification key not configured".into())
        })?;

        // Privy uses ES256 (ECDSA P-256 / SHA-256).
        let mut validation = Validation::new(Algorithm::ES256);
        // Privy sets iss = "privy.io"; aud = our app_id. We don't want
        // to hard-fail on issuer so we can accept either "privy.io" or
        // the app id itself (legacy). We DO validate audience.
        validation.set_audience(&[self.app_id.clone()]);
        validation.validate_exp = true;
        // Give 60 s of skew for mild clock drift between phone, VPS
        // and Privy's issuer clock — same allowance the old HTTP
        // path had via TOKEN_SKEW_SECONDS.
        validation.leeway = 60;

        let claims = decode::<PrivyClaims>(token, key, &validation)
            .map_err(|e| AppError::Unauthorized(format!("Privy JWT verify failed: {e}")))?
            .claims;

        let result = PrivyTokenResult {
            user_id: claims.sub.clone(),
            session_id: String::new(),
        };

        // Cache the result
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(token.to_string(), (result.clone(), Instant::now()));
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
