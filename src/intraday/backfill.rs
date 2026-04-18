use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub struct BackfillJob {
    pub id: Uuid,
    pub metric_type: String,
    pub new_version: i32,
}

pub async fn start_backfill(
    pool: &PgPool,
    metric: String,
    new_version: i32,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> sqlx::Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO backfill_jobs (id, metric_type, new_version, started_at, status)
         VALUES ($1, $2, $3, NOW(), 'running')",
    )
    .bind(id)
    .bind(&metric)
    .bind(new_version)
    .execute(pool)
    .await?;

    let pool = pool.clone();
    tokio::spawn(async move {
        if let Err(e) = run_backfill(&pool, id, metric, new_version, range_start, range_end).await {
            tracing::error!(?e, ?id, "backfill failed");
            let _ = sqlx::query(
                "UPDATE backfill_jobs SET status='failed', completed_at=NOW() WHERE id=$1",
            )
            .bind(id)
            .execute(&pool)
            .await;
        }
    });
    Ok(id)
}

async fn run_backfill(
    pool: &PgPool,
    id: Uuid,
    metric: String,
    new_version: i32,
    _range_start: DateTime<Utc>,
    _range_end: DateTime<Utc>,
) -> sqlx::Result<()> {
    // Simplified MVP: iterate users in batches, mark formula_version upgrade.
    // Full formula-recompute deferred to Projects B and C when they deploy new formulas.
    let users: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM users ORDER BY id ASC")
        .fetch_all(pool)
        .await?;
    let total = users.len() as f64;
    for (i, (user_id,)) in users.iter().enumerate() {
        sqlx::query(
            "UPDATE biometrics_5min SET formula_version=$1
             WHERE user_id=$2 AND metric_type=$3 AND formula_version < $1",
        )
        .bind(new_version)
        .bind(user_id)
        .bind(&metric)
        .execute(pool)
        .await?;

        if i % 10 == 0 && total > 0.0 {
            let progress = (i + 1) as f64 / total;
            sqlx::query(
                "UPDATE backfill_jobs SET progress_ratio=$1, last_user_id=$2 WHERE id=$3",
            )
            .bind(progress)
            .bind(user_id)
            .bind(id)
            .execute(pool)
            .await?;
        }
    }
    sqlx::query(
        "UPDATE backfill_jobs SET status='completed', completed_at=NOW(), progress_ratio=1.0 WHERE id=$1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
