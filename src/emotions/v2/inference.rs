//! Emotion v2 per-minute inference.
//!
//! Engine philosophy — always-explicit triplet:
//! every sample carries the three most probable labels from the 18-label
//! palette. There is no "between two emotions" fallback in the UI; instead
//! we surface a primary label (≥0.40 typical intensity), a secondary (0.20)
//! and a tertiary (0.10) so the user always sees an anchored word.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// 18-label emotion palette anchored in (valence, arousal) space.
pub const EMOTIONS_18: [(&str, f64, f64); 18] = [
    ("excited", 0.6, 0.8),
    ("happy", 0.8, 0.5),
    ("content", 0.7, 0.0),
    ("calm", 0.5, -0.5),
    ("relaxed", 0.6, -0.7),
    ("focused", 0.2, 0.3),
    ("energetic", 0.5, 0.9),
    ("tired", -0.3, -0.7),
    ("bored", -0.5, -0.5),
    ("anxious", -0.4, 0.6),
    ("stressed", -0.6, 0.7),
    ("angry", -0.7, 0.7),
    ("sad", -0.8, -0.3),
    ("depressed", -0.9, -0.6),
    ("surprised", 0.2, 0.9),
    ("confused", -0.2, 0.4),
    ("neutral", 0.0, 0.0),
    ("spacious", 0.4, -0.2),
];

pub async fn infer_minute(pool: &PgPool, user_id: Uuid, ts: DateTime<Utc>) -> anyhow::Result<()> {
    // Pull the last 60 s of relevant biometrics; fall back to typical-adult
    // defaults when the user has no recent samples (keeps the worker running
    // without dumping the sample row when the device is offline).
    let features: Option<(Option<f64>, Option<f64>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT AVG(value) FILTER (WHERE metric_type='hr') as hr,
                AVG(value) FILTER (WHERE metric_type='hrv') as hrv,
                AVG(value) FILTER (WHERE metric_type='coherence') as coh,
                AVG(value) FILTER (WHERE metric_type='breathing_rate') as br
         FROM biometrics_1min
         WHERE user_id=$1 AND ts > $2 - INTERVAL '60 seconds' AND ts <= $2",
    )
    .bind(user_id)
    .bind(ts)
    .fetch_optional(pool)
    .await?;

    let (hr, hrv, coh, br) = features
        .map(|(a, b, c, d)| {
            (
                a.unwrap_or(70.0),
                b.unwrap_or(50.0),
                c.unwrap_or(0.5),
                d.unwrap_or(16.0),
            )
        })
        .unwrap_or((70.0, 50.0, 0.5, 16.0));

    // Simplified fuzzy mapper: valence ~ coherence positivity + HR tension,
    // arousal ~ HR + breathing rate. Full clinical version will add HRV
    // trajectory and coherence bands per spec §4.3.
    let valence = ((coh - 0.5) * 2.0 - (hr - 70.0) / 50.0).clamp(-1.0, 1.0);
    let arousal = ((hr - 70.0) / 30.0 + (br - 16.0) / 10.0).clamp(-1.0, 1.0);
    let confidence = ((hrv - 30.0) / 40.0).clamp(0.3, 1.0);

    // Rank all 18 anchors by Euclidean distance; top 3 become the triplet.
    let mut distances: Vec<(&str, f64)> = EMOTIONS_18
        .iter()
        .map(|(name, v, a)| {
            let d = ((valence - v).powi(2) + (arousal - a).powi(2)).sqrt();
            (*name, d)
        })
        .collect();
    distances.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let (p, s, t) = (&distances[0], &distances[1], &distances[2]);
    let inv_p = 1.0 / (p.1 + 0.01);
    let inv_s = 1.0 / (s.1 + 0.01);
    let inv_t = 1.0 / (t.1 + 0.01);
    let sum = inv_p + inv_s + inv_t;
    let ip = inv_p / sum;
    let is = inv_s / sum;
    let it = inv_t / sum;

    let ctx = crate::sensitivity::types::ContextKey::from_ts(
        ts,
        crate::sensitivity::types::ActivityState::Resting,
    );

    sqlx::query(
        "INSERT INTO emotion_samples_1min (
            user_id, ts, valence, arousal, confidence,
            primary_emotion, primary_intensity,
            secondary_emotion, secondary_intensity,
            tertiary_emotion, tertiary_intensity,
            context_key
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (user_id, ts) DO NOTHING",
    )
    .bind(user_id)
    .bind(ts)
    .bind(valence)
    .bind(arousal)
    .bind(confidence)
    .bind(p.0)
    .bind(ip)
    .bind(s.0)
    .bind(is)
    .bind(t.0)
    .bind(it)
    .bind(ctx.as_str())
    .execute(pool)
    .await?;

    // Mirror into intraday stream so IntradayChart reuses one renderer.
    crate::intraday::ingest::write_1min(pool, user_id, ts, "valence", valence)
        .await
        .ok();
    crate::intraday::ingest::write_1min(pool, user_id, ts, "arousal", arousal)
        .await
        .ok();
    crate::intraday::ingest::write_1min(pool, user_id, ts, "emotion_confidence", confidence)
        .await
        .ok();

    // Detect emotion shift → write event so triggers engine has something to
    // correlate. A "shift" is either a primary-label change or a ≥0.15
    // intensity swing.
    let prev: Option<(String, f64)> = sqlx::query_as(
        "SELECT primary_emotion, primary_intensity FROM emotion_samples_1min
         WHERE user_id=$1 AND ts < $2
         ORDER BY ts DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(ts)
    .fetch_optional(pool)
    .await?;

    if let Some((prev_p, prev_i)) = prev {
        if prev_p != p.0 || (prev_i - ip).abs() > 0.15 {
            let meta = serde_json::json!({
                "from": prev_p,
                "to": p.0,
                "magnitude": (prev_i - ip).abs()
            });
            crate::intraday::ingest::write_event(pool, user_id, ts, "emotion_shift", meta)
                .await
                .ok();
        }
    }

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
                    tracing::warn!(?e, ?user_id, "emotion infer_minute failed");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_has_18_labels() {
        assert_eq!(EMOTIONS_18.len(), 18);
    }

    #[test]
    fn palette_values_in_unit_square() {
        for (_, v, a) in EMOTIONS_18 {
            assert!((-1.0..=1.0).contains(&v));
            assert!((-1.0..=1.0).contains(&a));
        }
    }
}
