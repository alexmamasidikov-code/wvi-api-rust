use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use axum::{extract::{Path, State}, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Alarm {
    pub alarm_id_client: Uuid,
    pub time_hhmm: String,
    pub weekday_mask: i16,
    pub enabled: bool,
    pub label: String,
    pub smart_wake: bool,
    pub last_modified: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct SyncBody {
    pub alarms: Vec<Alarm>,
}

pub async fn list_alarms(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<Vec<Alarm>>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let rows: Vec<Alarm> = sqlx::query_as(
        "SELECT alarm_id_client, time_hhmm, weekday_mask, enabled, label, smart_wake, last_modified
         FROM user_alarms WHERE user_id = $1 ORDER BY time_hhmm ASC"
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;
    Ok(Json(rows))
}

pub async fn sync_alarms(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<SyncBody>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    if body.alarms.len() > 10 {
        return Err(AppError::BadRequest("max 10 alarms".into()));
    }
    for a in &body.alarms {
        sqlx::query(
            "INSERT INTO user_alarms (user_id, alarm_id_client, time_hhmm, weekday_mask,
                                      enabled, label, smart_wake, last_modified)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (user_id, alarm_id_client) DO UPDATE SET
               time_hhmm     = EXCLUDED.time_hhmm,
               weekday_mask  = EXCLUDED.weekday_mask,
               enabled       = EXCLUDED.enabled,
               label         = EXCLUDED.label,
               smart_wake    = EXCLUDED.smart_wake,
               last_modified = CASE WHEN EXCLUDED.last_modified > user_alarms.last_modified
                                    THEN EXCLUDED.last_modified ELSE user_alarms.last_modified END"
        )
        .bind(user_id)
        .bind(a.alarm_id_client)
        .bind(&a.time_hhmm)
        .bind(a.weekday_mask)
        .bind(a.enabled)
        .bind(&a.label)
        .bind(a.smart_wake)
        .bind(a.last_modified)
        .execute(&pool)
        .await
        .map_err(AppError::from)?;
    }
    Ok(Json(serde_json::json!({ "synced": body.alarms.len() })))
}

pub async fn delete_alarm(
    user: AuthUser,
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    sqlx::query("DELETE FROM user_alarms WHERE user_id = $1 AND alarm_id_client = $2")
        .bind(user_id)
        .bind(id)
        .execute(&pool)
        .await
        .map_err(AppError::from)?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}
