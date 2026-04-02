use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use uuid::Uuid;

use super::models::Claims;

pub struct AuthUser {
    pub user_id: Uuid,
    pub email: String,
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header".into()))?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "Invalid Authorization format".into()))?;

        let jwt_secret = std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "wvi-super-secret-key-change-in-production".into());

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|e| (StatusCode::UNAUTHORIZED, format!("Invalid token: {e}")))?;

        Ok(AuthUser {
            user_id: token_data.claims.sub,
            email: token_data.claims.email,
        })
    }
}
