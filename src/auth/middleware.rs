use std::sync::Arc;
use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
};

use super::privy::PrivyClient;

/// Authenticated user extracted from Privy token
pub struct AuthUser {
    pub privy_did: String,
    pub email: Option<String>,
}

/// App state that includes the Privy client and DB pool
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub privy: Arc<PrivyClient>,
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract Bearer token from Authorization header or privy-token cookie
        let token = extract_token(parts)
            .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization token".into()))?;

        // Get PrivyClient from extensions (set by middleware layer)
        let privy = parts.extensions.get::<Arc<PrivyClient>>()
            .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Privy client not configured".into()))?;

        // Dev mode: if Privy not configured, use dev user
        if !privy.is_configured() {
            return Ok(AuthUser {
                privy_did: "did:privy:dev-user".into(),
                email: Some("dev@wvi.health".into()),
            });
        }

        // Verify token with Privy
        let result = privy.verify_token(&token).await
            .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

        Ok(AuthUser {
            privy_did: result.user_id,
            email: None, // fetched separately if needed
        })
    }
}

fn extract_token(parts: &Parts) -> Option<String> {
    // Try Authorization: Bearer <token>
    if let Some(auth) = parts.headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(token) = auth.strip_prefix("Bearer ") {
            return Some(token.to_string());
        }
    }

    // Try privy-token cookie
    if let Some(cookie) = parts.headers.get("cookie").and_then(|v| v.to_str().ok()) {
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some(token) = part.strip_prefix("privy-token=") {
                return Some(token.to_string());
            }
        }
    }

    None
}
