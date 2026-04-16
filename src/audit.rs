use axum::{extract::State, Json};
use sqlx::PgPool;
use serde_json::Value as JsonValue;

use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

/// Insert an audit log entry. Errors are silently ignored so audit
/// never breaks the primary request flow.
pub async fn log_action(
    pool: &PgPool,
    user_id: &str,
    action: &str,
    resource_type: &str,
    resource_id: Option<&str>,
    details: Option<JsonValue>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) {
    let _ = sqlx::query(
        "INSERT INTO audit_log (user_id, action, resource_type, resource_id, details, ip_address, user_agent) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)"
    )
    .bind(user_id)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(details)
    .bind(ip_address)
    .bind(user_agent)
    .execute(pool)
    .await;
}

/// GET /api/v1/audit/log — Return recent audit entries for the authenticated user
pub async fn get_audit_log(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<JsonValue>> {
    let rows = sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<JsonValue>, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, user_id, action, resource_type, resource_id, details, ip_address, user_agent, created_at \
         FROM audit_log WHERE user_id = $1 ORDER BY created_at DESC LIMIT 100"
    )
    .bind(&user.privy_did)
    .fetch_all(&pool)
    .await?;

    let data: Vec<JsonValue> = rows.iter().map(|r| serde_json::json!({
        "id": r.0,
        "userId": r.1,
        "action": r.2,
        "resourceType": r.3,
        "resourceId": r.4,
        "details": r.5,
        "ipAddress": r.6,
        "userAgent": r.7,
        "createdAt": r.8,
    })).collect();

    Ok(Json(serde_json::json!({
        "success": true,
        "data": data
    })))
}
