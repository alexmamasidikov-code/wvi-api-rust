use axum::{extract::State, Json};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use sqlx::PgPool;
use uuid::Uuid;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};

use crate::error::{AppError, AppResult};

use super::models::*;

fn create_tokens(user_id: Uuid, email: &str, secret: &str, expiry_hours: i64) -> AppResult<(String, String, i64)> {
    let now = Utc::now().timestamp() as usize;
    let exp = now + (expiry_hours as usize * 3600);

    let claims = Claims {
        sub: user_id,
        email: email.to_string(),
        exp,
        iat: now,
    };

    let access_token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    let refresh_claims = Claims {
        sub: user_id,
        email: email.to_string(),
        exp: now + (expiry_hours as usize * 3600 * 7),
        iat: now,
    };

    let refresh_token = encode(
        &Header::default(),
        &refresh_claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok((access_token, refresh_token, expiry_hours * 3600))
}

pub async fn register(
    State(pool): State<PgPool>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let existing = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE email = $1")
        .bind(&req.email)
        .fetch_one(&pool)
        .await?;

    if existing > 0 {
        return Err(AppError::Conflict("Email already registered".into()));
    }

    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .to_string();

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, email, name, password_hash, created_at) VALUES ($1, $2, $3, $4, NOW())",
    )
    .bind(user_id)
    .bind(&req.email)
    .bind(&req.name)
    .bind(&password_hash)
    .execute(&pool)
    .await?;

    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "wvi-super-secret-key-change-in-production".into());
    let (access_token, refresh_token, expires_in) = create_tokens(user_id, &req.email, &secret, 24)?;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "userId": user_id,
            "email": req.email,
            "name": req.name,
            "accessToken": access_token,
            "refreshToken": refresh_token,
            "expiresIn": expires_in,
        }
    })))
}

pub async fn login(
    State(pool): State<PgPool>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT id, email, name, password_hash FROM users WHERE email = $1",
    )
    .bind(&req.email)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;

    let (user_id, email, name, stored_hash) = row;

    let parsed_hash = PasswordHash::new(&stored_hash)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let valid = Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .is_ok();

    if !valid {
        return Err(AppError::Unauthorized("Invalid credentials".into()));
    }

    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "wvi-super-secret-key-change-in-production".into());
    let (access_token, refresh_token, expires_in) = create_tokens(user_id, &email, &secret, 24)?;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "userId": user_id,
            "email": email,
            "name": name,
            "accessToken": access_token,
            "refreshToken": refresh_token,
            "expiresIn": expires_in,
        }
    })))
}

pub async fn refresh(
    Json(req): Json<RefreshRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "wvi-super-secret-key-change-in-production".into());

    let token_data = jsonwebtoken::decode::<Claims>(
        &req.refresh_token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    )?;

    let (access_token, _, expires_in) =
        create_tokens(token_data.claims.sub, &token_data.claims.email, &secret, 24)?;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "accessToken": access_token,
            "expiresIn": expires_in,
        }
    })))
}
