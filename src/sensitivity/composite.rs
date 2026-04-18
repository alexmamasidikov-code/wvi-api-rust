//! Composite engine — matches pre-seeded correlation pairs against active
//! signals in the last 30-minute window, fires composite_signals rows with a
//! rarity percentile (180-day lookback).

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub struct CompositeSignal {
    pub id: Uuid,
    pub pair_id: String,
    pub anomaly_percentile: f64,
    pub severity: String,
}

pub async fn evaluate(pool: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<CompositeSignal>> {
    let window_start = Utc::now() - Duration::minutes(30);

    let signals: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT id, metric_type, direction, severity FROM signals
         WHERE user_id=$1 AND ts > $2 AND NOT ack",
    )
    .bind(user_id)
    .bind(window_start)
    .fetch_all(pool)
    .await?;

    let pairs: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT pair_id, metric_a, direction_a, metric_b, direction_b, severity_boost
         FROM correlation_pairs_config",
    )
    .fetch_all(pool)
    .await?;

    let mut created = vec![];
    for (pair_id, m_a, d_a, m_b, d_b, sev_boost) in pairs {
        let sig_a = signals.iter().find(|(_, m, d, _)| m == &m_a && d == &d_a);
        let sig_b = signals.iter().find(|(_, m, d, _)| m == &m_b && d == &d_b);
        let (Some(a), Some(b)) = (sig_a, sig_b) else { continue };

        // Dedup: skip if we already fired this pair inside the window.
        let exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM composite_signals
             WHERE user_id=$1 AND pair_id=$2 AND ts > $3",
        )
        .bind(user_id)
        .bind(&pair_id)
        .bind(window_start)
        .fetch_optional(pool)
        .await?;
        if exists.is_some() {
            continue;
        }

        let rarity = compute_rarity(pool, user_id, &pair_id).await?;
        let id = Uuid::new_v4();

        let components: Vec<Uuid> = vec![a.0, b.0];
        sqlx::query(
            "INSERT INTO composite_signals
                (id, user_id, ts, pair_id, component_signal_ids, anomaly_percentile, severity)
             VALUES ($1, $2, NOW(), $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(user_id)
        .bind(&pair_id)
        .bind(&components)
        .bind(rarity)
        .bind(&sev_boost)
        .execute(pool)
        .await?;

        created.push(CompositeSignal {
            id,
            pair_id,
            anomaly_percentile: rarity,
            severity: sev_boost,
        });
    }
    Ok(created)
}

/// Rarity percentile based on 180-day firing frequency — fewer fires ⇒ rarer.
pub async fn compute_rarity(pool: &PgPool, user_id: Uuid, pair_id: &str) -> sqlx::Result<f64> {
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM composite_signals
         WHERE user_id=$1 AND pair_id=$2 AND ts > NOW() - INTERVAL '180 days'",
    )
    .bind(user_id)
    .bind(pair_id)
    .fetch_one(pool)
    .await?;
    let percentile = if count.0 == 0 {
        0.99
    } else if count.0 < 5 {
        0.85
    } else if count.0 < 20 {
        0.5
    } else {
        0.2
    };
    Ok(percentile)
}
