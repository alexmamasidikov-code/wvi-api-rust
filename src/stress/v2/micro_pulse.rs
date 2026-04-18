//! Stress v2 — 5-second micro-pulse detector.
//!
//! Flags acute autonomic spikes (HR ↑>15%, HRV ↓>20% within 5 s) that the
//! 1-min inference path averages away. When triggered it flips the
//! `micro_pulse` flag on the current minute's stress row and writes a
//! `stress_pulse` intraday event so the iOS strip can draw a tick mark.
//!
//! Performance note — polling every 5 s for all users is heavy at scale;
//! the plan acknowledges this is out-of-scope for the MVP. A future
//! refactor should gate on "has-recent-biometric" and/or push-based wake.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn check(pool: &PgPool, user_id: Uuid) -> sqlx::Result<()> {
    let row: Option<(Option<f64>, Option<f64>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT
            (SELECT AVG(value) FROM biometrics_1min WHERE user_id=$1 AND metric_type='hr'
             AND ts > NOW() - INTERVAL '5 seconds') AS hr5,
            (SELECT AVG(value) FROM biometrics_1min WHERE user_id=$1 AND metric_type='hrv'
             AND ts > NOW() - INTERVAL '5 seconds') AS hrv5,
            (SELECT AVG(value) FROM biometrics_1min WHERE user_id=$1 AND metric_type='hr'
             AND ts BETWEEN NOW() - INTERVAL '70 seconds' AND NOW() - INTERVAL '60 seconds') AS hr_prev,
            (SELECT AVG(value) FROM biometrics_1min WHERE user_id=$1 AND metric_type='hrv'
             AND ts BETWEEN NOW() - INTERVAL '70 seconds' AND NOW() - INTERVAL '60 seconds') AS hrv_prev",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let Some((Some(hr5), Some(hrv5), Some(hr_prev), Some(hrv_prev))) = row else {
        return Ok(());
    };
    if hr_prev <= 0.0 || hrv_prev <= 0.0 {
        return Ok(());
    }
    let hr_jump = (hr5 - hr_prev) / hr_prev;
    let hrv_drop = (hrv_prev - hrv5) / hrv_prev;
    if hr_jump > 0.15 && hrv_drop > 0.20 {
        sqlx::query(
            "UPDATE stress_samples_1min SET micro_pulse=true
             WHERE user_id=$1 AND ts=date_trunc('minute', NOW())",
        )
        .bind(user_id)
        .execute(pool)
        .await?;
        let meta = serde_json::json!({"hr_jump": hr_jump, "hrv_drop": hrv_drop});
        let _ = crate::intraday::ingest::write_event(pool, user_id, Utc::now(), "stress_pulse", meta).await;
    }
    Ok(())
}

pub fn spawn_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            let users: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM users")
                .fetch_all(&pool)
                .await
                .unwrap_or_default();
            for (user_id,) in users {
                let _ = check(&pool, user_id).await;
            }
        }
    });
}
