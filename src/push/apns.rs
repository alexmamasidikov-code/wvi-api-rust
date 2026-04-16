//! APNs HTTP/2 client (token-based auth with a .p8 ES256 key).
//!
//! Env vars:
//!   APNS_KEY_P8        — contents of the .p8 file (PEM, `-----BEGIN PRIVATE KEY-----`)
//!   APNS_KEY_ID        — 10-character Key ID from developer.apple.com
//!   APNS_TEAM_ID       — 10-character Team ID
//!   APNS_BUNDLE_ID     — app bundle id, e.g. `com.wvi.health`
//!   APNS_ENV           — `development` (default) or `production`
//!
//! Missing env = no-op sends, warn-logged once. This keeps the rest of the
//! app working while the key is being procured.

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ApnsConfig {
    pub key_p8: String,
    pub key_id: String,
    pub team_id: String,
    pub bundle_id: String,
    pub host: String, // api.sandbox.push.apple.com or api.push.apple.com
}

impl ApnsConfig {
    pub fn from_env() -> Option<Self> {
        let key_p8 = std::env::var("APNS_KEY_P8").ok()?;
        let key_id = std::env::var("APNS_KEY_ID").ok()?;
        let team_id = std::env::var("APNS_TEAM_ID").ok()?;
        let bundle_id = std::env::var("APNS_BUNDLE_ID").ok()?;
        let env = std::env::var("APNS_ENV").unwrap_or_else(|_| "development".to_string());
        let host = if env == "production" {
            "api.push.apple.com".to_string()
        } else {
            "api.sandbox.push.apple.com".to_string()
        };
        Some(Self { key_p8, key_id, team_id, bundle_id, host })
    }
}

/// Cached JWT. APNs allows reuse for up to 1 hour; we refresh at 50 min.
#[derive(Clone)]
pub struct ApnsClient {
    config: Option<ApnsConfig>,
    http: Client,
    token: Arc<RwLock<Option<(String, u64)>>>, // (jwt, issued_at_unix)
}

impl ApnsClient {
    pub fn new() -> Self {
        let cfg = ApnsConfig::from_env();
        if cfg.is_none() {
            tracing::warn!("APNs: env not set — pushes will be no-op until APNS_KEY_P8 + APNS_KEY_ID + APNS_TEAM_ID + APNS_BUNDLE_ID are provided");
        } else {
            tracing::info!("APNs: configured ({}).", cfg.as_ref().unwrap().host);
        }
        let http = Client::builder()
            .http2_prior_knowledge()
            .build()
            .expect("reqwest http2 client");
        Self { config: cfg, http, token: Arc::new(RwLock::new(None)) }
    }

    pub fn is_configured(&self) -> bool {
        self.config.is_some()
    }

    async fn jwt(&self, cfg: &ApnsConfig) -> Result<String, String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs();
        {
            let guard = self.token.read().await;
            if let Some((t, issued)) = guard.as_ref() {
                if now.saturating_sub(*issued) < 3000 {
                    return Ok(t.clone());
                }
            }
        }
        #[derive(Serialize)]
        struct Claims<'a> { iss: &'a str, iat: u64 }
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(cfg.key_id.clone());
        let key = EncodingKey::from_ec_pem(cfg.key_p8.as_bytes()).map_err(|e| format!("apns key parse: {e}"))?;
        let jwt = encode(&header, &Claims { iss: &cfg.team_id, iat: now }, &key).map_err(|e| e.to_string())?;
        *self.token.write().await = Some((jwt.clone(), now));
        Ok(jwt)
    }

    /// Send a simple alert push. Returns Ok(()) on 200. On 400/410 (invalid
    /// token, unregistered) the caller should remove the token from DB.
    pub async fn send_alert(
        &self,
        device_token: &str,
        title: &str,
        body: &str,
        deeplink: Option<&str>,
    ) -> Result<(), String> {
        let cfg = match &self.config {
            Some(c) => c,
            None => return Ok(()), // silent no-op when unconfigured
        };
        let jwt = self.jwt(cfg).await?;

        let payload = serde_json::json!({
            "aps": {
                "alert": { "title": title, "body": body },
                "sound": "default",
                "mutable-content": 1,
            },
            "deeplink": deeplink,
        });

        let url = format!("https://{}/3/device/{}", cfg.host, device_token);
        let resp = self.http
            .post(&url)
            .bearer_auth(&jwt)
            .header("apns-topic", &cfg.bundle_id)
            .header("apns-push-type", "alert")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("apns http: {e}"))?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(format!("apns {status}: {body}"))
        }
    }
}
