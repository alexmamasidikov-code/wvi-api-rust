use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn write_1min(
    pool: &PgPool,
    user_id: Uuid,
    ts: DateTime<Utc>,
    metric: &str,
    value: f64,
) -> sqlx::Result<()> {
    if !value.is_finite() {
        tracing::warn!(?user_id, metric, value, "skip non-finite write_1min");
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO biometrics_1min (user_id, ts, metric_type, value)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, ts, metric_type) DO NOTHING",
    )
    .bind(user_id)
    .bind(ts)
    .bind(metric)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn write_event(
    pool: &PgPool,
    user_id: Uuid,
    ts: DateTime<Utc>,
    event_type: &str,
    meta: Value,
) -> sqlx::Result<()> {
    let now = Utc::now();
    if ts > now + chrono::Duration::seconds(5) {
        tracing::warn!(?user_id, event_type, ?ts, "reject future event");
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO intraday_events (user_id, ts, event_type, meta)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(ts)
    .bind(event_type)
    .bind(meta)
    .execute(pool)
    .await?;
    Ok(())
}

pub fn spawn_write_1min(pool: PgPool, user_id: Uuid, ts: DateTime<Utc>, metric: String, value: f64) {
    tokio::spawn(async move {
        if let Err(e) = write_1min(&pool, user_id, ts, &metric, value).await {
            tracing::error!(?e, ?user_id, metric, "write_1min failed");
        }
    });
}

pub fn spawn_write_event(
    pool: PgPool,
    user_id: Uuid,
    ts: DateTime<Utc>,
    event_type: String,
    meta: Value,
) {
    tokio::spawn(async move {
        if let Err(e) = write_event(&pool, user_id, ts, &event_type, meta).await {
            tracing::error!(?e, ?user_id, event_type, "write_event failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> PgPool {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wvi:wvi@localhost:5432/wvi".into());
        PgPool::connect(&url).await.expect("test DB not reachable")
    }

    #[tokio::test]
    async fn test_write_1min_basic() {
        let pool = test_pool().await;
        let user = Uuid::new_v4();
        let ts = Utc::now();
        write_1min(&pool, user, ts, "hr", 65.0).await.unwrap();
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM biometrics_1min WHERE user_id=$1")
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count.0, 1);
        sqlx::query("DELETE FROM biometrics_1min WHERE user_id=$1")
            .bind(user)
            .execute(&pool)
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_write_1min_idempotent() {
        let pool = test_pool().await;
        let user = Uuid::new_v4();
        let ts = Utc::now();
        write_1min(&pool, user, ts, "hr", 65.0).await.unwrap();
        write_1min(&pool, user, ts, "hr", 70.0).await.unwrap();
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM biometrics_1min WHERE user_id=$1")
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count.0, 1);
        sqlx::query("DELETE FROM biometrics_1min WHERE user_id=$1")
            .bind(user)
            .execute(&pool)
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_write_event_basic() {
        let pool = test_pool().await;
        let user = Uuid::new_v4();
        let meta = serde_json::json!({"kind":"running","duration_sec":1800});
        write_event(&pool, user, Utc::now(), "workout", meta)
            .await
            .unwrap();
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM intraday_events WHERE user_id=$1")
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count.0, 1);
        sqlx::query("DELETE FROM intraday_events WHERE user_id=$1")
            .bind(user)
            .execute(&pool)
            .await
            .ok();
    }
}
