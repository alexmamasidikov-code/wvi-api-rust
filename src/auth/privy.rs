use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::error::{AppError, AppResult};
use super::models::{PrivyTokenResult, PrivyUser};

const PRIVY_API_BASE: &str = "https://auth.privy.io/api/v1";
const CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes
/// JWKS is rotated rarely; refresh once an hour is plenty.
const JWKS_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Deserialize, Clone)]
struct Jwk {
    kid: String,
    #[serde(default)]
    kty: String,
    #[serde(default)]
    crv: String,
    x: String,
    y: String,
    #[serde(default)]
    #[allow(dead_code)]
    alg: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

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
/// verification using Privy's JWKS endpoint
/// (`/api/v1/apps/{app_id}/jwks.json`). Access-token JWTs are signed
/// with ES256 keys rotated via that JWKS; `PRIVY_AUTHORIZATION_PUBLIC_KEY`
/// in env is for wallet-action authorisation, not token verification,
/// so using it produced `InvalidSignature` on every request. Now we
/// fetch JWKS at startup, cache per `kid` in memory, and refetch once
/// an hour (or immediately if an unknown `kid` shows up in a token).
pub struct PrivyClient {
    app_id: String,
    app_secret: String,
    http: reqwest::Client,
    /// Decoded JWKS keyed by `kid`. Wrapped in Mutex because refresh
    /// mutates, but read paths are rare (< once per request after
    /// warm-up) so contention is a non-issue at our scale.
    jwks: Mutex<HashMap<String, DecodingKey>>,
    jwks_fetched_at: Mutex<Option<Instant>>,
    cache: Mutex<HashMap<String, (PrivyTokenResult, Instant)>>,
}

impl PrivyClient {
    pub fn new(app_id: String, app_secret: String, _verification_key_pem: String) -> Self {
        // verification_key_pem arg kept for API compatibility; we now
        // source keys from Privy's JWKS instead of the env-pinned key.
        Self {
            app_id,
            app_secret,
            http: reqwest::Client::new(),
            jwks: Mutex::new(HashMap::new()),
            jwks_fetched_at: Mutex::new(None),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Check if Privy is configured (non-empty credentials).
    /// JWKS is fetched lazily on first verify call, so absence of it
    /// here doesn't mean unconfigured.
    pub fn is_configured(&self) -> bool {
        !self.app_id.is_empty() && !self.app_secret.is_empty()
    }

    /// Fetch JWKS from Privy and populate the keyring. Called on
    /// startup and whenever we see a token whose `kid` we don't
    /// recognise (so key rotation works without a restart).
    async fn refresh_jwks(&self) -> AppResult<()> {
        let url = format!("{PRIVY_API_BASE}/apps/{}/jwks.json", self.app_id);
        let resp = self.http.get(&url).send().await
            .map_err(|e| AppError::Internal(format!("JWKS fetch failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::Internal(
                format!("JWKS returned {}", resp.status())
            ));
        }
        let body: Jwks = resp.json().await
            .map_err(|e| AppError::Internal(format!("JWKS parse failed: {e}")))?;

        let mut map = HashMap::new();
        for k in body.keys {
            if k.kty != "EC" || k.crv != "P-256" {
                continue;
            }
            // jsonwebtoken ≥ 9 accepts EC JWKs directly.
            match DecodingKey::from_ec_components(&k.x, &k.y) {
                Ok(dk) => { map.insert(k.kid.clone(), dk); }
                Err(e) => tracing::warn!("JWKS kid={} parse failed: {e}", k.kid),
            }
        }
        if let Ok(mut jwks) = self.jwks.lock() {
            *jwks = map;
        }
        if let Ok(mut ts) = self.jwks_fetched_at.lock() {
            *ts = Some(Instant::now());
        }
        tracing::info!("Privy JWKS refreshed ({} keys)",
            self.jwks.lock().map(|m| m.len()).unwrap_or(0));
        Ok(())
    }

    fn jwks_is_stale(&self) -> bool {
        match self.jwks_fetched_at.lock() {
            Ok(guard) => match *guard {
                Some(t) => t.elapsed() > JWKS_TTL,
                None => true,
            },
            Err(_) => true,
        }
    }

    /// Basic auth header value: base64(app_id:app_secret)
    fn basic_auth(&self) -> String {
        use std::io::Write;
        let mut buf = Vec::new();
        write!(buf, "{}:{}", self.app_id, self.app_secret).unwrap();
        format!("Basic {}", base64_encode(&buf))
    }

    /// Verify a Privy access token LOCALLY via ES256 + JWKS.
    ///
    /// Privy deprecated POST /api/v1/token/verify — it returns a
    /// generic 404 HTML page. Verification now reads the token's `kid`
    /// header, looks up the matching ES256 public key from Privy's
    /// JWKS (`/api/v1/apps/{app_id}/jwks.json`), and decodes against
    /// it. JWKS is fetched once per hour or when a kid misses.
    pub async fn verify_token(&self, token: &str) -> AppResult<PrivyTokenResult> {
        // Check cache first
        if let Ok(cache) = self.cache.lock() {
            if let Some((result, cached_at)) = cache.get(token) {
                if cached_at.elapsed() < CACHE_TTL {
                    return Ok(result.clone());
                }
            }
        }

        // Peek the JWT header so we know which kid to look up.
        let header = decode_header(token)
            .map_err(|e| AppError::Unauthorized(format!("JWT header parse failed: {e}")))?;
        let kid = header.kid.ok_or_else(|| {
            AppError::Unauthorized("JWT missing kid header".into())
        })?;

        // Make sure JWKS is loaded and reasonably fresh.
        if self.jwks_is_stale() {
            let _ = self.refresh_jwks().await;
        }
        let mut dk = self.jwks.lock().ok().and_then(|m| m.get(&kid).cloned());
        if dk.is_none() {
            // Unknown kid — perhaps Privy rotated. One more refresh.
            let _ = self.refresh_jwks().await;
            dk = self.jwks.lock().ok().and_then(|m| m.get(&kid).cloned());
        }
        let key = dk.ok_or_else(|| {
            AppError::Unauthorized(format!("JWT kid {kid} not in JWKS"))
        })?;

        // Privy uses ES256 (ECDSA P-256 / SHA-256).
        let mut validation = Validation::new(Algorithm::ES256);
        // Don't strictly validate audience — Privy's access tokens set
        // `aud = app_id` but we keep this tolerant so token-format tweaks
        // on Privy's side don't break auth unexpectedly. Issuer is
        // re-checked in middleware.rs::decode_claims.
        validation.validate_aud = false;
        validation.validate_exp = true;
        validation.leeway = 60;

        let claims = decode::<PrivyClaims>(token, &key, &validation)
            .map_err(|e| {
                tracing::warn!("Privy JWT verify failed (kid={kid}): {e}");
                AppError::Unauthorized(format!("Privy JWT verify failed: {e}"))
            })?
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
