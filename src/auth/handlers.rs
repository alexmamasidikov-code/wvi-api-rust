use std::sync::Arc;
use axum::{extract::State, Extension, Json};
use chrono::Utc;
use sqlx::PgPool;

use crate::error::{AppError, AppResult};
use super::middleware::AuthUser;
use super::models::*;
use super::privy::PrivyClient;

/// POST /auth/verify — Verify Privy token and upsert user in DB
pub async fn verify(
    State(pool): State<PgPool>,
    Extension(privy): Extension<Arc<PrivyClient>>,
    Json(req): Json<VerifyTokenRequest>,
) -> AppResult<Json<serde_json::Value>> {
    // Verify token with Privy
    let token_result = privy.verify_token(&req.token).await?;
    let did = &token_result.user_id;

    // Fetch full user from Privy
    let privy_user = privy.get_user(did).await.ok();
    let email = privy_user.as_ref().and_then(|u| u.email());
    let wallet = privy_user.as_ref().and_then(|u| u.wallet_address());
    let linked = privy_user.as_ref()
        .map(|u| serde_json::to_value(&u.linked_accounts).unwrap_or_default())
        .unwrap_or(serde_json::json!([]));

    // Upsert user in PostgreSQL
    sqlx::query(
        r#"INSERT INTO users (id, privy_did, email, name, linked_accounts, created_at, updated_at)
           VALUES (gen_random_uuid(), $1, $2, $2, $3, NOW(), NOW())
           ON CONFLICT (privy_did) DO UPDATE SET
             email = COALESCE(EXCLUDED.email, users.email),
             linked_accounts = EXCLUDED.linked_accounts,
             updated_at = NOW()"#,
    )
    .bind(did)
    .bind(&email)
    .bind(&linked)
    .execute(&pool)
    .await?;

    // Fetch the user back
    let row = sqlx::query_as::<_, (String, Option<String>, Option<String>, chrono::DateTime<Utc>)>(
        "SELECT privy_did, email, name, created_at FROM users WHERE privy_did = $1",
    )
    .bind(did)
    .fetch_one(&pool)
    .await?;

    crate::audit::log_action(&pool, did, "auth.verify", "session", None, None, None, None).await;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "userId": row.0,
            "privyDid": row.0,
            "email": row.1,
            "name": row.2,
            "walletAddress": wallet,
            "linkedAccounts": linked,
            "createdAt": row.3,
        }
    })))
}

/// GET /auth/me — Get current user profile
pub async fn me(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (String, Option<String>, Option<String>, serde_json::Value, chrono::DateTime<Utc>)>(
        "SELECT privy_did, email, name, COALESCE(linked_accounts, '[]'::jsonb), created_at FROM users WHERE privy_did = $1",
    )
    .bind(&user.privy_did)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "userId": row.0,
            "privyDid": row.0,
            "email": row.1,
            "name": row.2,
            "linkedAccounts": row.3,
            "createdAt": row.4,
        }
    })))
}

/// POST /auth/link-wallet — Link a crypto wallet to user
pub async fn link_wallet(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(req): Json<LinkWalletRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let chain = req.chain_type.unwrap_or_else(|| "ethereum".into());

    sqlx::query(
        r#"UPDATE users SET
           linked_accounts = linked_accounts || $1::jsonb,
           updated_at = NOW()
           WHERE privy_did = $2"#,
    )
    .bind(serde_json::json!([{
        "type": "wallet",
        "address": req.wallet_address,
        "chainType": chain,
    }]))
    .bind(&user.privy_did)
    .execute(&pool)
    .await?;

    crate::audit::log_action(&pool, &user.privy_did, "auth.link_wallet", "wallet", Some(&req.wallet_address), None, None, None).await;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "message": "Wallet linked", "address": req.wallet_address }
    })))
}

/// POST /auth/logout — Invalidate session (client-side mainly)
pub async fn logout(
    _user: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    // Privy sessions are managed on the client side
    // Server can clear any local session state if needed
    // Note: no pool available here, so audit is skipped for logout
    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "message": "Logged out" }
    })))
}
