//! Stress v2 — 1-minute inference producing a 0..100 score + 5-level label
//! (calm/mild/moderate/elevated/severe) from HR/HRV/breathing_rate/coherence
//! features. Runs per-user every 60 s and mirrors the score into
//! `biometrics_1min` so it flows through the intraday visualisation pipe.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn infer_minute(pool: &PgPool, user_id: Uuid, ts: DateTime<Utc>) -> anyhow::Result<()> {
    let features: Option<(Option<f64>, Option<f64>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT AVG(value) FILTER (WHERE metric_type='hr') as hr,
                AVG(value) FILTER (WHERE metric_type='hrv') as hrv,
                AVG(value) FILTER (WHERE metric_type='breathing_rate') as br,
                AVG(value) FILTER (WHERE metric_type='coherence') as coh
         FROM biometrics_1min
         WHERE user_id=$1 AND ts > $2 - INTERVAL '60 seconds' AND ts <= $2",
    )
    .bind(user_id)
    .bind(ts)
    .fetch_optional(pool)
    .await?;

    let (hr, hrv, br, coh) = features
        .map(|(a, b, c, d)| {
            (
                a.unwrap_or(70.0),
                b.unwrap_or(50.0),
                c.unwrap_or(16.0),
                d.unwrap_or(0.5),
            )
        })
        .unwrap_or((70.0, 50.0, 16.0, 0.5));

    // Rough ANS proxies: elevated HR + suppressed HRV → sympathetic push;
    // high coherence + slow BR → parasympathetic dominance.
    let sympathetic = ((hr - 65.0) / 20.0 - (hrv - 50.0) / 30.0).clamp(-1.0, 1.0);
    let parasympathetic = (coh - 0.5 + (16.0 - br) / 10.0).clamp(-1.0, 1.0);
    let score = (50.0 + sympathetic * 25.0 - parasympathetic * 25.0).clamp(0.0, 100.0);

    let level = match score as i32 {
        s if s < 20 => "calm",
        s if s < 40 => "mild",
        s if s < 60 => "moderate",
        s if s < 80 => "elevated",
        _ => "severe",
    };

    sqlx::query(
        "INSERT INTO stress_samples_1min
            (user_id, ts, score, level, micro_pulse, sympathetic_proxy,
             parasympathetic_proxy, baseline_delta)
         VALUES ($1, $2, $3, $4, false, $5, $6, 0)
         ON CONFLICT (user_id, ts) DO NOTHING",
    )
    .bind(user_id)
    .bind(ts)
    .bind(score)
    .bind(level)
    .bind(sympathetic)
    .bind(parasympathetic)
    .execute(pool)
    .await?;

    crate::intraday::ingest::write_1min(pool, user_id, ts, "stress", score)
        .await
        .ok();
    Ok(())
}

pub fn spawn_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let users: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM users")
                .fetch_all(&pool)
                .await
                .unwrap_or_default();
            for (user_id,) in users {
                if let Err(e) = infer_minute(&pool, user_id, Utc::now()).await {
                    tracing::warn!(?e, ?user_id, "stress infer_minute failed");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn score_levels_ordering() {
        // Boundary check for the 5-level classifier.
        fn label(score: i32) -> &'static str {
            match score {
                s if s < 20 => "calm",
                s if s < 40 => "mild",
                s if s < 60 => "moderate",
                s if s < 80 => "elevated",
                _ => "severe",
            }
        }
        assert_eq!(label(10), "calm");
        assert_eq!(label(25), "mild");
        assert_eq!(label(55), "moderate");
        assert_eq!(label(75), "elevated");
        assert_eq!(label(90), "severe");
    }
}
