use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use axum::{
    extract::{FromRequestParts, Request},
    http::{header::{AUTHORIZATION, WWW_AUTHENTICATE}, request::Parts, HeaderName, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};
use serde::Deserialize;

use super::privy::PrivyClient;

pub struct AuthUser { pub privy_did: String, pub email: Option<String> }

#[derive(Clone)]
pub struct AppState { pub pool: sqlx::PgPool, pub privy: Arc<PrivyClient> }

#[derive(Clone, Copy)]
struct RefreshHint;

#[derive(Debug, Deserialize)]
struct JwtClaims { #[serde(default)] exp: u64, #[serde(default)] iss: String }

type Rej = (StatusCode, [(HeaderName, String); 1], String);

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
fn skew_secs() -> u64 {
    std::env::var("TOKEN_SKEW_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(60)
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut map = [255u8; 256];
    for (i, c) in T.iter().enumerate() { map[*c as usize] = i as u8; }
    let (mut out, mut buf, mut bits) = (Vec::new(), 0u32, 0u32);
    for &b in s.as_bytes() {
        let v = map[b as usize];
        if v == 255 { return None; }
        buf = (buf << 6) | v as u32; bits += 6;
        if bits >= 8 { bits -= 8; out.push(((buf >> bits) & 0xFF) as u8); }
    }
    Some(out)
}

fn decode_claims(token: &str) -> Option<JwtClaims> {
    let mut p = token.split('.'); p.next()?;
    serde_json::from_slice(&b64url_decode(p.next()?)?).ok()
}

fn unauth(msg: &str, desc: &str) -> Rej {
    let hdr = format!(r#"Bearer error="invalid_token", error_description="{desc}""#);
    (StatusCode::UNAUTHORIZED, [(WWW_AUTHENTICATE, hdr)], msg.into())
}

impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = Rej;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Rej> {
        let token = extract_token(parts).ok_or_else(|| unauth("missing token", "missing token"))?;
        let privy = parts.extensions.get::<Arc<PrivyClient>>()
            .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, [(WWW_AUTHENTICATE, "Bearer".into())], "privy not configured".into()))?
            .clone();
        if !privy.is_configured() {
            return Ok(AuthUser { privy_did: "did:privy:dev-user".into(), email: Some("dev@wvi.health".into()) });
        }

        if let Some(claims) = decode_claims(&token) {
            let now = now_secs();
            let skew = skew_secs();
            if claims.exp > 0 && claims.exp + skew < now {
                return Err(unauth("token expired", "token expired"));
            }
            let expected = std::env::var("PRIVY_APP_ID").unwrap_or_default();
            if !expected.is_empty() && !claims.iss.is_empty()
                && claims.iss != expected && claims.iss != "privy.io" {
                return Err(unauth("issuer mismatch", "issuer mismatch"));
            }
            if claims.exp > now && claims.exp - now < 300 {
                parts.extensions.insert(RefreshHint);
            }
        }

        let result = privy.verify_token(&token).await
            .map_err(|e| unauth(&e.to_string(), "verification failed"))?;
        Ok(AuthUser { privy_did: result.user_id, email: None })
    }
}

/// Response middleware: emits `X-Token-Refresh-Hint: true` when extractor flagged it.
pub async fn inject_refresh_hint(request: Request, next: Next) -> Response {
    let flag = request.extensions().get::<RefreshHint>().is_some();
    let mut response = next.run(request).await;
    if flag {
        response.headers_mut().insert("X-Token-Refresh-Hint", HeaderValue::from_static("true"));
    }
    response
}

fn extract_token(parts: &Parts) -> Option<String> {
    if let Some(auth) = parts.headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(t) = auth.strip_prefix("Bearer ") { return Some(t.to_string()); }
    }
    if let Some(cookie) = parts.headers.get("cookie").and_then(|v| v.to_str().ok()) {
        for part in cookie.split(';') {
            if let Some(t) = part.trim().strip_prefix("privy-token=") { return Some(t.to_string()); }
        }
    }
    None
}
